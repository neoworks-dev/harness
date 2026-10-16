//! A small markdown reader for issue and comment bodies: enough to read them in a terminal.
//! Headings, lists, task lists, quotes, rules, fenced code and the inline `code`, **bold**,
//! *italic*, ~~strike~~, links and images; HTML comments (pull request templates are full of
//! them) and common HTML tags are dropped. Everything wraps to the width given.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

use crate::style::Styles;
use suite_github::strip_comments;

/// A styled character, the unit wrapping works on.
type Cell = (char, Style);

/// `text` as lines no wider than `width` cells, each starting with `indent` spaces.
pub fn render(text: &str, width: usize, indent: usize, st: &Styles) -> Vec<Line<'static>> {
    let pad = " ".repeat(indent);
    let width = width.saturating_sub(indent).max(1);
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut fenced = false;
    let text = strip_comments(text).replace("\r", "");
    let text = break_tags(&text);
    for raw in text.lines() {
        let raw = raw.trim_end();
        let trimmed = raw.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            for part in hard_wrap(raw, width.saturating_sub(2).max(1)) {
                out.push(Line::from(vec![
                    Span::raw(format!("{pad}  ")),
                    Span::styled(part, st.accent),
                ]));
            }
            continue;
        }
        if trimmed.is_empty() {
            if out.last().is_some_and(|l| !l.spans.is_empty()) {
                out.push(Line::default());
            }
            continue;
        }
        for line in block(raw, width, st) {
            let mut spans = vec![Span::raw(pad.clone())];
            spans.extend(line.spans);
            out.push(Line::from(spans));
        }
    }
    while out.last().is_some_and(|l| l.spans.is_empty()) {
        out.pop();
    }
    out
}

/// One source line: a heading, quote, list item, rule or paragraph line.
fn block(raw: &str, width: usize, st: &Styles) -> Vec<Line<'static>> {
    let lead = raw.len() - raw.trim_start().len();
    let trimmed = raw.trim_start();
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&level) && trimmed[level..].starts_with(' ') {
        let style = if level <= 2 {
            st.accent.add_modifier(Modifier::BOLD)
        } else {
            Style::new().add_modifier(Modifier::BOLD)
        };
        let cells = inline(trimmed[level..].trim(), style, st);
        return wrap(&cells, width, &[], &[]);
    }
    if is_rule(trimmed) {
        return vec![Line::from(Span::styled(
            "─".repeat(width.min(40)),
            st.border,
        ))];
    }
    if let Some(rest) = trimmed.strip_prefix('>') {
        // Quotes nest: each level adds a bar.
        let mut depth = 1;
        let mut rest = rest.trim_start();
        while let Some(r) = rest.strip_prefix('>') {
            depth += 1;
            rest = r.trim_start();
        }
        let bar = vec![Span::styled("│ ".repeat(depth), st.border)];
        let avail = width.saturating_sub(2 * depth).max(1);
        let cells = inline(rest, st.muted, st);
        return wrap(&cells, avail, &bar, &bar);
    }
    let nest = " ".repeat((lead / 2).min(6) * 2);
    if let Some((marker, rest)) = list_marker(trimmed) {
        let first = vec![Span::raw(format!("{nest}{marker}"))];
        let rest_pad = vec![Span::raw(" ".repeat(nest.len() + marker.chars().count()))];
        let avail = width
            .saturating_sub(nest.len() + marker.chars().count())
            .max(1);
        return wrap(&inline(rest, Style::new(), st), avail, &first, &rest_pad);
    }
    wrap(&inline(trimmed, Style::new(), st), width, &[], &[])
}

fn is_rule(s: &str) -> bool {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    s.len() >= 3
        && ["-", "*", "_"]
            .iter()
            .any(|m| s.chars().all(|c| m.starts_with(c)))
}

/// `- x`, `* x`, `+ x`, `1. x`, `- [ ] x`: the marker to draw and the text after it.
fn list_marker(s: &str) -> Option<(String, &str)> {
    let (marker, rest) =
        if let Some(rest) = ["- ", "* ", "+ "].iter().find_map(|m| s.strip_prefix(m)) {
            ("• ".to_string(), rest)
        } else {
            let digits = s.chars().take_while(char::is_ascii_digit).count();
            if digits == 0 || digits > 9 {
                return None;
            }
            let after = &s[digits..];
            let rest = after
                .strip_prefix(". ")
                .or_else(|| after.strip_prefix(") "))?;
            (format!("{}. ", &s[..digits]), rest)
        };
    let task = |box_: &str| rest.strip_prefix(box_).map(str::trim_start);
    if let Some(r) = task("[ ] ") {
        return Some(("☐ ".into(), r));
    }
    if let Some(r) = task("[x] ").or_else(|| task("[X] ")) {
        return Some(("☑ ".into(), r));
    }
    Some((marker, rest))
}

const TAGS: &[&str] = &[
    "a",
    "b",
    "blockquote",
    "br",
    "code",
    "details",
    "div",
    "em",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "img",
    "kbd",
    "li",
    "ol",
    "p",
    "picture",
    "pre",
    "source",
    "span",
    "strong",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "th",
    "thead",
    "tr",
    "ul",
];

/// Line-breaking tags become newlines, an image becomes `[image]`, other known HTML tags go.
fn break_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        let end = tail
            .find('>')
            .filter(|e| *e < 300 && !tail[..*e].contains('<'));
        let name = tail
            .trim_start_matches('/')
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_ascii_lowercase();
        match end {
            Some(e) if TAGS.contains(&name.as_str()) => {
                match name.as_str() {
                    "br" | "p" | "div" | "li" | "tr" | "details" | "summary" | "hr" | "pre" => {
                        out.push('\n')
                    }
                    "img" => out.push_str(&image_label(&tail[..e])),
                    _ => {}
                }
                rest = &tail[e + 1..];
            }
            _ => {
                out.push('<');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `[image: alt]` for the attributes of an `<img>` tag.
fn image_label(attrs: &str) -> String {
    let alt = attrs
        .split("alt=\"")
        .nth(1)
        .and_then(|a| a.split('"').next())
        .filter(|a| !a.is_empty());
    match alt {
        Some(a) => format!("[image: {a}]"),
        None => "[image]".into(),
    }
}

/// Inline markup as styled characters.
fn inline(s: &str, base: Style, st: &Styles) -> Vec<Cell> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    inline_into(&chars, base, st, &mut out);
    out
}

fn inline_into(c: &[char], base: Style, st: &Styles, out: &mut Vec<Cell>) {
    let mut i = 0;
    let push = |out: &mut Vec<Cell>, ch: char| out.push((ch, base));
    while i < c.len() {
        match c[i] {
            '\\' if c.get(i + 1).is_some_and(|n| n.is_ascii_punctuation()) => {
                push(out, c[i + 1]);
                i += 2;
            }
            '`' => {
                let ticks = c[i..].iter().take_while(|ch| **ch == '`').count();
                let close = find_seq(c, i + ticks, &vec!['`'; ticks]);
                match close {
                    Some(j) if j > i + ticks => {
                        let code: String = c[i + ticks..j].iter().collect();
                        let code = code.trim();
                        out.extend(code.chars().map(|ch| (ch, st.accent)));
                        i = j + ticks;
                    }
                    _ => {
                        (0..ticks).for_each(|_| push(out, '`'));
                        i += ticks;
                    }
                }
            }
            '!' if c.get(i + 1) == Some(&'[') => match link(c, i + 1) {
                Some((text, _, end)) => {
                    let label = if text.is_empty() {
                        "[image]".to_string()
                    } else {
                        format!("[image: {text}]")
                    };
                    out.extend(label.chars().map(|ch| (ch, st.muted)));
                    i = end;
                }
                None => {
                    push(out, '!');
                    i += 1;
                }
            },
            '[' => match link(c, i) {
                Some((text, _, end)) => {
                    let chars: Vec<char> = text.chars().collect();
                    let style = base.add_modifier(Modifier::UNDERLINED);
                    inline_into(&chars, style, st, out);
                    i = end;
                }
                None => {
                    push(out, '[');
                    i += 1;
                }
            },
            m @ ('*' | '_' | '~') => {
                let double = c.get(i + 1) == Some(&m);
                let len = if double { 2 } else { 1 };
                let opens = c.get(i + len).is_some_and(|n| !n.is_whitespace())
                    && (m != '_' || i == 0 || !c[i - 1].is_alphanumeric());
                let seq = vec![m; len];
                let close = opens
                    .then(|| find_seq(c, i + len, &seq))
                    .flatten()
                    .filter(|j| *j > i + len && !c[j - 1].is_whitespace());
                match close {
                    Some(j) if !(m == '~' && !double) => {
                        let modifier = match (m, double) {
                            ('~', _) => Modifier::CROSSED_OUT,
                            (_, true) => Modifier::BOLD,
                            _ => Modifier::ITALIC,
                        };
                        inline_into(&c[i + len..j], base.add_modifier(modifier), st, out);
                        i = j + len;
                    }
                    _ => {
                        push(out, m);
                        i += 1;
                    }
                }
            }
            ch => {
                push(out, ch);
                i += 1;
            }
        }
    }
}

/// Where `seq` next appears in `c` from `from` on.
fn find_seq(c: &[char], from: usize, seq: &[char]) -> Option<usize> {
    if seq.is_empty() || c.len() < seq.len() {
        return None;
    }
    (from..=c.len() - seq.len()).find(|&j| c[j..j + seq.len()] == *seq)
}

/// `[text](url)` starting at the `[` at `at`: the text, the url, and where it ends.
fn link(c: &[char], at: usize) -> Option<(String, String, usize)> {
    let close = c[at..].iter().position(|ch| *ch == ']')? + at;
    if c.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = c[close + 2..].iter().position(|ch| *ch == ')')? + close + 2;
    Some((
        c[at + 1..close].iter().collect(),
        c[close + 2..end].iter().collect(),
        end + 1,
    ))
}

fn cell_width(ch: char) -> usize {
    ch.width().unwrap_or(0)
}

/// `s` cut into pieces of at most `w` cells.
pub fn hard_wrap(s: &str, w: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let cw = cell_width(ch);
        if used + cw > w && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            used = 0;
        }
        cur.push(ch);
        used += cw;
    }
    out.push(cur);
    out
}

/// Greedy word wrap of styled characters to `width` cells (the prefixes included in the lines,
/// not in `width`: callers subtract them).
fn wrap(
    cells: &[Cell],
    width: usize,
    first: &[Span<'static>],
    rest: &[Span<'static>],
) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut lines: Vec<Vec<Cell>> = Vec::new();
    let mut cur: Vec<Cell> = Vec::new();
    let mut used = 0;
    let mut words = cells
        .split(|(ch, _)| ch.is_whitespace())
        .filter(|w| !w.is_empty());
    for word in &mut words {
        let ww: usize = word.iter().map(|(ch, _)| cell_width(*ch)).sum();
        if used > 0 && used + 1 + ww <= width {
            let gap = cur.last().map_or(Style::new(), |c| c.1);
            cur.push((' ', gap));
            cur.extend_from_slice(word);
            used += 1 + ww;
            continue;
        }
        if used > 0 {
            lines.push(std::mem::take(&mut cur));
            used = 0;
        }
        // A word longer than a line is broken wherever it hits the edge.
        for cell in word {
            let cw = cell_width(cell.0);
            if used + cw > width && !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
                used = 0;
            }
            cur.push(*cell);
            used += cw;
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
        .into_iter()
        .enumerate()
        .map(|(n, line)| {
            let mut spans: Vec<Span<'static>> = (if n == 0 { first } else { rest }).to_vec();
            let mut text = String::new();
            let mut style = None;
            for (ch, s) in line {
                if style.is_some_and(|prev| prev != s) {
                    spans.push(Span::styled(
                        std::mem::take(&mut text),
                        style.take().unwrap(),
                    ));
                }
                style = Some(s);
                text.push(ch);
            }
            if let Some(s) = style {
                spans.push(Span::styled(text, s));
            }
            Line::from(spans)
        })
        .collect()
}

/// Plain text (no markup) wrapped to `width` in one style.
pub fn paragraph(text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    let cells: Vec<Cell> = text.chars().map(|c| (c, style)).collect();
    wrap(&cells, width, &[], &[])
}
