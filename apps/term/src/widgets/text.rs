//! Small text helpers shared by the panels: cleaning, cell widths, wrapping, cutting.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Display width in terminal cells.
pub(crate) fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// One line of untrusted text made safe to draw: control characters (escape sequences) and
/// bidi overrides are dropped, tabs expand to 4-column stops.
pub(crate) fn clean_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut col = 0;
    for c in s.chars() {
        match c {
            '\t' => {
                let n = 4 - col % 4;
                out.push_str(&" ".repeat(n));
                col += n;
            }
            c if c.is_control()
                || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}') =>
                {}
            c => {
                out.push(c);
                col += c.width().unwrap_or(0);
            }
        }
    }
    out
}

/// Cuts `s` to at most `max` cells, ending in `…` when something was dropped.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > max - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Like [`truncate`] but keeps the end (file names) and puts the `…` first.
pub(crate) fn truncate_start(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut rev = Vec::new();
    let mut used = 0;
    for c in s.chars().rev() {
        let w = c.width().unwrap_or(0);
        if used + w > max - 1 {
            break;
        }
        rev.push(c);
        used += w;
    }
    let mut out = String::from("…");
    out.extend(rev.into_iter().rev());
    out
}

/// Cuts `s` to `max` cells without a marker.
fn cut(s: &str, max: usize) -> (&str, &str) {
    let mut used = 0;
    for (i, c) in s.char_indices() {
        let w = c.width().unwrap_or(0);
        if used + w > max {
            // Always make progress, even when a single character is wider than `max`.
            if i == 0 {
                let n = c.len_utf8();
                return (&s[..n], &s[n..]);
            }
            return (&s[..i], &s[i..]);
        }
        used += w;
    }
    (s, "")
}

/// Word-wraps `text` (any whitespace, newlines included, is a break opportunity) to `width`
/// cells; words longer than a line are cut. Empty text gives no lines.
pub(crate) fn wrap(text: &str, width_cells: usize) -> Vec<String> {
    let max = width_cells.max(1);
    let clean = clean_line(&text.replace(['\n', '\r', '\t'], " "));
    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0;
    for word in clean.split_whitespace() {
        let mut word = word;
        let mut w = width(word);
        if w > max {
            if cur_w > 0 {
                lines.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
            while width(word) > max {
                let (head, tail) = cut(word, max);
                lines.push(head.to_string());
                word = tail;
            }
            w = width(word);
            if word.is_empty() {
                continue;
            }
        }
        if cur_w == 0 {
            cur.push_str(word);
            cur_w = w;
        } else if cur_w + 1 + w <= max {
            cur.push(' ');
            cur.push_str(word);
            cur_w += 1 + w;
        } else {
            lines.push(std::mem::take(&mut cur));
            cur.push_str(word);
            cur_w = w;
        }
    }
    if cur_w > 0 {
        lines.push(cur);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_untrusted_text() {
        assert_eq!(clean_line("a\tb"), "a   b");
        assert_eq!(clean_line("\x1b[31mred\x1b[0m"), "[31mred[0m");
        assert_eq!(clean_line("x\u{202e}y"), "xy");
    }
}
