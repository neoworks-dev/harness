//! Helpers for the widget tests: buffers as plain text.

use ratatui::buffer::Buffer;

/// One line per row, trailing blanks trimmed.
pub(crate) fn dump(buf: &Buffer) -> String {
    let area = buf.area;
    let mut out = Vec::new();
    for y in area.y..area.bottom() {
        let mut row = String::new();
        for x in area.x..area.right() {
            row.push_str(buf[(x, y)].symbol());
        }
        out.push(row.trim_end().to_string());
    }
    out.join("\n")
}
