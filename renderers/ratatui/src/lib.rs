//! Draws neo's resolved view items (`protocol/schemas/view.schema.json`) as styled
//! [ratatui] lines, for full-screen terminal apps.
//!
//! The renderer is pure: [`render_item`] turns one [`ViewItem`] into lines wrapped to a width,
//! and the app scrolls over them, so it can cache the lines per item until the item, the width
//! or the options change. Separating items (one blank line is what `renderers/tui` does) and
//! the input box are the app's job. See `renderers/README.md` for what every renderer shares.
//!
//! ```
//! use neo_renderer_ratatui::{Options, render_item};
//! # let item: neo_protocol::ViewItem = serde_json::from_str(
//! #   r#"{"ref":"m1","kind":"agent","component":{"type":"text","blocks":[
//! #   {"type":"paragraph","spans":[{"text":"hello"}]}]}}"#).unwrap();
//! let lines = render_item(&item, 80, &Options::default());
//! assert_eq!(lines.len(), 1);
//! ```

mod chart_image;
mod icons;
mod neoui;
mod palette;
mod render;
mod shell;
mod summary;
pub mod syntax;
mod text;

use std::collections::HashMap;
use std::sync::Arc;

use neo_protocol as pb;
use neo_protocol::ViewItem;
use ratatui::text::Line;

pub use chart_image::{CHART_SCHEME, ChartKind, ChartSpec, SpecSeries, draw_chart};
pub use icons::file_icon;
pub use neoui::{
    UiDocInfo, UiEdit, UiKind, UiOption, UiState, UiTarget, is_date, neoui_docs, neoui_targets,
};
pub use palette::Palette;

/// What to draw and how.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Tool items: the full result under the header instead of a summary of up to three lines.
    /// Items with buttons, and neoui docs with fields or buttons, are always drawn in full.
    pub expanded: bool,
    /// Thought items: the text (dim italic) instead of one `∴ thinking` line.
    pub show_thoughts: bool,
    /// Items with buttons (a permission request previewing a diff) show their diffs and code in
    /// full; with this set they stop after that many lines in all and say how many are left, so
    /// the buttons stay near. `expanded` shows everything. None: no limit.
    pub preview_lines: Option<usize>,
    /// Highlights the n-th button of this item (index into [`buttons`]).
    pub focused_button: Option<usize>,
    /// Draws a button as `[Label]` without the key that triggers it (a card that takes its
    /// keys for something else, such as a filter).
    pub hide_keys: bool,
    /// Prose (paragraphs, headings, lists, quotes) wraps at this many cells at most, however
    /// wide the item; tables and code still use the whole width. None: the whole width.
    pub prose_width: Option<usize>,
    pub theme: Palette,
    /// Reserve rows for images the app can draw itself (see [`ImageOptions`]). None: an image is
    /// a one-line `[image …]` placeholder.
    pub images: Option<ImageOptions>,
    /// The visual state of the item's neoui docs (focus, tabs, edits…), per doc id.
    pub neoui: HashMap<String, UiState>,
    /// The app drawing the chat: the user's messages from it (or another of its instances,
    /// `terminal-2`) leave out their `· app` tag. None: every message has it.
    pub app: Option<String>,
    /// A diff's path starts with its file type icon (a Nerd Font glyph, see [`file_icon`]).
    pub file_icons: bool,
    /// The session's directory: paths in it (a diff's, a tool's label) are shown relative to it.
    pub cwd: Option<String>,
    /// Code blocks and diffs without syntax colors: the same lines, much quicker. An app lays a
    /// long conversation out with this and highlights only what comes into view.
    pub plain_code: bool,
    /// Diffs side by side (old left, new right) where there is room for both, else unified.
    pub split_diffs: bool,
    /// A finished edit (a call with one diff) shows its changes, with a few lines around them,
    /// under its `edit path  +3 −1` line, rather than that line alone.
    pub open_edits: bool,
}

/// `path` relative to `cwd` when it is inside it, else as it is.
pub fn relative_path<'a>(path: &'a str, cwd: Option<&str>) -> &'a str {
    let Some(cwd) = cwd
        .map(|c| c.trim_end_matches('/'))
        .filter(|c| !c.is_empty())
    else {
        return path;
    };
    path.strip_prefix(cwd)
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(path)
}

/// `command` without a leading `cd` into `cwd` (`cd /w/site && ls`, `cd "/w/site"; ls`): agents
/// start most commands with one, and the session's directory goes without saying. Anything
/// else, a `cd` elsewhere or nothing after it, stays as it is.
pub fn strip_cd<'a>(command: &'a str, cwd: Option<&str>) -> &'a str {
    let Some(cwd) = cwd
        .map(|c| c.trim_end_matches('/'))
        .filter(|c| !c.is_empty())
    else {
        return command;
    };
    let Some(rest) = command.trim_start().strip_prefix("cd ") else {
        return command;
    };
    let rest = rest.trim_start_matches(' ');
    let after = ["\"", "'", ""].iter().find_map(|q| {
        let r = rest.strip_prefix(q)?.strip_prefix(cwd)?;
        let r = r.strip_prefix('/').unwrap_or(r);
        r.strip_prefix(q)
    });
    let Some(after) = after else {
        return command;
    };
    let after = after.trim_start_matches(' ');
    let next = after
        .strip_prefix("&&")
        .or_else(|| after.strip_prefix(';'))
        .or_else(|| after.strip_prefix('\n'))
        .map(str::trim_start)
        .filter(|n| !n.is_empty());
    next.unwrap_or(command)
}

/// What the app knows about an image source (`src` of an image component).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageStatus {
    /// Not looked at yet, or on its way.
    Loading,
    /// Readable, with its size in pixels.
    Ready { width: u32, height: u32 },
    /// Why it can't be shown.
    Failed(String),
}

/// Images drawn by the app over lines the renderer reserves for them.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageOptions {
    /// Rows a thumbnail may take at most.
    pub max_rows: u16,
    /// Pixel size of a terminal cell (width, height), to keep the aspect ratio.
    pub cell: (u16, u16),
    /// Per source; one that is missing counts as [`ImageStatus::Loading`].
    pub status: Arc<HashMap<String, ImageStatus>>,
    /// Charts are pictures: room under a `chart:` source ([`ChartSpec`]) that the app draws
    /// with [`draw_chart`]; their labels stay text. Needs a true-color theme; else, and while
    /// a doc streams in, charts are characters.
    pub charts: bool,
}

/// Where an item's lines hold room for an image: the app draws it there. Only images that are
/// [`ImageStatus::Ready`] get room; the others stay a line of text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSlot {
    /// Index of the first reserved line within the item's lines.
    pub line: usize,
    /// Column of the slot's left edge.
    pub x: u16,
    pub cols: u16,
    pub rows: u16,
    pub src: String,
}

/// An item's lines and the room they hold for images.
#[derive(Debug, Clone, Default)]
pub struct Rendered {
    pub lines: Vec<Line<'static>>,
    pub images: Vec<ImageSlot>,
    /// The first line of the focused neoui target, if the state has one in view.
    pub focus_line: Option<usize>,
}

/// The cells a `width`x`height` pixel image takes in a terminal whose cells are `cell` pixels:
/// its natural size, shrunk (never grown) to fit `max_cols` x `max_rows`, aspect ratio kept.
/// At least 1x1.
pub fn image_cells(
    width: u32,
    height: u32,
    cell: (u16, u16),
    max_cols: u16,
    max_rows: u16,
) -> (u16, u16) {
    let (cw, ch) = (u32::from(cell.0.max(1)), u32::from(cell.1.max(1)));
    let natural = (
        f64::from(width.div_ceil(cw).max(1)),
        f64::from(height.div_ceil(ch).max(1)),
    );
    let scale = (f64::from(max_cols.max(1)) / natural.0)
        .min(f64::from(max_rows.max(1)) / natural.1)
        .min(1.0);
    let cols = ((natural.0 * scale).round() as u16).clamp(1, max_cols.max(1));
    let rows = ((natural.1 * scale).round() as u16).clamp(1, max_rows.max(1));
    (cols, rows)
}

/// A button of an item as the app presents it.
#[derive(Debug, Clone, PartialEq)]
pub struct ButtonRef {
    pub label: String,
    /// `app.tool`, `copy:<text>`, `neo.permit`, `neo.choose`.
    pub action: String,
    pub args: Option<serde_json::Value>,
    /// What triggers it: the button's own key, else its 1-based position among the item's
    /// buttons. The same text is drawn in the button.
    pub key: String,
    /// The value in effect now (a picker's current model): focus starts here.
    pub current: bool,
    /// What a choice says about this option: the text after the button in its row.
    pub description: Option<String>,
    /// The answer is the agent's to hear: a note for it may go with the press (`args.note`).
    pub note: bool,
}

impl ButtonRef {
    /// The text to copy when this is a `copy:` button, which the client handles itself instead
    /// of sending `ui.action`.
    pub fn copy_text(&self) -> Option<&str> {
        self.action.strip_prefix("copy:")
    }
}

/// The item's buttons in drawing order: [`Options::focused_button`] indexes this list.
pub fn buttons(item: &ViewItem) -> Vec<ButtonRef> {
    let mut out = Vec::new();
    collect(&item.component, &mut out);
    out
}

fn collect(c: &pb::Component, out: &mut Vec<ButtonRef>) {
    match c {
        pb::Component::ButtonComponent(b) => out.push(ButtonRef {
            label: b.label.clone(),
            action: b.action.clone(),
            args: b.args.clone(),
            key: render::button_key(b, out.len()),
            current: b.current == Some(true),
            description: None,
            note: b.note == Some(true),
        }),
        pb::Component::CardComponent(v) => v.children.iter().for_each(|c| collect(c, out)),
        pb::Component::RowComponent(v) => {
            let mut last: Option<usize> = None;
            for child in &v.children {
                match child {
                    // The first text after a button in a row says what the button does.
                    pb::Component::TextComponent(t) => {
                        if let Some(i) = last.take()
                            && let Some(button) = out.get_mut(i)
                        {
                            button.description = Some(blocks_text(&t.blocks));
                        }
                    }
                    other => {
                        let before = out.len();
                        collect(other, out);
                        last = (out.len() > before).then(|| out.len() - 1);
                    }
                }
            }
        }
        pb::Component::StackComponent(v) => v.children.iter().for_each(|c| collect(c, out)),
        _ => {}
    }
}

/// The text of a run of blocks, one per line.
fn blocks_text(blocks: &[pb::Block]) -> String {
    let spans = |list: &[pb::Span]| list.iter().map(|s| s.text.as_str()).collect::<String>();
    let mut lines = Vec::new();
    for block in blocks {
        match block {
            pb::Block::ParagraphBlock(b) => lines.push(spans(&b.spans)),
            pb::Block::HeadingBlock(b) => lines.push(spans(&b.spans)),
            pb::Block::CodeBlock(b) => lines.push(b.code.clone()),
            pb::Block::QuoteBlock(b) => lines.push(blocks_text(&b.blocks)),
            pb::Block::ListBlock(b) => lines.extend(
                b.items
                    .iter()
                    .map(|item| format!("- {}", blocks_text(&item.blocks))),
            ),
            pb::Block::TableBlock(b) => {
                let row = |cells: &[pb::TableCell]| {
                    cells
                        .iter()
                        .map(|c| spans(&c.spans))
                        .collect::<Vec<_>>()
                        .join(" | ")
                };
                lines.push(row(&b.header));
                lines.extend(b.rows.iter().map(|r| row(r)));
            }
            pb::Block::RuleBlock(_) | pb::Block::NeoUiBlock(_) | pb::Block::Unknown(_) => {}
        }
    }
    lines.join("\n")
}

/// The item with only the buttons at the `keep` positions of [`buttons`] (and the text that
/// follows each in its row). Everything else stays; a row left without a button goes.
pub fn keep_buttons(item: &ViewItem, keep: &[usize]) -> ViewItem {
    let mut item = item.clone();
    let mut next = 0;
    item.component = prune(item.component, keep, &mut next).unwrap_or_else(|| {
        pb::Component::StackComponent(pb::StackComponent {
            gap: None,
            children: Vec::new(),
        })
    });
    item
}

fn prune(c: pb::Component, keep: &[usize], next: &mut usize) -> Option<pb::Component> {
    match c {
        pb::Component::ButtonComponent(_) => {
            let at = *next;
            *next += 1;
            keep.contains(&at).then_some(c)
        }
        pb::Component::CardComponent(mut v) => {
            v.children = prune_children(v.children, false, keep, next);
            Some(pb::Component::CardComponent(v))
        }
        pb::Component::StackComponent(mut v) => {
            v.children = prune_children(v.children, false, keep, next);
            Some(pb::Component::StackComponent(v))
        }
        pb::Component::RowComponent(mut v) => {
            let had = v.children.iter().any(render::has_buttons);
            v.children = prune_children(v.children, true, keep, next);
            let has = v.children.iter().any(render::has_buttons);
            (has || !had).then_some(pb::Component::RowComponent(v))
        }
        other => Some(other),
    }
}

fn prune_children(
    children: Vec<pb::Component>,
    row: bool,
    keep: &[usize],
    next: &mut usize,
) -> Vec<pb::Component> {
    let mut out = Vec::new();
    // Whether the last button seen in a row was kept: its text follows it.
    let mut last = None;
    for child in children {
        let is_text = matches!(child, pb::Component::TextComponent(_));
        if row && is_text && last.is_some() {
            if last == Some(true) {
                out.push(child);
            }
            last = None;
            continue;
        }
        let buttons = render::has_buttons(&child);
        let kept = prune(child, keep, next);
        if buttons {
            last = Some(kept.as_ref().is_some_and(render::has_buttons));
        }
        out.extend(kept);
    }
    out
}

/// The item as styled lines, none wider than `width` cells (no lines at all for width 0).
/// A shell command as styled spans (command names, strings, variables, operators, comments),
/// newlines kept and the text unchanged; `base` is patched under every span.
pub fn highlight_shell(
    source: &str,
    palette: &Palette,
    base: ratatui::style::Style,
) -> Vec<ratatui::text::Span<'static>> {
    shell::highlight(source, palette, base)
}

pub fn render_item(item: &ViewItem, width: u16, opts: &Options) -> Vec<Line<'static>> {
    render_item_images(item, width, opts).lines
}

/// [`render_item`] plus where the item's images go (see [`Options::images`]).
pub fn render_item_images(item: &ViewItem, width: u16, opts: &Options) -> Rendered {
    if width == 0 {
        return Rendered::default();
    }
    let mut cx = render::Cx::new(opts);
    let lines = cx.item(item, usize::from(width));
    Rendered {
        lines,
        images: cx.take_slots(),
        focus_line: cx.ui.focus_line,
    }
}

/// The `src` of every image component of the item that is drawn in full (not inside a table or
/// the like): what an app has to look at to show them.
pub fn image_sources(item: &ViewItem) -> Vec<String> {
    let mut out = Vec::new();
    render::sources(&item.component, &mut out);
    out
}

/// The visible text of lines, one string per line, trailing blanks trimmed. For tests, copying
/// and snapshots.
pub fn plain_text(lines: &[Line<'_>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            let s: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            s.trim_end().to_string()
        })
        .collect()
}
