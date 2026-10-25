use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use neo_protocol::{ViewItem, ViewSnapshot};
use neo_renderer_ratatui::Palette;
use neo_sdk::Event;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Style};
use serde_json::json;

use super::*;
use crate::app::{App, Config};
use crate::chat::{Chat, ChatOpts};
use crate::history::HistoryStore;

/// A directory of its own under the system's temp dir, removed when dropped.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "neo-term-images-{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }

    /// A solid red png of the given size; returns its path.
    fn png(&self, name: &str, width: u32, height: u32) -> PathBuf {
        let path = self.0.join(name);
        image::RgbImage::from_pixel(width, height, image::Rgb([220, 30, 30]))
            .save(&path)
            .unwrap();
        path
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn images(dir: &Dir) -> Images {
    Images::new(Picker::halfblocks(), dir.0.clone(), None)
}

/// Polls until `done` holds (the threads need a moment).
fn wait(images: &mut Images, done: impl Fn(&Images) -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !done(images) {
        assert!(Instant::now() < until, "timed out waiting for the images");
        images.poll();
        thread::sleep(Duration::from_millis(5));
    }
}

fn opts() -> ChatOpts {
    ChatOpts {
        expanded: false,
        show_thoughts: false,
        palette: Palette::default(),
        active: None,
        in_panel: false,
        marker: Style::new(),
        ui: Default::default(),
    }
}

fn image_item(r: &str, src: &str) -> ViewItem {
    serde_json::from_value(json!({
        "ref": r, "kind": "agent",
        "component": {"type":"image","src":src,"alt":"a red box"}
    }))
    .unwrap()
}

fn snapshot(items: Vec<ViewItem>) -> ViewSnapshot {
    ViewSnapshot {
        agent: None,
        session: "s".into(),
        seq: 1,
        items,
    }
}

/// Cells with a symbol or a background (half blocks of one color are a colored space).
fn non_blank(buf: &Buffer, area: Rect) -> usize {
    let mut n = 0;
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let cell = &buf[(x, y)];
            if !cell.symbol().trim().is_empty() || cell.bg != Color::Reset {
                n += 1;
            }
        }
    }
    n
}

fn row_text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol().to_string())
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[test]
fn the_env_value_picks_the_mode() {
    assert_eq!(parse_mode(None), Mode::Auto);
    assert_eq!(parse_mode(Some("")), Mode::Auto);
    assert_eq!(parse_mode(Some("auto")), Mode::Auto);
    assert_eq!(parse_mode(Some("off")), Mode::Off);
    assert_eq!(parse_mode(Some("OFF")), Mode::Off);
    assert_eq!(parse_mode(Some("kitty")), Mode::Force(ProtocolType::Kitty));
    assert_eq!(parse_mode(Some("sixel")), Mode::Force(ProtocolType::Sixel));
    assert_eq!(
        parse_mode(Some("iterm2")),
        Mode::Force(ProtocolType::Iterm2)
    );
    assert_eq!(
        parse_mode(Some(" halfblocks ")),
        Mode::Force(ProtocolType::Halfblocks)
    );
    assert!(detect(Mode::Off).is_none(), "off never asks the terminal");
}

#[test]
fn sources_resolve_to_files() {
    let cwd = Path::new("/work/site");
    let ok = |src: &str, assets: Option<&Assets>| resolve(src, cwd, assets);
    assert_eq!(ok("/tmp/a.png", None), Ok(PathBuf::from("/tmp/a.png")));
    assert_eq!(
        ok("shots/a.png", None),
        Ok(PathBuf::from("/work/site/shots/a.png"))
    );
    assert_eq!(
        ok("file:///tmp/a.png", None),
        Ok(PathBuf::from("/tmp/a.png"))
    );
    assert!(
        ok("https://x.test/a.png", None).is_err(),
        "no daemon to ask"
    );
    assert!(
        ok("data:image/png;base64,AAAA", None)
            .unwrap_err()
            .contains("inline")
    );
    assert!(ok("asset://img-1", None).is_err(), "no daemon to ask");
    let assets: Assets = Arc::new(|src| match src {
        "asset://img-1" => Ok(PathBuf::from("/store/img-1.png")),
        "https://x.test/a.png" => Ok(PathBuf::from("/store/img-2.png")),
        _ => Err("unknown asset".into()),
    });
    assert_eq!(
        ok("asset://img-1", Some(&assets)),
        Ok(PathBuf::from("/store/img-1.png"))
    );
    assert_eq!(
        ok("https://x.test/a.png", Some(&assets)),
        Ok(PathBuf::from("/store/img-2.png")),
        "the daemon downloads web images"
    );
    assert_eq!(
        ok("asset://nope", Some(&assets)),
        Err("unknown asset".into())
    );
}

#[test]
fn a_source_is_probed_once_and_its_size_becomes_known() {
    let dir = Dir::new("probe");
    let path = dir.png("a.png", 120, 80);
    let src = path.to_string_lossy().into_owned();
    let calls = Arc::new(AtomicUsize::new(0));
    let handle_calls = calls.clone();
    let target = path.clone();
    let assets: Assets = Arc::new(move |_| {
        handle_calls.fetch_add(1, Ordering::SeqCst);
        Ok(target.clone())
    });
    let mut images = Images::new(Picker::halfblocks(), dir.0.clone(), Some(assets));
    images.request(["asset://img-1", "asset://img-1", src.as_str(), src.as_str()]);
    let view = images.chat_view();
    assert_eq!(view.status["asset://img-1"], ImageStatus::Loading);
    wait(&mut images, |i| {
        i.chat_view()
            .status
            .values()
            .all(|s| *s != ImageStatus::Loading)
    });
    let status = images.chat_view().status;
    let ready = ImageStatus::Ready {
        width: 120,
        height: 80,
    };
    assert_eq!(status["asset://img-1"], ready);
    assert_eq!(status[&src], ready);
    // Asking again, even after the answer, does not look it up again.
    images.request(["asset://img-1"]);
    assert!(!images.poll());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn what_can_not_be_shown_says_why() {
    let dir = Dir::new("errors");
    std::fs::write(dir.0.join("notes.png"), "just text").unwrap();
    let mut images = images(&dir);
    images.request(["missing.png", "notes.png", "https://x.test/a.png"]);
    wait(&mut images, |i| {
        i.chat_view()
            .status
            .values()
            .all(|s| *s != ImageStatus::Loading)
    });
    let status = images.chat_view().status;
    assert_eq!(
        status["missing.png"],
        ImageStatus::Failed("file not found".into())
    );
    assert!(
        matches!(&status["notes.png"], ImageStatus::Failed(why) if why.contains("not an image"))
    );
    assert!(matches!(
        &status["https://x.test/a.png"],
        ImageStatus::Failed(_)
    ));
    // The chat shows the reason where the picture would be.
    let mut chat = Chat::new();
    chat.apply_snapshot(snapshot(vec![image_item("i", "missing.png")]));
    chat.set_images(Some(images.chat_view()));
    chat.layout(60, 10, &opts());
    assert_eq!(
        neo_renderer_ratatui::plain_text(&chat.visible()),
        ["[image a red box · file not found]"]
    );
    assert!(chat.placements().is_empty());
}

#[test]
fn an_image_in_view_is_encoded_once_and_drawn_into_its_rows() {
    let dir = Dir::new("draw");
    let path = dir.png("box.png", 400, 400);
    let src = path.to_string_lossy().into_owned();
    let mut images = images(&dir);
    let mut chat = Chat::new();
    chat.apply_snapshot(snapshot(vec![image_item("i", &src)]));
    images.request(chat.image_sources());
    wait(&mut images, |i| {
        matches!(
            i.chat_view().status.get(&src),
            Some(ImageStatus::Ready { .. })
        )
    });
    chat.set_images(Some(images.chat_view()));
    chat.layout(60, 20, &opts());
    // 400x400 at 10x20 pixel cells is 40x20 cells; 12 rows at most keeps the aspect: 24x12.
    let placements = chat.placements();
    assert_eq!(placements.len(), 1);
    let at = &placements[0];
    assert_eq!((at.x, at.y, at.cols, at.rows), (0, 0, 24, 12));
    let area = Rect::new(0, 0, 60, 20);
    let slot = Rect::new(0, 0, 24, 12);

    // Not encoded yet: the first row says so, nothing else of the slot is drawn.
    let mut buf = Buffer::empty(area);
    images.draw(&mut buf, area, at, Style::new());
    assert!(row_text(&buf, 0).starts_with("loading image…"));
    assert_eq!(images.encoded(), 1);

    wait(&mut images, |i| {
        matches!(i.protos.values().next(), Some(Proto::Ready(_)))
    });
    let mut buf = Buffer::empty(area);
    images.draw(&mut buf, area, at, Style::new());
    assert_eq!(
        non_blank(&buf, slot),
        24 * 12,
        "half blocks fill every cell"
    );
    assert_eq!(
        non_blank(&buf, Rect::new(24, 0, 36, 20)),
        0,
        "nothing beside it"
    );
    assert_eq!(
        non_blank(&buf, Rect::new(0, 12, 60, 8)),
        0,
        "nothing below it"
    );
    // The caption is part of the chat's own lines.
    assert_eq!(
        neo_renderer_ratatui::plain_text(&chat.visible())[12],
        "a red box · 400×400"
    );
    // Drawing again asks for nothing new.
    images.draw(&mut buf, area, at, Style::new());
    assert_eq!(images.encoded(), 1);
}

#[test]
fn a_partly_scrolled_image_is_cut_at_the_edges_of_the_pane() {
    let dir = Dir::new("clip");
    let path = dir.png("box.png", 400, 400);
    let src = path.to_string_lossy().into_owned();
    let mut images = images(&dir);
    images.request([src.as_str()]);
    wait(&mut images, |i| i.paths.contains_key(&src));
    let area = Rect::new(2, 1, 60, 10);
    let at = |y: i32| Placement {
        src: src.clone(),
        x: 3,
        y,
        cols: 24,
        rows: 12,
    };
    images.draw(
        &mut Buffer::empty(Rect::new(0, 0, 70, 12)),
        area,
        &at(0),
        Style::new(),
    );
    wait(&mut images, |i| {
        matches!(i.protos.values().next(), Some(Proto::Ready(_)))
    });
    let blank = || Buffer::empty(Rect::new(0, 0, 70, 12));
    // Top 5 rows scrolled away: 7 rows show, from the pane's first row.
    let mut buf = blank();
    images.draw(&mut buf, area, &at(-5), Style::new());
    assert_eq!(non_blank(&buf, Rect::new(5, 1, 24, 7)), 24 * 7);
    assert_eq!(non_blank(&buf, Rect::new(0, 0, 70, 12)), 24 * 7);
    // Starting 6 rows down, the pane ends after 4 of its 12 rows.
    let mut buf = blank();
    images.draw(&mut buf, area, &at(6), Style::new());
    assert_eq!(non_blank(&buf, Rect::new(5, 7, 24, 4)), 24 * 4);
    assert_eq!(non_blank(&buf, Rect::new(0, 0, 70, 12)), 24 * 4);
    // Fully out of view: nothing.
    for y in [-12, -20, 10, 30] {
        let mut buf = blank();
        images.draw(&mut buf, area, &at(y), Style::new());
        assert_eq!(non_blank(&buf, Rect::new(0, 0, 70, 12)), 0, "y {y}");
    }
}

#[test]
fn the_oldest_encodings_are_let_go() {
    let dir = Dir::new("lru");
    let mut images = images(&dir);
    for i in 0..KEEP_ENCODED + 5 {
        let key = (format!("{i}.png"), 10, 5);
        images.protos.insert(key.clone(), Proto::Pending);
        images.recent.push_back(key);
    }
    images.touch(&("0.png".to_string(), 10, 5));
    images.forget_old();
    assert_eq!(images.encoded(), KEEP_ENCODED);
    assert!(
        images.protos.contains_key(&("0.png".to_string(), 10, 5)),
        "drawn last"
    );
    assert!(!images.protos.contains_key(&("1.png".to_string(), 10, 5)));
    assert!(images.protos.contains_key(&("36.png".to_string(), 10, 5)));
}

// ---- in the app ----

fn app_with(images: Option<Images>, dir: &Dir) -> App {
    let mut app = App::new(Config {
        app_id: "terminal".into(),
        session: "s".into(),
        cwd: dir.0.to_string_lossy().into_owned(),
        home: None,
        no_color: false,
        history: HistoryStore::default(),
        prefs: crate::theme::Prefs::default(),
    });
    if let Some(images) = images {
        app.set_images(images);
    }
    app
}

fn screen(app: &mut App) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal.backend().buffer().clone()
}

fn screen_text(buf: &Buffer) -> Vec<String> {
    (0..buf.area.height).map(|y| row_text(buf, y)).collect()
}

#[test]
fn the_app_swaps_the_placeholder_for_the_picture() {
    let dir = Dir::new("app");
    let path = dir.png("box.png", 400, 400);
    let src = path.to_string_lossy().into_owned();
    let mut app = app_with(Some(images(&dir)), &dir);
    app.handle_event(Event::ViewSnapshot(snapshot(vec![image_item("i", &src)])));

    // The first frame asks; until the size is known it is a line of text.
    let first = screen_text(&screen(&mut app));
    assert!(
        first
            .iter()
            .any(|l| l.contains("[image a red box] loading image…")),
        "{first:#?}"
    );

    // Then the rows are reserved and the picture fills them.
    let until = Instant::now() + Duration::from_secs(10);
    let buf = loop {
        app.tick(0);
        if app.take_dirty() {
            let buf = screen(&mut app);
            let text = screen_text(&buf);
            if text.iter().any(|l| l.contains("a red box · 400×400"))
                && !text.iter().any(|l| l.contains("loading image"))
            {
                break buf;
            }
        }
        assert!(Instant::now() < until, "the picture never came");
        thread::sleep(Duration::from_millis(5));
    };
    let text = screen_text(&buf);
    let caption = text
        .iter()
        .position(|l| l.contains("a red box · 400×400"))
        .unwrap();
    assert!(caption >= 12, "twelve rows above the caption: {text:#?}");
    let slot_top = caption - 12;
    // Chat items start at the left edge of the chat; the sessions sidebar may sit before it.
    let x0 = (0..buf.area.width)
        .find(|x| buf[(*x, slot_top as u16)].bg != Color::Reset)
        .expect("a painted cell in the slot's first row");
    assert_eq!(
        non_blank(&buf, Rect::new(x0, slot_top as u16, 24, 12)),
        24 * 12
    );
}

#[test]
fn without_images_the_placeholder_stays() {
    let dir = Dir::new("off");
    let path = dir.png("box.png", 400, 400);
    let src = path.to_string_lossy().into_owned();
    let mut app = app_with(None, &dir);
    app.handle_event(Event::ViewSnapshot(snapshot(vec![image_item("i", &src)])));
    let text = screen_text(&screen(&mut app));
    assert!(
        text.iter().any(|l| l.contains("[image a red box]")),
        "{text:#?}"
    );
    assert!(!text.iter().any(|l| l.contains("loading image")));
}

#[test]
fn every_protocol_encodes_and_draws_into_the_buffer() {
    let dir = Dir::new("protocols");
    let path = dir.png("box.png", 200, 200);
    let src = path.to_string_lossy().into_owned();
    for protocol in [
        ProtocolType::Halfblocks,
        ProtocolType::Kitty,
        ProtocolType::Sixel,
        ProtocolType::Iterm2,
    ] {
        let mut picker = Picker::halfblocks();
        picker.set_protocol_type(protocol);
        let mut images = Images::new(picker, dir.0.clone(), None);
        assert_eq!(images.protocol(), protocol);
        images.request([src.as_str()]);
        wait(&mut images, |i| i.paths.contains_key(&src));
        let area = Rect::new(0, 0, 40, 12);
        let at = Placement {
            src: src.clone(),
            x: 2,
            y: 1,
            cols: 10,
            rows: 5,
        };
        images.draw(&mut Buffer::empty(area), area, &at, Style::new());
        wait(&mut images, |i| {
            matches!(i.protos.values().next(), Some(Proto::Ready(_)))
        });
        let mut buf = Buffer::empty(area);
        images.draw(&mut buf, area, &at, Style::new());
        assert!(
            non_blank(&buf, Rect::new(2, 1, 10, 5)) > 0,
            "{protocol:?} drew nothing"
        );
        assert_eq!(non_blank(&buf, Rect::new(0, 6, 40, 6)), 0, "{protocol:?}");
    }
}

#[test]
fn kitty_and_ghostty_under_tmux_get_kitty_graphics() {
    assert_eq!(
        outer_protocol("kitty(0.48.2)|xterm-kitty"),
        Some(ProtocolType::Kitty)
    );
    assert_eq!(
        outer_protocol("ghostty 1.2.0|xterm-ghostty"),
        Some(ProtocolType::Kitty)
    );
    assert_eq!(outer_protocol("|xterm-256color"), None);
}

#[test]
fn a_pasted_image_is_drawn_above_the_editor() {
    let dir = Dir::new("preview");
    let path = dir.png("shot.png", 400, 200);
    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(ProtocolType::Kitty);
    let mut app = app_with(Some(Images::new(picker, dir.0.clone(), None)), &dir);
    app.handle_paste(&path.to_string_lossy());
    let until = Instant::now() + Duration::from_secs(10);
    let buf = loop {
        app.tick(0);
        let buf = screen(&mut app);
        let text = screen_text(&buf);
        if !text.iter().any(|l| l.contains("loading"))
            && text.iter().any(|l| l.contains("[image 1]"))
        {
            break buf;
        }
        assert!(Instant::now() < until, "the preview never came: {text:#?}");
        thread::sleep(Duration::from_millis(5));
    };
    // The picture's kitty placeholders fill the rows above its marker.
    let text = screen_text(&buf);
    let label = text
        .iter()
        .position(|l| l.trim_start().starts_with("[image 1]"))
        .unwrap();
    let placeholders = (0..buf.area.width)
        .filter(|x| {
            buf[(*x, (label - 1) as u16)]
                .symbol()
                .starts_with('\u{10eeee}')
        })
        .count();
    assert!(placeholders > 0, "{text:#?}");
}
