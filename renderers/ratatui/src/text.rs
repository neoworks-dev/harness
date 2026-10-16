//! Display width, sanitizing, wrapping and truncating of styled text.
//!
//! Width is terminal cells as `unicode-width` counts them (CJK and emoji take two). Text is cut
//! at "clusters": a character plus what attaches to it (combining marks, variation selectors,
//! joined emoji, skin tones), so a cut never splits an emoji.

use std::borrow::Cow;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Terminal cells `s` takes.
pub(crate) fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

pub(crate) fn line_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| width(&s.content)).sum()
}

/// Makes untrusted text safe to print: drops control characters (escape sequences from tool
/// output must not reach the terminal) and bidi overrides, keeps newlines, expands tabs to
/// 4-column stops.
pub(crate) fn clean(s: &str) -> Cow<'_, str> {
    if !s.chars().any(|c| c == '\t' || unsafe_char(c)) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut col = 0;
    for c in s.chars() {
        match c {
            '\n' => {
                out.push('\n');
                col = 0;
            }
            '\t' => {
                let n = 4 - col % 4;
                out.extend(std::iter::repeat_n(' ', n));
                col += n;
            }
            c if unsafe_char(c) => {}
            c => {
                out.push(c);
                col += c.width().unwrap_or(0);
            }
        }
    }
    Cow::Owned(out)
}

fn unsafe_char(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t')
        || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Text that must stay on one line (labels, table cells): cleaned, whitespace collapsed.
pub(crate) fn one_line(s: &str) -> String {
    clean(s).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Splits a string into clusters.
pub(crate) struct Clusters<'a>(&'a str);

impl<'a> Clusters<'a> {
    pub(crate) fn new(s: &'a str) -> Self {
        Clusters(s)
    }
}

impl<'a> Iterator for Clusters<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let mut chars = self.0.char_indices();
        let (_, first) = chars.next()?;
        let mut end = first.len_utf8();
        let mut last = first;
        for (i, c) in chars {
            let attaches = last == '\u{200d}'
                || c.width() == Some(0)
                || matches!(c, '\u{1f3fb}'..='\u{1f3ff}');
            if !attaches {
                break;
            }
            end = i + c.len_utf8();
            last = c;
        }
        let (head, tail) = self.0.split_at(end);
        self.0 = tail;
        Some(head)
    }
}

/// Shortens `s` to at most `w` cells, ending in `…` when it had to cut.
pub(crate) fn truncate(s: &str, w: usize) -> String {
    if width(s) <= w {
        return s.to_string();
    }
    if w == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for cl in Clusters::new(s) {
        let cw = width(cl);
        if used + cw > w - 1 {
            break;
        }
        out.push_str(cl);
        used += cw;
    }
    out.push('…');
    out
}

/// [`truncate`] for styled text; the `…` takes the style of what it replaces.
pub(crate) fn truncate_spans(spans: Vec<Span<'static>>, w: usize) -> Vec<Span<'static>> {
    if line_width(&spans) <= w {
        return spans;
    }
    let mut out: Vec<Span<'static>> = Vec::new();
    if w == 0 {
        return out;
    }
    let mut used = 0;
    let mut style = Style::default();
    'outer: for span in &spans {
        style = span.style;
        for cl in Clusters::new(&span.content) {
            let cw = width(cl);
            if used + cw > w - 1 {
                break 'outer;
            }
            push_text(&mut out, cl, span.style);
            used += cw;
        }
    }
    push_text(&mut out, "…", style);
    out
}

/// A one-span line. (`Line::styled` would put the style on the line, where the tinting and
/// restyling of whole blocks, which work on spans, wouldn't see it.)
pub(crate) fn sline(text: impl Into<String>, style: Style) -> Line<'static> {
    Line::from(Span::styled(text.into(), style))
}

/// Appends text to a line, merging into the last span when the style is the same.
pub(crate) fn push_text(line: &mut Vec<Span<'static>>, text: &str, style: Style) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = line.last_mut()
        && last.style == style
    {
        last.content.to_mut().push_str(text);
        return;
    }
    line.push(Span::styled(text.to_string(), style));
}

/// Breaks each line of `s` after `w` cells, ignoring words: code keeps its spacing. Every line
/// is at most `w` cells wide. A trailing newline doesn't add an empty line.
pub(crate) fn hard_wrap(s: &str, w: usize) -> Vec<String> {
    let w = w.max(1);
    let cleaned = clean(s);
    let text = cleaned.strip_suffix('\n').unwrap_or(&cleaned);
    if text.is_empty() && s.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for raw in text.split('\n') {
        let mut cur = String::new();
        let mut used = 0;
        for cl in Clusters::new(raw) {
            let (cl, cw) = fit(cl, w);
            if used + cw > w {
                out.push(std::mem::take(&mut cur));
                used = 0;
            }
            cur.push_str(cl);
            used += cw;
        }
        out.push(cur);
    }
    out
}

/// A cluster wider than the whole line (a CJK character at width 1) becomes `…`.
fn fit(cluster: &str, w: usize) -> (&str, usize) {
    let cw = width(cluster);
    if cw > w { ("…", 1) } else { (cluster, cw) }
}

/// Wraps styled text to `w` cells. Words break at spaces; a word longer than a line is cut;
/// newlines are kept; spaces at the start of a line (and trailing ones) are dropped.
pub(crate) fn wrap(spans: &[Span<'static>], w: usize) -> Vec<Line<'static>> {
    let mut wr = Wrapper {
        w: w.max(1),
        lines: Vec::new(),
        cur: Vec::new(),
        cur_w: 0,
        gap: Vec::new(),
        gap_w: 0,
        word: Vec::new(),
    };
    for span in spans {
        for c in span.content.chars() {
            match c {
                '\n' => {
                    wr.flush_word();
                    wr.newline();
                }
                ' ' => {
                    wr.flush_word();
                    if wr.cur_w > 0 {
                        push_text(&mut wr.gap, " ", span.style);
                        wr.gap_w += 1;
                    }
                }
                c => {
                    let mut buf = [0u8; 4];
                    push_text(&mut wr.word, c.encode_utf8(&mut buf), span.style);
                }
            }
        }
    }
    wr.flush_word();
    if wr.cur_w > 0 || !wr.cur.is_empty() {
        wr.newline();
    }
    wr.lines
}

struct Wrapper {
    w: usize,
    lines: Vec<Line<'static>>,
    cur: Vec<Span<'static>>,
    cur_w: usize,
    /// Spaces seen since the last word, kept only if another word follows on the same line.
    gap: Vec<Span<'static>>,
    gap_w: usize,
    word: Vec<Span<'static>>,
}

impl Wrapper {
    fn newline(&mut self) {
        self.lines.push(Line::from(std::mem::take(&mut self.cur)));
        self.cur_w = 0;
        self.gap.clear();
        self.gap_w = 0;
    }

    fn flush_word(&mut self) {
        if self.word.is_empty() {
            return;
        }
        let word = std::mem::take(&mut self.word);
        let word_w = line_width(&word);
        if self.cur_w > 0 && self.cur_w + self.gap_w + word_w > self.w {
            self.newline();
        }
        if self.cur_w > 0 {
            for g in std::mem::take(&mut self.gap) {
                push_text(&mut self.cur, &g.content, g.style);
            }
            for p in &word {
                push_text(&mut self.cur, &p.content, p.style);
            }
            self.cur_w += self.gap_w + word_w;
            self.gap_w = 0;
        } else if word_w <= self.w {
            self.cur = word;
            self.cur_w = word_w;
        } else {
            self.break_word(&word);
        }
    }

    /// Cuts a word that is longer than a line. The last piece stays open for the next word.
    fn break_word(&mut self, word: &[Span<'static>]) {
        for part in word {
            for cl in Clusters::new(&part.content) {
                let (cl, cw) = fit(cl, self.w);
                if self.cur_w + cw > self.w {
                    self.newline();
                }
                push_text(&mut self.cur, cl, part.style);
                self.cur_w += cw;
            }
        }
    }
}

/// Cuts every line to `w` cells (a safety net: components fit on their own).
pub(crate) fn clip(lines: &mut [Line<'static>], w: usize) {
    for line in lines {
        if line_width(&line.spans) > w {
            let spans = std::mem::take(&mut line.spans);
            line.spans = clip_spans(spans, w);
        }
    }
}

fn clip_spans(spans: Vec<Span<'static>>, w: usize) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for span in &spans {
        for cl in Clusters::new(&span.content) {
            let cw = width(cl);
            if used + cw > w {
                return out;
            }
            push_text(&mut out, cl, span.style);
            used += cw;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    fn wrapped(s: &str, w: usize) -> Vec<String> {
        plain(&wrap(&[Span::raw(s.to_string())], w))
    }

    #[test]
    fn wraps_at_spaces() {
        assert_eq!(
            wrapped("the quick brown fox", 10),
            ["the quick", "brown fox"]
        );
        assert_eq!(wrapped("a  b", 10), ["a  b"]);
        assert_eq!(wrapped("  lead and trail  ", 20), ["lead and trail"]);
        assert_eq!(wrapped("", 10), Vec::<String>::new());
    }

    #[test]
    fn keeps_newlines() {
        assert_eq!(wrapped("a\n\nb\n", 10), ["a", "", "b"]);
    }

    #[test]
    fn breaks_long_words() {
        assert_eq!(wrapped("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrapped("x abcdefghij y", 4), ["x", "abcd", "efgh", "ij y"]);
    }

    #[test]
    fn counts_wide_characters_as_two_cells() {
        assert_eq!(wrapped("日本語のテキスト", 6), ["日本語", "のテキ", "スト"]);
        assert_eq!(wrapped("a日b", 2), ["a", "日", "b"]);
        // A wide character can't fit one cell: it becomes …
        assert_eq!(wrapped("日", 1), ["…"]);
    }

    #[test]
    fn never_splits_an_emoji() {
        let family = "👨\u{200d}👩\u{200d}👧";
        assert_eq!(width(family), 2);
        assert_eq!(
            wrapped(&format!("{family}{family}{family}"), 4),
            [format!("{family}{family}"), family.to_string()]
        );
        assert_eq!(
            Clusters::new("e\u{301}x👍🏽").collect::<Vec<_>>(),
            ["e\u{301}", "x", "👍🏽"]
        );
        assert_eq!(width("❤\u{fe0f}"), 2);
    }

    #[test]
    fn keeps_styles_across_a_wrap() {
        let bold = Style::new().add_modifier(ratatui::style::Modifier::BOLD);
        let lines = wrap(&[Span::raw("aa "), Span::styled("bbbb", bold)], 4);
        assert_eq!(plain(&lines), ["aa", "bbbb"]);
        assert_eq!(lines[1].spans[0].style, bold);
    }

    #[test]
    fn truncates_with_an_ellipsis() {
        assert_eq!(truncate("hello", 5), "hello");
        assert_eq!(truncate("hello", 4), "hel…");
        assert_eq!(truncate("日本語", 4), "日…");
        assert_eq!(truncate("hello", 0), "");
        assert_eq!(truncate("hello", 1), "…");
    }

    #[test]
    fn cleans_control_characters() {
        assert_eq!(clean("a\u{1b}[31mred\r\n"), "a[31mred\n");
        assert_eq!(clean("a\tb"), "a   b");
        assert_eq!(clean("ab\tc"), "ab  c");
        assert_eq!(clean("\u{202e}evil"), "evil");
        assert!(matches!(clean("plain"), Cow::Borrowed(_)));
        assert_eq!(one_line("a \n b\t c"), "a b c");
    }

    #[test]
    fn hard_wrap_keeps_spacing() {
        assert_eq!(hard_wrap("  ab cd\nx\n", 4), ["  ab", " cd", "x"]);
        assert_eq!(hard_wrap("", 4), Vec::<String>::new());
        assert_eq!(hard_wrap("a\n\nb", 4), ["a", "", "b"]);
    }
}
