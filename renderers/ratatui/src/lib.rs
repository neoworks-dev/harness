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
mod summary;
mod text;

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
    pub theme: Palette,
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
        }),
        pb::Component::CardComponent(v) => v.children.iter().for_each(|c| collect(c, out)),
        pb::Component::RowComponent(v) => v.children.iter().for_each(|c| collect(c, out)),
        pb::Component::StackComponent(v) => v.children.iter().for_each(|c| collect(c, out)),
        _ => {}
    }
}

/// The item as styled lines, none wider than `width` cells (no lines at all for width 0).
pub fn render_item(item: &ViewItem, width: u16, opts: &Options) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    render::Cx::new(opts).item(item, usize::from(width))
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
