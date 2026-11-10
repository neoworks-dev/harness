//! Pictures in the chat: finding out what the terminal can show, loading and encoding the images
//! on background threads, and drawing them over the lines the renderer reserved for them.
//!
//! An image source (`src` of an image component) goes through two steps, each on a thread of
//! its own so the screen never waits: a *probe* resolves it to a file (an `asset://` handle or
//! an `http(s)://` URL asks the daemon, which downloads the image into its asset store) and reads the size from its header, which lets the renderer reserve the right
//! number of rows; an *encode* decodes it, scales it to the cells it is shown in and turns that
//! into what the terminal protocol wants (kitty, sixel, iTerm2 or half blocks). Only images that
//! are in view are encoded. Results are kept by (source, cells) and the oldest are let go.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use image::DynamicImage;
use neo_renderer_ratatui::{ChartSpec, ImageStatus, draw_chart};
use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui::style::Style;
use ratatui::widgets::Widget;
use ratatui_image::picker::cap_parser::QueryStdioOptions;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::sliced::{SignedPosition, SlicedImage, SlicedProtocol};
use ratatui_image::{FilterType, Resize};

use crate::chat::{ChatImages, Placement};

/// `kitty`, `sixel`, `iterm2`, `halfblocks` force a protocol, `off` keeps the text placeholder;
/// anything else (or unset) asks the terminal.
pub const ENV: &str = "NEO_TERM_IMAGES";

/// How long to wait for the terminal to answer the detection query.
const QUERY_TIMEOUT: Duration = Duration::from_millis(1000);
/// Encoded images kept (each is a screenful of escape codes or cells at most).
const KEEP_ENCODED: usize = 32;
/// Decoded images the encoder keeps for the next size or scroll.
const KEEP_DECODED: usize = 4;
/// Larger images are scaled down once, after decoding: no thumbnail needs more.
const MAX_PIXELS: u32 = 2000;

/// What `NEO_TERM_IMAGES` asks for.
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

/// Asks the terminal what it can draw and how big a cell is. None when images are switched off.
/// It talks to the terminal on stdin and stdout, so it runs after raw mode is on and before any
/// key is read; tests build a [`Picker::halfblocks`] instead.
///
/// Inside tmux the query goes through passthrough (ratatui-image turns `allow-passthrough` on for
/// the pane), but kitty's answer to the graphics query does not come back through tmux, so it
/// finds half blocks. tmux knows the terminal its client runs in, though: when that draws kitty
/// graphics, they are used (as unicode placeholders, which tmux passes on).
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
    if picker.protocol_type() == ProtocolType::Halfblocks
        && std::env::var_os("TMUX").is_some()
        && let Some(protocol) = tmux_client_terminal().as_deref().and_then(outer_protocol)
    {
        picker.set_protocol_type(protocol);
    }
    if let Mode::Force(protocol) = mode {
        picker.set_protocol_type(protocol);
    }
    Some(picker)
}

/// The terminal tmux's client runs in, as tmux says (`kitty(0.48.2)|xterm-kitty`).
fn tmux_client_terminal() -> Option<String> {
    let out = std::process::Command::new("tmux")
        .args(["display", "-p", "#{client_termtype}|#{client_termname}"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The graphics protocol of a terminal tmux runs in, by its name: kitty's for kitty and
/// Ghostty, which draw it with unicode placeholders.
fn outer_protocol(terminal: &str) -> Option<ProtocolType> {
    let terminal = terminal.to_ascii_lowercase();
    (terminal.contains("kitty") || terminal.contains("ghostty")).then_some(ProtocolType::Kitty)
}

/// Turns an `asset://` handle or an `http(s)://` URL into a file on disk: the stored file, or
/// the image the daemon downloaded into the store (a call to the daemon either way).
pub type Assets = Arc<dyn Fn(&str) -> Result<PathBuf, String> + Send + Sync>;

/// Where the image behind `src` lives on disk: a path (relative ones to `cwd`), a `file://`
/// URL, an asset handle or a web address. Inline images are not loaded.
pub fn resolve(src: &str, cwd: &Path, assets: Option<&Assets>) -> Result<PathBuf, String> {
    let remote = src.starts_with("http://") || src.starts_with("https://");
    if remote || src.starts_with("asset://") {
        return match assets {
            Some(assets) => assets(src),
            None => Err("no connection to look the image up".into()),
        };
    }
    if src.starts_with("data:") {
        return Err("inline images are not shown".into());
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

/// The chart drawn over `cols` x `rows` cells of the terminal's font, transparent around it.
fn chart(spec: &ChartSpec, picker: &Picker, cols: u16, rows: u16) -> Result<DynamicImage, String> {
    let font = picker.font_size();
    let (w, h) = (
        u32::from(cols) * u32::from(font.width),
        u32::from(rows) * u32::from(font.height),
    );
    let pixels = draw_chart(spec, w, h).ok_or("the chart has no room")?;
    image::RgbaImage::from_raw(w, h, pixels)
        .map(DynamicImage::ImageRgba8)
        .ok_or_else(|| "the chart drew the wrong size".to_string())
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
    source: Source,
}

/// What an encoding is of: a file, or a chart the renderer laid out (drawn here at the size
/// of its cells).
enum Source {
    File(PathBuf),
    Chart(Box<ChartSpec>),
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
            .name("neo-term-image-probe".into())
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
            .name("neo-term-image-encode".into())
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
            charts: false,
        }
    }

    /// Whether the terminal draws pictures well enough for charts: half blocks are too coarse
    /// and have no transparency.
    pub fn draws_charts(&self) -> bool {
        self.picker.protocol_type() != ProtocolType::Halfblocks
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
        self.draw_with(buf, area, at, Some(wait));
    }

    /// Like [`Images::draw`] but says nothing while the image is not there: for small pictures
    /// next to text, where a message would not fit.
    pub fn draw_quiet(&mut self, buf: &mut Buffer, area: Rect, at: &Placement) {
        self.draw_with(buf, area, at, None);
    }

    fn draw_with(&mut self, buf: &mut Buffer, area: Rect, at: &Placement, wait: Option<Style>) {
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
                let source = match ChartSpec::from_src(&at.src) {
                    Some(spec) => Source::Chart(Box::new(spec)),
                    None => match self.paths.get(&at.src) {
                        Some(path) => Source::File(path.clone()),
                        None => return,
                    },
                };
                self.protos.insert(key.clone(), Proto::Pending);
                self.recent.push_back(key.clone());
                self.forget_old();
                let _ = self.encode_tx.send(EncodeJob { key, source });
            }
        }
        if let Some(wait) = wait
            && at.y >= 0
            && at.y < i32::from(area.height)
        {
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
        let (_, cols, rows) = &job.key;
        let image = match job.source {
            Source::Chart(spec) => chart(&spec, picker, *cols, *rows).map(Arc::new),
            Source::File(path) => match decoded.iter().position(|(p, _)| *p == path) {
                Some(at) => Ok(decoded[at].1.clone()),
                None => decode(&path).map(|img| {
                    let img = Arc::new(img);
                    decoded.push((path.clone(), img.clone()));
                    if decoded.len() > KEEP_DECODED {
                        decoded.remove(0);
                    }
                    img
                }),
            },
        };
        let result = image.and_then(|img| {
            SlicedProtocol::new_with_resize(
                picker,
                DynamicImage::clone(&img),
                Size::new(*cols, *rows),
                // Grown too: a small picture still takes the room the chat gave it.
                Resize::Scale(Some(FilterType::Triangle)),
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
