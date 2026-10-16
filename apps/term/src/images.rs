//! Pictures in the chat: finding out what the terminal can show, loading and encoding the images
//! on background threads, and drawing them over the lines the renderer reserved for them.
//!
//! An image source (`src` of an image component) goes through two steps, each on a thread of
//! its own so the screen never waits: a *probe* resolves it to a file (an `asset://` handle asks
//! the daemon) and reads the size from its header, which lets the renderer reserve the right
//! number of rows; an *encode* decodes it, scales it to the cells it is shown in and turns that
//! into what the terminal protocol wants (kitty, sixel, iTerm2 or half blocks). Only images that
//! are in view are encoded. Results are kept by (source, cells) and the oldest are let go.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use image::DynamicImage;
use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui::style::Style;
use ratatui::widgets::Widget;
use ratatui_image::picker::cap_parser::QueryStdioOptions;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::sliced::{SignedPosition, SlicedImage, SlicedProtocol};
use ratatui_image::{FilterType, Resize};
use suite_renderer_ratatui::ImageStatus;

use crate::chat::{ChatImages, Placement};

/// `kitty`, `sixel`, `iterm2`, `halfblocks` force a protocol, `off` keeps the text placeholder;
/// anything else (or unset) asks the terminal.
pub const ENV: &str = "SUITE_TERM_IMAGES";

/// How long to wait for the terminal to answer the detection query.
const QUERY_TIMEOUT: Duration = Duration::from_millis(1000);
/// Encoded images kept (each is a screenful of escape codes or cells at most).
const KEEP_ENCODED: usize = 32;
/// Decoded images the encoder keeps for the next size or scroll.
const KEEP_DECODED: usize = 4;
/// Larger images are scaled down once, after decoding: no thumbnail needs more.
const MAX_PIXELS: u32 = 2000;

/// What `SUITE_TERM_IMAGES` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Off,
    Auto,
    Force(ProtocolType),
}

pub fn parse_mode(value: Option<&str>) -> Mode {
    match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("off" | "0" | "none" | "no" | "false") => Mode::Off,
        Some("kitty") => Mode::Force(ProtocolType::Kitty),
        Some("sixel") => Mode::Force(ProtocolType::Sixel),
        Some("iterm2" | "iterm") => Mode::Force(ProtocolType::Iterm2),
        Some("halfblocks" | "half" | "blocks") => Mode::Force(ProtocolType::Halfblocks),
        _ => Mode::Auto,
    }
}

/// Asks the terminal what it can draw and how big a cell is (kitty, sixel and iTerm2 graphics
/// through tmux work with `allow-passthrough on`; without, the terminal does not answer and
/// half blocks it is). None when images are switched off. It talks to the terminal on stdin and
/// stdout, so it runs after raw mode is on and before any key is read; tests build a
/// [`Picker::halfblocks`] instead.
pub fn detect(mode: Mode) -> Option<Picker> {
    if mode == Mode::Off {
        return None;
    }
    let options = QueryStdioOptions {
        timeout: QUERY_TIMEOUT,
        ..QueryStdioOptions::default()
    };
    let mut picker =
        Picker::from_query_stdio_with_options(options).unwrap_or_else(|_| Picker::halfblocks());
    if let Mode::Force(protocol) = mode {
        picker.set_protocol_type(protocol);
    }
    Some(picker)
}

/// Turns an `asset://` handle into the stored file's path (a call to the daemon).
pub type Assets = Arc<dyn Fn(&str) -> Result<PathBuf, String> + Send + Sync>;

/// Where the image behind `src` lives on disk: a path (relative ones to `cwd`), a `file://`
/// URL, or an asset handle. Remote and inline images are not loaded.
pub fn resolve(src: &str, cwd: &Path, assets: Option<&Assets>) -> Result<PathBuf, String> {
    if src.starts_with("asset://") {
        return match assets {
            Some(assets) => assets(src),
            None => Err("no connection to look the image up".into()),
        };
    }
    if src.starts_with("data:") {
        return Err("inline images are not shown".into());
    }
    if src.starts_with("http://") || src.starts_with("https://") {
        return Err("remote images are not fetched".into());
    }
    let path = Path::new(src.strip_prefix("file://").unwrap_or(src));
    Ok(cwd.join(path))
}

/// The pixel size from the file's header.
fn dimensions(path: &Path) -> Result<(u32, u32), String> {
    let reader = image::ImageReader::open(path)
        .map_err(|e| failed_to_open(&e))?
        .with_guessed_format()
        .map_err(|e| failed_to_open(&e))?;
    reader
        .into_dimensions()
        .map_err(|_| "not an image I can show (png, jpeg, gif, webp)".to_string())
}

fn failed_to_open(err: &std::io::Error) -> String {
    match err.kind() {
        std::io::ErrorKind::NotFound => "file not found".into(),
        _ => err.to_string(),
    }
}

fn decode(path: &Path) -> Result<DynamicImage, String> {
    let img = image::ImageReader::open(path)
        .map_err(|e| failed_to_open(&e))?
        .with_guessed_format()
        .map_err(|e| failed_to_open(&e))?
        .decode()
        .map_err(|e| format!("can't decode: {e}"))?;
    Ok(if img.width().max(img.height()) > MAX_PIXELS {
        img.resize(MAX_PIXELS, MAX_PIXELS, FilterType::Triangle)
    } else {
        img
    })
}

/// (source, columns, rows): one encoding.
type Key = (String, u16, u16);

enum Proto {
    /// Asked for; the encoder is on it.
    Pending,
    Ready(Box<SlicedProtocol>),
}

struct EncodeJob {
    key: Key,
    path: PathBuf,
}

enum Done {
    Probed(String, Result<(PathBuf, u32, u32), String>),
    Encoded(Key, Result<Box<SlicedProtocol>, String>),
}

/// Image state of the app: what is known of each source, the encoded images, the threads.
pub struct Images {
    picker: Picker,
    status: Arc<HashMap<String, ImageStatus>>,
    paths: HashMap<String, PathBuf>,
    protos: HashMap<Key, Proto>,
    /// Keys of `protos`, least recently drawn first.
    recent: VecDeque<Key>,
    probe_tx: mpsc::Sender<String>,
    encode_tx: mpsc::Sender<EncodeJob>,
    done: mpsc::Receiver<Done>,
}

impl Images {
    /// Starts the two worker threads. They end when this is dropped.
    pub fn new(picker: Picker, cwd: PathBuf, assets: Option<Assets>) -> Images {
        let (probe_tx, probe_rx) = mpsc::channel::<String>();
        let (encode_tx, encode_rx) = mpsc::channel::<EncodeJob>();
        let (done_tx, done) = mpsc::channel();
        let tx = done_tx.clone();
        thread::Builder::new()
            .name("suite-term-image-probe".into())
            .spawn(move || {
                while let Ok(src) = probe_rx.recv() {
                    let result = resolve(&src, &cwd, assets.as_ref())
                        .and_then(|path| dimensions(&path).map(|(w, h)| (path, w, h)));
                    if tx.send(Done::Probed(src, result)).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn image probe thread");
        let encoder = picker.clone();
        thread::Builder::new()
            .name("suite-term-image-encode".into())
            .spawn(move || encode_loop(&encoder, &encode_rx, &done_tx))
            .expect("spawn image encode thread");
        Images {
            picker,
            status: Arc::new(HashMap::new()),
            paths: HashMap::new(),
            protos: HashMap::new(),
            recent: VecDeque::new(),
            probe_tx,
            encode_tx,
            done,
        }
    }

    /// What the chat needs to reserve room for images.
    pub fn chat_view(&self) -> ChatImages {
        let font = self.picker.font_size();
        ChatImages {
            cell: (font.width, font.height),
            status: self.status.clone(),
        }
    }

    pub fn protocol(&self) -> ProtocolType {
        self.picker.protocol_type()
    }

    /// Starts looking at the sources not seen before.
    pub fn request<'a>(&mut self, sources: impl IntoIterator<Item = &'a str>) {
        for src in sources {
            if self.status.contains_key(src) {
                continue;
            }
            Arc::make_mut(&mut self.status).insert(src.to_string(), ImageStatus::Loading);
            let _ = self.probe_tx.send(src.to_string());
        }
    }

    /// Takes in what the threads finished; whether anything changed (so the screen needs
    /// painting).
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(done) = self.done.try_recv() {
            changed = true;
            match done {
                Done::Probed(src, Ok((path, width, height))) => {
                    self.paths.insert(src.clone(), path);
                    Arc::make_mut(&mut self.status)
                        .insert(src, ImageStatus::Ready { width, height });
                }
                Done::Probed(src, Err(why)) => {
                    Arc::make_mut(&mut self.status).insert(src, ImageStatus::Failed(why));
                }
                Done::Encoded(key, Ok(proto)) => {
                    // Dropped meanwhile (let go as old): nobody waits for it.
                    if let Some(slot) = self.protos.get_mut(&key) {
                        *slot = Proto::Ready(proto);
                    }
                }
                Done::Encoded(key, Err(why)) => {
                    self.protos.remove(&key);
                    self.recent.retain(|k| *k != key);
                    Arc::make_mut(&mut self.status).insert(key.0, ImageStatus::Failed(why));
                }
            }
        }
        changed
    }

    /// Draws one image of the chat into `buf`: `area` is the chat pane and the placement is
    /// relative to it, rows outside it are cut (scrolled away). Until the encoded image is
    /// there it says so on the first row.
    pub fn draw(&mut self, buf: &mut Buffer, area: Rect, at: &Placement, wait: Style) {
        let key: Key = (at.src.clone(), at.cols, at.rows);
        match self.protos.get(&key) {
            Some(Proto::Ready(proto)) => {
                let x = i16::try_from(at.x).unwrap_or(i16::MAX);
                let y = i16::try_from(at.y).unwrap_or(i16::MAX);
                SlicedImage::new(proto, SignedPosition::from((x, y))).render(area, buf);
                self.touch(&key);
                return;
            }
            Some(Proto::Pending) => {}
            None => {
                let Some(path) = self.paths.get(&at.src).cloned() else {
                    return;
                };
                self.protos.insert(key.clone(), Proto::Pending);
                self.recent.push_back(key.clone());
                self.forget_old();
                let _ = self.encode_tx.send(EncodeJob { key, path });
            }
        }
        if at.y >= 0 && at.y < i32::from(area.height) {
            let x = area.x + at.x.min(area.width);
            let room = usize::from(area.width.saturating_sub(at.x));
            buf.set_stringn(x, area.y + at.y as u16, "loading image…", room, wait);
        }
    }

    fn touch(&mut self, key: &Key) {
        if self.recent.back() != Some(key) {
            self.recent.retain(|k| k != key);
            self.recent.push_back(key.clone());
        }
    }

    /// Lets the least recently drawn encodings go.
    fn forget_old(&mut self) {
        while self.recent.len() > KEEP_ENCODED {
            if let Some(old) = self.recent.pop_front() {
                self.protos.remove(&old);
            }
        }
    }

    #[cfg(test)]
    fn encoded(&self) -> usize {
        self.protos.len()
    }
}

/// Decodes and encodes what the app asks for, one at a time, keeping the last few decoded
/// images (scrolling past one image asks for several sizes of the same file).
fn encode_loop(picker: &Picker, jobs: &mpsc::Receiver<EncodeJob>, done: &mpsc::Sender<Done>) {
    let mut decoded: Vec<(PathBuf, Arc<DynamicImage>)> = Vec::new();
    while let Ok(job) = jobs.recv() {
        let image = match decoded.iter().position(|(p, _)| *p == job.path) {
            Some(at) => Ok(decoded[at].1.clone()),
            None => decode(&job.path).map(|img| {
                let img = Arc::new(img);
                decoded.push((job.path.clone(), img.clone()));
                if decoded.len() > KEEP_DECODED {
                    decoded.remove(0);
                }
                img
            }),
        };
        let (_, cols, rows) = &job.key;
        let result = image.and_then(|img| {
            SlicedProtocol::new_with_resize(
                picker,
                DynamicImage::clone(&img),
                Size::new(*cols, *rows),
                Resize::Fit(Some(FilterType::Triangle)),
            )
            .map(Box::new)
            .map_err(|e| e.to_string())
        });
        if done.send(Done::Encoded(job.key, result)).is_err() {
            break;
        }
    }
}

#[cfg(test)]
#[path = "images_tests.rs"]
mod tests;
