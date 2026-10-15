//! The text and caret of the single-line input, without any GPUI. Platform input methods count
//! in UTF-16 offsets, everything else here in bytes; the `*_utf16` methods convert.

use std::ops::Range;

#[derive(Debug, Default)]
pub struct Field {
    text: String,
    /// Caret position, a byte offset on a char boundary.
    cursor: usize,
    /// Text an input method is still composing.
    marked: Option<Range<usize>>,
}

impl Field {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    /// Empties the field and returns what was in it.
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        self.marked = None;
        std::mem::take(&mut self.text)
    }

    /// Types `text` at the caret (replacing what is being composed, if anything).
    pub fn insert(&mut self, text: &str) {
        self.replace(
            self.marked.clone().unwrap_or(self.cursor..self.cursor),
            text,
        );
        self.marked = None;
    }

    pub fn backspace(&mut self) {
        if let Some(before) = self.text[..self.cursor].chars().next_back() {
            let start = self.cursor - before.len_utf8();
            self.replace(start..self.cursor, "");
        }
    }

    pub fn delete(&mut self) {
        if let Some(after) = self.text[self.cursor..].chars().next() {
            self.replace(self.cursor..self.cursor + after.len_utf8(), "");
        }
    }

    pub fn left(&mut self) {
        if let Some(before) = self.text[..self.cursor].chars().next_back() {
            self.cursor -= before.len_utf8();
        }
    }

    pub fn right(&mut self) {
        if let Some(after) = self.text[self.cursor..].chars().next() {
            self.cursor += after.len_utf8();
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.len();
    }

    /// An input method replacing `range` (UTF-16; `None` means the composition, else the caret).
    pub fn replace_utf16(&mut self, range: Option<Range<usize>>, text: &str) {
        let range = self.resolve(range);
        self.replace(range, text);
        self.marked = None;
    }

    /// An input method replacing `range` with text it is still composing. `selected` is the
    /// caret inside the new text (UTF-16), by default its end.
    pub fn replace_and_mark_utf16(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
    ) {
        let range = self.resolve(range);
        let start = range.start;
        self.replace(range, text);
        let text = single_line(text);
        self.marked = Some(start..start + text.len());
        if let Some(selected) = selected {
            self.cursor = start + byte_offset(&text, selected.end);
        }
    }

    pub fn unmark(&mut self) {
        self.marked = None;
    }

    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        utf16_offset(&self.text, range.start)..utf16_offset(&self.text, range.end)
    }

    /// The text of a UTF-16 range, and the range actually covered once clamped to the text.
    pub fn slice_utf16(&self, range: Range<usize>) -> (String, Range<usize>) {
        let start = byte_offset(&self.text, range.start);
        let end = byte_offset(&self.text, range.end).max(start);
        (
            self.text[start..end].to_string(),
            utf16_offset(&self.text, start)..utf16_offset(&self.text, end),
        )
    }

    fn resolve(&self, range: Option<Range<usize>>) -> Range<usize> {
        match range {
            Some(range) => {
                let start = byte_offset(&self.text, range.start);
                start..byte_offset(&self.text, range.end).max(start)
            }
            None => self.marked.clone().unwrap_or(self.cursor..self.cursor),
        }
    }

    /// Replaces a byte range and puts the caret after the new text.
    fn replace(&mut self, range: Range<usize>, text: &str) {
        let text = single_line(text);
        self.text.replace_range(range.clone(), &text);
        self.cursor = range.start + text.len();
    }
}

/// The field has one line: line breaks (pasted text) become spaces.
fn single_line(text: &str) -> String {
    text.replace("\r\n", " ").replace(['\n', '\r'], " ")
}

/// Byte offset of a UTF-16 offset, clamped to the text and to a char boundary.
pub fn byte_offset(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (index, ch) in text.char_indices() {
        if units >= utf16 {
            return index;
        }
        units += ch.len_utf16();
    }
    text.len()
}

pub fn utf16_offset(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(text: &str) -> Field {
        let mut field = Field::default();
        field.insert(text);
        field
    }

    #[test]
    fn typing_and_editing_at_the_caret() {
        let mut f = field("héllo");
        assert_eq!(f.cursor(), "héllo".len());
        f.left();
        f.left();
        f.insert("X");
        assert_eq!(f.text(), "hélXlo");
        f.backspace();
        f.backspace();
        assert_eq!(f.text(), "hélo");
        f.home();
        f.delete();
        assert_eq!(f.text(), "élo");
        f.end();
        f.right();
        f.backspace();
        assert_eq!(f.text(), "él");
    }

    #[test]
    fn edges_do_nothing() {
        let mut f = Field::default();
        f.backspace();
        f.delete();
        f.left();
        f.right();
        assert_eq!((f.text(), f.cursor()), ("", 0));
    }

    #[test]
    fn multibyte_characters_move_as_one() {
        let mut f = field("a😀b");
        f.left();
        f.left();
        assert_eq!(f.cursor(), 1);
        f.delete();
        assert_eq!(f.text(), "ab");
    }

    #[test]
    fn take_empties_the_field() {
        let mut f = field("send me");
        assert_eq!(f.take(), "send me");
        assert_eq!((f.text(), f.cursor()), ("", 0));
    }

    #[test]
    fn line_breaks_become_spaces() {
        assert_eq!(field("a\nb\r\nc").text(), "a b c");
    }

    #[test]
    fn utf16_offsets_count_surrogate_pairs() {
        let f = field("a😀b");
        assert_eq!(utf16_offset(f.text(), 5), 3);
        assert_eq!(byte_offset(f.text(), 3), 5);
        assert_eq!(byte_offset(f.text(), 99), f.text().len());
        let (slice, covered) = f.slice_utf16(1..3);
        assert_eq!((slice.as_str(), covered), ("😀", 1..3));
    }

    #[test]
    fn composing_replaces_the_previous_composition() {
        let mut f = field("ab");
        f.replace_and_mark_utf16(None, "ka", None);
        assert_eq!((f.text(), f.marked()), ("abka", Some(2..4)));
        f.replace_and_mark_utf16(None, "か", None);
        assert_eq!((f.text(), f.marked()), ("abか", Some(2..5)));
        f.replace_utf16(None, "か");
        assert_eq!((f.text(), f.marked(), f.cursor()), ("abか", None, 5));
    }

    #[test]
    fn replacing_an_explicit_range() {
        let mut f = field("hello");
        f.replace_utf16(Some(1..3), "EY");
        assert_eq!(f.text(), "hEYlo");
        assert_eq!(f.cursor(), 3);
        assert_eq!(f.range_to_utf16(&(1..3)), 1..3);
    }

    #[test]
    fn composition_caret_inside_the_new_text() {
        let mut f = Field::default();
        f.replace_and_mark_utf16(None, "abc", Some(1..1));
        assert_eq!(f.cursor(), 1);
    }
}
