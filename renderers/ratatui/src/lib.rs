//! Draws the suite's resolved view items (`protocol/schemas/view.schema.json`) as styled
//! [ratatui] lines, for full-screen terminal apps.
//!
//! The renderer is pure: [`render_item`] turns one [`ViewItem`] into lines wrapped to a width,
//! and the app scrolls over them, so it can cache the lines per item until the item, the width
//! or the options change. Separating items (one blank line is what `renderers/tui` does) and
//! the input box are the app's job. See `renderers/README.md` for what every renderer shares.
//!
//! ```
//! use suite_renderer_ratatui::{Options, render_item};
//! # let item: suite_protocol::ViewItem = serde_json::from_str(
//! #   r#"{"ref":"m1","kind":"agent","component":{"type":"text","blocks":[
//! #   {"type":"paragraph","spans":[{"text":"hello"}]}]}}"#).unwrap();
//! let lines = render_item(&item, 80, &Options::default());
//! assert_eq!(lines.len(), 1);
//! ```

mod palette;
mod render;
mod shell;
mod summary;
mod text;

use std::collections::HashMap;
use std::sync::Arc;

use ratatui::text::Line;
use suite_protocol as pb;
use suite_protocol::ViewItem;

pub use palette::Palette;

/// What to draw and how.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Tool items: the full result under the header instead of a summary of up to three lines.
    /// Items with buttons are always drawn in full.
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
    pub theme: Palette,
    /// Reserve rows for images the app can draw itself (see [`ImageOptions`]). None: an image is
    /// a one-line `[image …]` placeholder.
    pub images: Option<ImageOptions>,
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
    /// `app.tool`, `copy:<text>`, `suite.permit`, `suite.choose`.
    pub action: String,
    pub args: Option<serde_json::Value>,
    /// What triggers it: the button's own key, else its 1-based position among the item's
    /// buttons. The same text is drawn in the button.
    pub key: String,
    /// The value in effect now (a picker's current model): focus starts here.
    pub current: bool,
    /// What a choice says about this option: the text after the button in its row.
    pub description: Option<String>,
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
            pb::Block::RuleBlock(_) | pb::Block::Unknown(_) => {}
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
