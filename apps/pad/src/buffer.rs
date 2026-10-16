//! The text of one open file: lines, a cursor with an optional selection, editing and undo.
//! No GPUI in here. Columns count characters (not bytes, not UTF-16), lines are 0-based; the
//! callers that talk 1-based (tools, diagnostics) convert.

use std::ops::Range;

/// A position in the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    pub fn new(line: usize, col: usize) -> Pos {
        Pos { line, col }
    }
}

/// How many snapshots undo keeps, and the most text they may hold together.
const UNDO_DEPTH: usize = 200;
const UNDO_BYTES: usize = 64 * 1024 * 1024;
/// A tab is drawn as this many spaces.
pub const TAB_WIDTH: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Typing,
    Erasing,
    Other,
}

#[derive(Debug, Clone)]
struct Snapshot {
    lines: Vec<String>,
    cursor: Pos,
    anchor: Option<Pos>,
    revision: u64,
}

impl Snapshot {
    fn bytes(&self) -> usize {
        self.lines.iter().map(|l| l.len() + 1).sum()
    }
}

#[derive(Debug, Clone)]
pub struct Buffer {
    lines: Vec<String>,
    /// The file used CRLF line ends; saving writes them back.
    crlf: bool,
    /// The file ended with a line break; saving keeps it that way.
    final_newline: bool,
    cursor: Pos,
    anchor: Option<Pos>,
    /// The column vertical movement tries to return to.
    goal_col: Option<usize>,
    /// Text an input method is composing (one line).
    marked: Option<(Pos, Pos)>,
    revision: u64,
    next_revision: u64,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    undo_bytes: usize,
    /// Where the next typed character or erase continues the last edit (so undo takes a word,
    /// not a letter).
    coalesce: Option<(Kind, Pos)>,
}

impl Default for Buffer {
    fn default() -> Buffer {
        Buffer::from_text("")
    }
}

impl Buffer {
    pub fn from_text(text: &str) -> Buffer {
        let (lines, crlf, final_newline) = split_text(text);
        Buffer {
            lines,
            crlf,
            final_newline,
            cursor: Pos::default(),
            anchor: None,
            goal_col: None,
            marked: None,
            revision: 0,
            next_revision: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            undo_bytes: 0,
            coalesce: None,
        }
    }

    /// Replaces the text with what is on disk now: the cursor stays where it was (clamped) and
    /// undo starts over, since its snapshots describe a text that is gone.
    pub fn reload(&mut self, text: &str) {
        let (lines, crlf, final_newline) = split_text(text);
        self.lines = lines;
        self.crlf = crlf;
        self.final_newline = final_newline;
        self.cursor = self.clamp(self.cursor);
        self.anchor = self.anchor.map(|a| self.clamp(a));
        self.marked = None;
        self.undo.clear();
        self.redo.clear();
        self.undo_bytes = 0;
        self.coalesce = None;
        self.bump();
    }

    /// The text as the file holds it (line ends and final newline as they were).
    pub fn file_text(&self) -> String {
        let nl = if self.crlf { "\r\n" } else { "\n" };
        let mut out = self.lines.join(nl);
        if self.final_newline {
            out.push_str(nl);
        }
        out
    }

    /// The text with `\n` line ends and no final one.
    #[cfg(test)]
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line(&self, index: usize) -> &str {
        self.lines.get(index).map_or("", String::as_str)
    }

    pub fn line_len(&self, index: usize) -> usize {
        self.line(index).chars().count()
    }

    /// Changes with every edit and is restored by undo, so "revision equals the saved one" means
    /// the text is what was saved.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn cursor(&self) -> Pos {
        self.cursor
    }

    pub fn marked(&self) -> Option<(Pos, Pos)> {
        self.marked
    }

    /// The selection as an ordered pair, if anything is selected.
    pub fn selection(&self) -> Option<(Pos, Pos)> {
        let anchor = self.anchor?;
        (anchor != self.cursor).then(|| (anchor.min(self.cursor), anchor.max(self.cursor)))
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|(a, b)| self.text_between(a, b))
    }

    pub fn text_between(&self, from: Pos, to: Pos) -> String {
        let (from, to) = (self.clamp(from), self.clamp(to));
        if from.line == to.line {
            return slice_chars(self.line(from.line), from.col..to.col).to_string();
        }
        let mut out = String::new();
        out.push_str(slice_chars(
            self.line(from.line),
            from.col..self.line_len(from.line),
        ));
        for line in from.line + 1..to.line {
            out.push('\n');
            out.push_str(self.line(line));
        }
        out.push('\n');
        out.push_str(slice_chars(self.line(to.line), 0..to.col));
        out
    }

    pub fn clamp(&self, pos: Pos) -> Pos {
        let line = pos.line.min(self.lines.len() - 1);
        Pos {
            line,
            col: pos.col.min(self.line_len(line)),
        }
    }

    // --- cursor ---

    /// Puts the cursor at `pos`; with `extend` the selection grows from where it was.
    pub fn set_cursor(&mut self, pos: Pos, extend: bool) {
        self.place(pos, extend);
        self.goal_col = None;
    }

    fn place(&mut self, pos: Pos, extend: bool) {
        let pos = self.clamp(pos);
        if extend {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = pos;
        self.coalesce = None;
    }

    pub fn move_left(&mut self, extend: bool) {
        if !extend && let Some((start, _)) = self.selection() {
            return self.set_cursor(start, false);
        }
        let Pos { line, col } = self.cursor;
        let target = if col > 0 {
            Pos::new(line, col - 1)
        } else if line > 0 {
            Pos::new(line - 1, self.line_len(line - 1))
        } else {
            self.cursor
        };
        self.set_cursor(target, extend);
    }

    pub fn move_right(&mut self, extend: bool) {
        if !extend && let Some((_, end)) = self.selection() {
            return self.set_cursor(end, false);
        }
        let Pos { line, col } = self.cursor;
        let target = if col < self.line_len(line) {
            Pos::new(line, col + 1)
        } else if line + 1 < self.lines.len() {
            Pos::new(line + 1, 0)
        } else {
            self.cursor
        };
        self.set_cursor(target, extend);
    }

    /// Moves `delta` lines, trying to keep the column it started from.
    pub fn move_lines(&mut self, delta: isize, extend: bool) {
        let goal = self.goal_col.unwrap_or(self.cursor.col);
        let wanted = self.cursor.line as isize + delta;
        let last = self.lines.len() - 1;
        // Past either end: to the start or end of the text.
        let target = if wanted < 0 {
            Pos::new(0, 0)
        } else if wanted as usize > last {
            Pos::new(last, self.line_len(last))
        } else {
            Pos::new(wanted as usize, goal)
        };
        self.place(target, extend);
        self.goal_col = Some(goal);
    }

    pub fn move_home(&mut self, extend: bool) {
        // First press: the first non-blank character; again: the very start.
        let line = self.cursor.line;
        let indent = self
            .line(line)
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();
        let col = if self.cursor.col == indent || indent == self.line_len(line) {
            0
        } else {
            indent
        };
        self.set_cursor(Pos::new(line, col), extend);
    }

    pub fn move_end(&mut self, extend: bool) {
        let line = self.cursor.line;
        self.set_cursor(Pos::new(line, self.line_len(line)), extend);
    }

    pub fn move_doc_start(&mut self, extend: bool) {
        self.set_cursor(Pos::new(0, 0), extend);
    }

    pub fn move_doc_end(&mut self, extend: bool) {
        let line = self.lines.len() - 1;
        self.set_cursor(Pos::new(line, self.line_len(line)), extend);
    }

    pub fn select_all(&mut self) {
        self.set_cursor(Pos::new(0, 0), false);
        self.move_doc_end(true);
    }

    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }

    // --- editing ---

    /// Types `text` over the selection (or the composition). Line breaks start new lines.
    pub fn insert(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if text.is_empty() && self.selection().is_none() && self.marked.is_none() {
            return;
        }
        let range = self.target_range();
        let single = !text.contains('\n') && text.chars().count() == 1 && range.0 == range.1;
        let kind = if single { Kind::Typing } else { Kind::Other };
        self.edit(kind, range.0, range.1, &text);
    }

    /// An input method puts `text` where the composition (or `range`) was and keeps composing it.
    pub fn compose(&mut self, range: Option<(Pos, Pos)>, text: &str) {
        let text = text.replace(['\n', '\r'], " ");
        let (from, to) = range.map_or_else(|| self.target_range(), |(a, b)| (a.min(b), a.max(b)));
        self.edit(Kind::Other, from, to, &text);
        let end = Pos::new(from.line, from.col + text.chars().count());
        self.marked = (!text.is_empty()).then_some((from, end));
    }

    pub fn unmark(&mut self) {
        self.marked = None;
    }

    /// Enter: a new line indented like this one.
    pub fn newline(&mut self) {
        let (from, to) = self.target_range();
        let indent: String = self
            .line(from.line)
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .take(from.col)
            .collect();
        self.edit(Kind::Other, from, to, &format!("\n{indent}"));
    }

    /// Tab: indents the way the file does (a tab when it uses them, else two spaces).
    pub fn indent(&mut self) {
        let unit = if self.lines.iter().any(|l| l.starts_with('\t')) {
            "\t"
        } else {
            "  "
        };
        self.insert(unit);
    }

    pub fn backspace(&mut self) {
        if let Some((a, b)) = self.selection() {
            return self.edit(Kind::Other, a, b, "");
        }
        let Pos { line, col } = self.cursor;
        let from = if col > 0 {
            Pos::new(line, col - 1)
        } else if line > 0 {
            Pos::new(line - 1, self.line_len(line - 1))
        } else {
            return;
        };
        self.edit(Kind::Erasing, from, self.cursor, "");
    }

    pub fn delete(&mut self) {
        if let Some((a, b)) = self.selection() {
            return self.edit(Kind::Other, a, b, "");
        }
        let Pos { line, col } = self.cursor;
        let to = if col < self.line_len(line) {
            Pos::new(line, col + 1)
        } else if line + 1 < self.lines.len() {
            Pos::new(line + 1, 0)
        } else {
            return;
        };
        self.edit(Kind::Erasing, self.cursor, to, "");
    }

    /// Removes the selection and returns it.
    pub fn cut(&mut self) -> Option<String> {
        let (a, b) = self.selection()?;
        let text = self.text_between(a, b);
        self.edit(Kind::Other, a, b, "");
        Some(text)
    }

    /// Replaces `from..to` with `text` and puts the cursor after it. The one place text changes.
    pub fn replace(&mut self, from: Pos, to: Pos, text: &str) {
        let (from, to) = (self.clamp(from), self.clamp(to));
        self.edit(Kind::Other, from.min(to), from.max(to), text);
    }

    fn target_range(&self) -> (Pos, Pos) {
        if let Some((a, b)) = self.marked {
            return (a, b);
        }
        self.selection().unwrap_or((self.cursor, self.cursor))
    }

    fn edit(&mut self, kind: Kind, from: Pos, to: Pos, text: &str) {
        let continues = kind != Kind::Other && self.coalesce == Some((kind, self.cursor));
        if !continues {
            self.push_undo();
        }
        self.redo.clear();

        let head = take_chars(self.line(from.line), from.col);
        let tail = skip_chars(self.line(to.line), to.col);
        let mut pieces = text.split('\n');
        let first = pieces.next().unwrap_or("");
        let mut replacement: Vec<String> = vec![format!("{head}{first}")];
        replacement.extend(pieces.map(String::from));
        let last = replacement.last_mut().expect("at least one line");
        let col = last.chars().count();
        last.push_str(tail);
        let line = from.line + replacement.len() - 1;
        self.lines.splice(from.line..=to.line, replacement);

        self.marked = None;
        self.cursor = Pos::new(line, col);
        self.anchor = None;
        self.goal_col = None;
        self.coalesce = (kind != Kind::Other).then_some((kind, self.cursor));
        self.bump();
    }

    // --- undo ---

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            lines: self.lines.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
            revision: self.revision,
        }
    }

    fn push_undo(&mut self) {
        let snap = self.snapshot();
        self.undo_bytes += snap.bytes();
        self.undo.push(snap);
        while self.undo.len() > UNDO_DEPTH || (self.undo_bytes > UNDO_BYTES && self.undo.len() > 1)
        {
            let dropped = self.undo.remove(0);
            self.undo_bytes = self.undo_bytes.saturating_sub(dropped.bytes());
        }
    }

    fn restore(&mut self, snap: Snapshot) {
        self.lines = snap.lines;
        self.cursor = snap.cursor;
        self.anchor = snap.anchor;
        self.revision = snap.revision;
        self.marked = None;
        self.goal_col = None;
        self.coalesce = None;
    }

    pub fn undo(&mut self) -> bool {
        let Some(snap) = self.undo.pop() else {
            return false;
        };
        self.undo_bytes = self.undo_bytes.saturating_sub(snap.bytes());
        self.redo.push(self.snapshot());
        self.restore(snap);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snap) = self.redo.pop() else {
            return false;
        };
        let current = self.snapshot();
        self.undo_bytes += current.bytes();
        self.undo.push(current);
        self.restore(snap);
        true
    }

    fn bump(&mut self) {
        self.next_revision += 1;
        self.revision = self.next_revision;
    }

    // --- UTF-16 offsets, for the platform's input methods (over `text()`) ---

    pub fn utf16_of(&self, pos: Pos) -> usize {
        let pos = self.clamp(pos);
        let before: usize = self.lines[..pos.line]
            .iter()
            .map(|l| l.encode_utf16().count() + 1)
            .sum();
        before
            + self
                .line(pos.line)
                .chars()
                .take(pos.col)
                .map(char::len_utf16)
                .sum::<usize>()
    }

    pub fn pos_of_utf16(&self, offset: usize) -> Pos {
        let mut rest = offset;
        for (line, text) in self.lines.iter().enumerate() {
            let units = text.encode_utf16().count();
            if rest <= units {
                let mut col = 0;
                let mut seen = 0;
                for ch in text.chars() {
                    if seen >= rest {
                        break;
                    }
                    seen += ch.len_utf16();
                    col += 1;
                }
                return Pos::new(line, col);
            }
            rest -= units + 1;
        }
        let line = self.lines.len() - 1;
        Pos::new(line, self.line_len(line))
    }

    #[cfg(test)]
    pub fn utf16_len(&self) -> usize {
        self.utf16_of(Pos::new(usize::MAX, usize::MAX))
    }
}

fn split_text(text: &str) -> (Vec<String>, bool, bool) {
    let crlf = text.contains("\r\n");
    let normalized = if crlf {
        text.replace("\r\n", "\n")
    } else {
        text.to_string()
    };
    let final_newline = normalized.ends_with('\n');
    let body = normalized.strip_suffix('\n').unwrap_or(&normalized);
    let mut lines: Vec<String> = body.split('\n').map(String::from).collect();
    if lines.is_empty() {
        lines.push(String::new());
    }
    (lines, crlf, final_newline)
}

fn byte_at(text: &str, col: usize) -> usize {
    text.char_indices().nth(col).map_or(text.len(), |(i, _)| i)
}

fn slice_chars(text: &str, range: Range<usize>) -> &str {
    let start = byte_at(text, range.start);
    let end = byte_at(text, range.end).max(start);
    &text[start..end]
}

fn take_chars(text: &str, count: usize) -> &str {
    &text[..byte_at(text, count)]
}

fn skip_chars(text: &str, count: usize) -> &str {
    &text[byte_at(text, count)..]
}

// --- how a line is drawn: tabs become spaces ---

/// The line as drawn.
pub fn display_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for ch in line.chars() {
        if ch == '\t' {
            out.push_str(&" ".repeat(TAB_WIDTH));
        } else {
            out.push(ch);
        }
    }
    out
}

/// The drawn column (in characters) of character column `col`.
#[cfg(test)]
pub fn visual_col(line: &str, col: usize) -> usize {
    line.chars()
        .take(col)
        .map(|c| if c == '\t' { TAB_WIDTH } else { 1 })
        .sum()
}

/// The byte offset in [`display_line`]'s output of character column `col`.
pub fn display_byte(line: &str, col: usize) -> usize {
    line.chars()
        .take(col)
        .map(|c| if c == '\t' { TAB_WIDTH } else { c.len_utf8() })
        .sum()
}

/// The character column drawn at drawn column `vcol` (the nearer edge, for clicks).
pub fn col_at_visual(line: &str, vcol: usize) -> usize {
    let mut at = 0;
    for (col, ch) in line.chars().enumerate() {
        let width = if ch == '\t' { TAB_WIDTH } else { 1 };
        if vcol < at + width.div_ceil(2) {
            return col;
        }
        at += width;
    }
    line.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(text: &str) -> Buffer {
        Buffer::from_text(text)
    }

    fn at(line: usize, col: usize) -> Pos {
        Pos::new(line, col)
    }

    #[test]
    fn splitting_and_saving_keep_the_file_as_it_was() {
        for text in [
            "",
            "a",
            "a\n",
            "a\nb",
            "a\nb\n",
            "a\r\nb\r\n",
            "\n",
            "a\n\n",
        ] {
            assert_eq!(buf(text).file_text(), text, "{text:?}");
        }
        assert_eq!(buf("a\nb\n").line_count(), 2);
        assert_eq!(buf("").line_count(), 1);
        assert_eq!(buf("a\r\nb").text(), "a\nb");
    }

    #[test]
    fn typing_and_erasing() {
        let mut b = buf("héllo");
        b.move_doc_end(false);
        b.insert("!");
        assert_eq!(b.text(), "héllo!");
        b.move_left(false);
        b.move_left(false);
        b.backspace();
        assert_eq!(b.text(), "hélo!");
        b.delete();
        assert_eq!(b.text(), "hél!");
        assert_eq!(b.cursor(), at(0, 3));
    }

    #[test]
    fn lines_split_and_join() {
        let mut b = buf("ab");
        b.set_cursor(at(0, 1), false);
        b.insert("1\n2\n3");
        assert_eq!(b.text(), "a1\n2\n3b");
        assert_eq!(b.cursor(), at(2, 1));
        b.set_cursor(at(1, 0), false);
        b.backspace();
        assert_eq!(b.text(), "a12\n3b");
        b.set_cursor(at(0, 3), false);
        b.delete();
        assert_eq!(b.text(), "a123b");
        // Edges do nothing.
        b.move_doc_start(false);
        b.backspace();
        b.move_doc_end(false);
        b.delete();
        assert_eq!(b.text(), "a123b");
    }

    #[test]
    fn enter_keeps_the_indent_up_to_the_cursor() {
        let mut b = buf("  \tfoo");
        b.move_doc_end(false);
        b.newline();
        assert_eq!(b.line(1), "  \t");
        assert_eq!(b.cursor(), at(1, 3));
        let mut b = buf("    x");
        b.set_cursor(at(0, 2), false);
        b.newline();
        assert_eq!((b.line(0), b.line(1)), ("  ", "    x"));
        assert_eq!(b.cursor(), at(1, 2));
    }

    #[test]
    fn indent_follows_the_file() {
        let mut b = buf("fn a() {\n}");
        b.indent();
        assert_eq!(b.line(0), "  fn a() {");
        let mut b = buf("func a() {\n\tx()\n}");
        b.indent();
        assert_eq!(b.line(0), "\tfunc a() {");
    }

    #[test]
    fn selection_moves_and_replaces() {
        let mut b = buf("one two\nthree");
        b.set_cursor(at(0, 4), false);
        b.move_right(true);
        b.move_right(true);
        b.move_right(true);
        assert_eq!(b.selected_text().as_deref(), Some("two"));
        // Arrow without shift collapses to the edge.
        b.move_left(false);
        assert_eq!((b.cursor(), b.selection()), (at(0, 4), None));
        b.set_cursor(at(0, 4), false);
        b.move_lines(1, true);
        assert_eq!(b.selected_text().as_deref(), Some("two\nthre"));
        b.insert("X");
        assert_eq!(b.text(), "one Xe");
        b.select_all();
        assert_eq!(b.cut().as_deref(), Some("one Xe"));
        assert_eq!(b.text(), "");
    }

    #[test]
    fn vertical_moves_remember_the_column() {
        let mut b = buf("abcdef\nab\nabcdef");
        b.set_cursor(at(0, 5), false);
        b.move_lines(1, false);
        assert_eq!(b.cursor(), at(1, 2));
        b.move_lines(1, false);
        assert_eq!(b.cursor(), at(2, 5));
        b.move_lines(5, false);
        assert_eq!(b.cursor(), at(2, 6), "past the end goes to the end");
        b.move_lines(-9, false);
        assert_eq!(b.cursor(), at(0, 0), "past the start goes to the start");
    }

    #[test]
    fn home_goes_to_the_indent_then_the_start() {
        let mut b = buf("    code");
        b.move_end(false);
        b.move_home(false);
        assert_eq!(b.cursor().col, 4);
        b.move_home(false);
        assert_eq!(b.cursor().col, 0);
        b.move_home(false);
        assert_eq!(b.cursor().col, 4);
    }

    #[test]
    fn undo_and_redo_restore_text_and_revision() {
        let mut b = buf("x");
        let saved = b.revision();
        b.move_doc_end(false);
        for ch in ["a", "b", "c"] {
            b.insert(ch);
        }
        assert_ne!(b.revision(), saved);
        b.newline();
        b.insert("d");
        assert_eq!(b.text(), "xabc\nd");
        assert!(b.undo());
        assert_eq!(b.text(), "xabc\n", "typing after the newline is one step");
        assert!(b.undo());
        assert_eq!(b.text(), "xabc");
        assert!(b.undo());
        assert_eq!(b.text(), "x", "abc is one step");
        assert_eq!(b.revision(), saved, "back to the saved text");
        assert!(!b.undo());
        assert!(b.redo());
        assert_eq!(b.text(), "xabc");
        b.insert("!");
        assert!(!b.redo(), "an edit ends the redo history");
    }

    #[test]
    fn reload_keeps_the_cursor_and_forgets_undo() {
        let mut b = buf("one\ntwo\nthree");
        b.set_cursor(at(2, 4), false);
        b.insert("x");
        b.reload("one\ntwo");
        assert_eq!(b.cursor(), at(1, 3));
        assert!(!b.undo());
        assert_eq!(b.text(), "one\ntwo");
    }

    #[test]
    fn composition_replaces_the_previous_composition() {
        let mut b = buf("ab");
        b.move_doc_end(false);
        b.compose(None, "k");
        b.compose(None, "ka");
        assert_eq!(
            (b.text(), b.marked()),
            ("abka".into(), Some((at(0, 2), at(0, 4))))
        );
        b.compose(None, "か");
        b.unmark();
        assert_eq!(b.text(), "abか");
    }

    #[test]
    fn utf16_offsets_round_trip() {
        let b = buf("a😀b\nxy");
        assert_eq!(b.utf16_of(at(0, 2)), 3);
        assert_eq!(b.utf16_of(at(1, 1)), 5 + 1);
        assert_eq!(b.pos_of_utf16(3), at(0, 2));
        assert_eq!(b.pos_of_utf16(6), at(1, 1));
        assert_eq!(b.pos_of_utf16(999), at(1, 2));
        assert_eq!(b.utf16_len(), 7);
    }

    #[test]
    fn tabs_are_drawn_as_spaces() {
        assert_eq!(display_line("\tab"), "    ab");
        assert_eq!(visual_col("\tab", 2), 5);
        assert_eq!(display_byte("é\tb", 2), 2 + 4);
        assert_eq!(col_at_visual("\tab", 0), 0);
        assert_eq!(col_at_visual("\tab", 3), 1);
        assert_eq!(col_at_visual("\tab", 4), 1);
        assert_eq!(col_at_visual("\tab", 99), 3);
    }
}
