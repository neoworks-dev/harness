//! The multi-line input editor: text, a cursor and soft wrapping. Pure state; `app` draws it.

use unicode_width::UnicodeWidthChar;

use crate::util::width;

/// One visual row of the wrapped text: bytes `start..end` of the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub start: usize,
    pub end: usize,
    /// The last row of its logical line (the cursor may sit at `end`).
    pub last: bool,
}

#[derive(Debug, Clone)]
pub struct Editor {
    text: String,
    /// Byte offset, always on a char boundary.
    cursor: usize,
    /// Column kept while moving up and down across short rows.
    goal: Option<usize>,
    /// Cells available for text; rows wrap at it.
    width: usize,
}

impl Default for Editor {
    fn default() -> Self {
        Editor {
            text: String::new(),
            cursor: 0,
            goal: None,
            width: 80,
        }
    }
}

impl Editor {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_width(&mut self, width: usize) {
        self.width = width.max(1);
    }

    pub fn set_text(&mut self, text: &str) {
        self.text = clean(text);
        self.cursor = self.text.len();
        self.goal = None;
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.goal = None;
    }

    /// The text, leaving the editor empty.
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        self.goal = None;
        std::mem::take(&mut self.text)
    }

    pub fn insert_char(&mut self, ch: char) {
        let mut buf = [0u8; 4];
        self.insert_str(ch.encode_utf8(&mut buf));
    }

    /// Inserts text at the cursor: `\r\n` and `\r` become `\n`, tabs four spaces, other control
    /// characters are dropped.
    pub fn insert_str(&mut self, s: &str) {
        let s = clean(s);
        self.text.insert_str(self.cursor, &s);
        self.cursor += s.len();
        self.goal = None;
    }

    pub fn newline(&mut self) {
        self.insert_str("\n");
    }

    pub fn backspace(&mut self) {
        if let Some(prev) = self.prev_boundary(self.cursor) {
            self.text.replace_range(prev..self.cursor, "");
            self.cursor = prev;
        }
        self.goal = None;
    }

    /// Deletes the `bytes` before the cursor (they must end and start on character boundaries).
    pub fn delete_before(&mut self, bytes: usize) {
        let start = self.cursor.saturating_sub(bytes);
        if self.text.is_char_boundary(start) {
            self.text.replace_range(start..self.cursor, "");
            self.cursor = start;
        }
        self.goal = None;
    }

    pub fn delete(&mut self) {
        if let Some(next) = self.next_boundary(self.cursor) {
            self.text.replace_range(self.cursor..next, "");
        }
        self.goal = None;
    }

    pub fn left(&mut self) {
        if let Some(prev) = self.prev_boundary(self.cursor) {
            self.cursor = prev;
        }
        self.goal = None;
    }

    pub fn right(&mut self) {
        if let Some(next) = self.next_boundary(self.cursor) {
            self.cursor = next;
        }
        self.goal = None;
    }

    pub fn word_left(&mut self) {
        self.cursor = self.word_start_before(self.cursor);
        self.goal = None;
    }

    pub fn word_right(&mut self) {
        self.cursor = self.word_end_after(self.cursor);
        self.goal = None;
    }

    /// Start of the logical line (Ctrl-A).
    pub fn line_start(&mut self) {
        self.cursor = self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1);
        self.goal = None;
    }

    /// End of the logical line (Ctrl-E).
    pub fn line_end(&mut self) {
        self.cursor = self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |i| self.cursor + i);
        self.goal = None;
    }

    pub fn text_start(&mut self) {
        self.cursor = 0;
        self.goal = None;
    }

    pub fn text_end(&mut self) {
        self.cursor = self.text.len();
        self.goal = None;
    }

    /// Ctrl-W: the word before the cursor.
    pub fn delete_word_back(&mut self) {
        let start = self.word_start_before(self.cursor);
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
        self.goal = None;
    }

    /// Alt-D: the word after the cursor.
    pub fn delete_word_forward(&mut self) {
        let end = self.word_end_after(self.cursor);
        self.text.replace_range(self.cursor..end, "");
        self.goal = None;
    }

    /// Ctrl-U: back to the start of the line (at the start: joins with the previous line).
    pub fn delete_to_line_start(&mut self) {
        let start = self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1);
        let from = if start == self.cursor {
            self.prev_boundary(self.cursor).unwrap_or(start)
        } else {
            start
        };
        self.text.replace_range(from..self.cursor, "");
        self.cursor = from;
        self.goal = None;
    }

    /// Ctrl-K: to the end of the line (at the end: joins with the next line).
    pub fn delete_to_line_end(&mut self) {
        let end = self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |i| self.cursor + i);
        let to = if end == self.cursor {
            self.next_boundary(self.cursor).unwrap_or(end)
        } else {
            end
        };
        self.text.replace_range(self.cursor..to, "");
        self.goal = None;
    }

    /// The wrapped rows at the current width (at least one).
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut base = 0;
        for line in self.text.split('\n') {
            wrap_line(line, base, self.width, &mut rows);
            base += line.len() + 1;
        }
        rows
    }

    /// How many rows the text takes.
    pub fn height(&self) -> usize {
        self.rows().len()
    }

    /// The cursor's row and its column in cells within that row.
    pub fn cursor_pos(&self) -> (usize, usize) {
        let rows = self.rows();
        let r = row_of(&rows, self.cursor);
        (r, width(&self.text[rows[r].start..self.cursor]))
    }

    pub fn on_first_row(&self) -> bool {
        self.cursor_pos().0 == 0
    }

    pub fn on_last_row(&self) -> bool {
        self.cursor_pos().0 + 1 == self.rows().len()
    }

    /// Moves up one visual row; false (and no move) on the first.
    pub fn up(&mut self) -> bool {
        let rows = self.rows();
        let r = row_of(&rows, self.cursor);
        if r == 0 {
            return false;
        }
        let col = self
            .goal
            .unwrap_or_else(|| width(&self.text[rows[r].start..self.cursor]));
        self.cursor = self.byte_at_col(&rows[r - 1], col);
        self.goal = Some(col);
        true
    }

    /// Moves down one visual row; false (and no move) on the last.
    pub fn down(&mut self) -> bool {
        let rows = self.rows();
        let r = row_of(&rows, self.cursor);
        if r + 1 >= rows.len() {
            return false;
        }
        let col = self
            .goal
            .unwrap_or_else(|| width(&self.text[rows[r].start..self.cursor]));
        self.cursor = self.byte_at_col(&rows[r + 1], col);
        self.goal = Some(col);
        true
    }

    fn byte_at_col(&self, row: &Row, col: usize) -> usize {
        let mut pos = row.start;
        let mut acc = 0;
        for (idx, ch) in self.text[row.start..row.end].char_indices() {
            let w = ch.width().unwrap_or(0);
            if acc + w > col {
                break;
            }
            acc += w;
            pos = row.start + idx + ch.len_utf8();
        }
        if !row.last && pos == row.end && row.end > row.start {
            // The end of a wrapped row is the start of the next one.
            pos = self.prev_boundary(pos).unwrap_or(pos);
        }
        pos
    }

    fn prev_boundary(&self, at: usize) -> Option<usize> {
        self.text[..at].char_indices().next_back().map(|(i, _)| i)
    }

    fn next_boundary(&self, at: usize) -> Option<usize> {
        self.text[at..].chars().next().map(|c| at + c.len_utf8())
    }

    fn word_start_before(&self, at: usize) -> usize {
        let before = &self.text[..at];
        let trimmed = before.trim_end_matches(char::is_whitespace);
        trimmed.rfind(char::is_whitespace).map_or(0, |i| {
            i + trimmed[i..].chars().next().map_or(1, char::len_utf8)
        })
    }

    fn word_end_after(&self, at: usize) -> usize {
        let after = &self.text[at..];
        let rest = after.trim_start_matches(char::is_whitespace);
        let skipped = after.len() - rest.len();
        at + skipped + rest.find(char::is_whitespace).unwrap_or(rest.len())
    }
}

fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\t' => out.push_str("    "),
            '\n' => out.push('\n'),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

fn row_of(rows: &[Row], cursor: usize) -> usize {
    rows.iter()
        .position(|r| cursor >= r.start && (cursor < r.end || (cursor == r.end && r.last)))
        .unwrap_or(rows.len() - 1)
}

/// Greedy word wrap of one logical line that starts at byte `base` of the buffer.
fn wrap_line(line: &str, base: usize, max: usize, rows: &mut Vec<Row>) {
    let mut start = 0;
    let mut cols = 0;
    // Byte just after the last space in the current row.
    let mut after_space: Option<usize> = None;
    for (idx, ch) in line.char_indices() {
        let w = ch.width().unwrap_or(0);
        if cols + w > max && idx > start {
            if ch == ' ' {
                // A space at the edge hangs off the row instead of starting the next.
                rows.push(Row {
                    start: base + start,
                    end: base + idx + 1,
                    last: false,
                });
                start = idx + 1;
                cols = 0;
                after_space = None;
                continue;
            }
            match after_space.filter(|&sp| sp > start) {
                Some(sp) => {
                    rows.push(Row {
                        start: base + start,
                        end: base + sp,
                        last: false,
                    });
                    cols = width(&line[sp..idx]);
                    start = sp;
                }
                None => {
                    rows.push(Row {
                        start: base + start,
                        end: base + idx,
                        last: false,
                    });
                    cols = 0;
                    start = idx;
                }
            }
            after_space = None;
        }
        cols += w;
        if ch == ' ' {
            after_space = Some(idx + 1);
        }
    }
    rows.push(Row {
        start: base + start,
        end: base + line.len(),
        last: true,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed(text: &str) -> Editor {
        let mut e = Editor::default();
        e.insert_str(text);
        e
    }

    #[test]
    fn typing_and_deleting() {
        let mut e = ed("héllo");
        assert_eq!(e.text(), "héllo");
        e.backspace();
        e.left();
        e.insert_char('X');
        assert_eq!(e.text(), "hélXl");
        e.delete();
        assert_eq!(e.text(), "hélX");
        e.text_start();
        e.backspace();
        assert_eq!(e.text(), "hélX");
        e.right();
        e.right();
        e.delete();
        assert_eq!(e.text(), "héX");
    }

    #[test]
    fn paste_is_normalized_and_stays_in_the_editor() {
        let mut e = Editor::default();
        e.insert_str("a\r\nb\rc\td\u{1b}[31m");
        assert_eq!(e.text(), "a\nb\nc    d[31m");
        assert_eq!(e.height(), 3);
    }

    #[test]
    fn word_moves_and_deletes() {
        let mut e = ed("one two  three");
        e.word_left();
        assert_eq!(&e.text()[e.cursor()..], "three");
        e.word_left();
        assert_eq!(&e.text()[e.cursor()..], "two  three");
        e.word_right();
        assert_eq!(&e.text()[e.cursor()..], "  three");
        e.delete_word_back();
        assert_eq!(e.text(), "one   three");
        e.delete_word_forward();
        assert_eq!(e.text(), "one ");
        e.text_end();
        e.delete_word_back();
        assert_eq!(e.text(), "");
    }

    #[test]
    fn line_kills() {
        let mut e = ed("first\nsecond");
        e.left();
        e.left();
        e.delete_to_line_start();
        assert_eq!(e.text(), "first\nnd");
        e.delete_to_line_start();
        assert_eq!(e.text(), "firstnd");
        e.line_start();
        e.delete_to_line_end();
        assert_eq!(e.text(), "");
        let mut e = ed("a\nb");
        e.text_start();
        e.line_end();
        e.delete_to_line_end();
        assert_eq!(e.text(), "ab");
    }

    #[test]
    fn ctrl_a_and_e_work_on_the_logical_line() {
        let mut e = ed("ab\ncde");
        e.line_start();
        assert_eq!(e.cursor(), 3);
        e.line_end();
        assert_eq!(e.cursor(), 6);
        e.up();
        e.line_end();
        assert_eq!(e.cursor(), 2);
    }

    #[test]
    fn up_and_down_keep_the_column_across_short_rows() {
        let mut e = ed("abcdef\nab\nabcdef");
        assert_eq!(e.cursor_pos(), (2, 6));
        assert!(e.up());
        assert_eq!(e.cursor_pos(), (1, 2));
        assert!(e.up());
        assert_eq!(e.cursor_pos(), (0, 6));
        assert!(!e.up());
        assert!(e.down());
        assert!(e.down());
        assert_eq!(e.cursor_pos(), (2, 6));
        assert!(!e.down());
    }

    #[test]
    fn rows_wrap_at_the_width_preferring_spaces() {
        let mut e = ed("hello brave new world");
        e.set_width(10);
        let rows = e.rows();
        let texts: Vec<_> = rows.iter().map(|r| &e.text()[r.start..r.end]).collect();
        assert_eq!(texts, ["hello ", "brave new ", "world"]);
        // A word longer than the width is cut.
        let mut e = ed("abcdefghijkl");
        e.set_width(5);
        assert_eq!(e.height(), 3);
    }

    #[test]
    fn the_cursor_at_a_wrap_belongs_to_the_next_row() {
        let mut e = ed("hello brave");
        e.set_width(6);
        e.text_end();
        assert_eq!(e.cursor_pos(), (1, 5));
        e.left();
        e.left();
        e.left();
        e.left();
        e.left();
        assert_eq!(e.cursor_pos(), (1, 0));
        assert!(e.up());
        assert_eq!(e.cursor_pos(), (0, 0));
    }

    #[test]
    fn wide_characters_count_two_cells() {
        let mut e = ed("日本語");
        assert_eq!(e.cursor_pos(), (0, 6));
        e.set_width(4);
        assert_eq!(e.height(), 2);
        e.text_start();
        e.right();
        assert_eq!(e.cursor_pos(), (0, 2));
    }

    #[test]
    fn take_empties() {
        let mut e = ed("x");
        assert_eq!(e.take(), "x");
        assert!(e.is_empty());
        assert_eq!(e.cursor(), 0);
    }
}
