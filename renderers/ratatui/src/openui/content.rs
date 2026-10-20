//! Content components: text, code, tables, lists, key/values, tags, metrics, progress, images,
//! links.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use suite_protocol as pb;

use super::{N, join, scalar};
use crate::render::{Cx, Lines, fit_columns, link, prefixed, tint};
use crate::text::{clean, line_width, push_text, sline, truncate, truncate_spans, width, wrap};

/// A row of an `EntityList`.
struct Entity {
    title: String,
    subtitle: Option<String>,
    trailing: Vec<Span<'static>>,
}

impl<'a> Cx<'a> {
    fn ui_blocks(&self, n: N<'_>) -> Option<Vec<pb::Block>> {
        serde_json::from_value(n.prop("blocks")?.clone()).ok()
    }

    /// The `blocks` the daemon parsed; the plain `text` when there are none.
    fn ui_text_lines(&mut self, n: N<'_>, w: usize, tinted: bool) -> Lines {
        match self.ui_blocks(n) {
            Some(blocks) => {
                let role = match n.str("role") {
                    Some("title") => Some(pb::TextRole::Title),
                    Some("code") => Some(pb::TextRole::Code),
                    _ => None,
                };
                let c = pb::TextComponent {
                    role,
                    tone: n.tone().filter(|_| tinted),
                    blocks,
                };
                self.text(&c, w)
            }
            None => match n.str("text") {
                Some(t) => {
                    let mut lines = wrap(&[Span::raw(clean(t).into_owned())], w);
                    if let Some(tone) = n.tone().filter(|_| tinted) {
                        tint(&mut lines, self.p.tone(tone));
                    }
                    lines
                }
                None => Vec::new(),
            },
        }
    }

    pub(super) fn ui_text(&mut self, n: N<'_>, w: usize) -> Lines {
        self.ui_text_lines(n, w, true)
    }

    /// A gutter in the tone's color (the accent by default), the title bold on top.
    pub(super) fn ui_callout(&mut self, n: N<'_>, w: usize) -> Lines {
        let tone = n.tone().unwrap_or(pb::Tone::Accent);
        let color = self.p.tone(tone);
        let bar = Style::new().fg(color);
        let inner = w.saturating_sub(2).max(1);
        let mut body = Vec::new();
        if let Some(title) = n.line("title") {
            body.push(sline(
                truncate(&title, inner),
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            ));
        }
        // The tone is the gutter's and the title's; the text keeps its own color.
        let text = self.ui_text_lines(n, inner, false);
        body.extend(text);
        if w < 4 {
            return body;
        }
        body.into_iter()
            .map(|line| {
                if line.spans.is_empty() {
                    sline("┃", bar)
                } else {
                    let mut spans = vec![Span::styled("┃ ", bar)];
                    spans.extend(line.spans);
                    Line::from(spans)
                }
            })
            .collect()
    }

    pub(super) fn ui_code(&mut self, n: N<'_>, w: usize) -> Lines {
        let code = n.str("code").unwrap_or("");
        let language = n.str("language").filter(|l| !l.is_empty());
        self.code(code, language, false, w)
    }

    /// `[label]`, in the tone's color.
    fn tag_spans(&self, n: N<'_>) -> Vec<Span<'static>> {
        let label = n.line("label").unwrap_or_default();
        let style = match n.tone() {
            Some(t) => Style::new().fg(self.p.tone(t)),
            None => Style::new(),
        };
        vec![Span::styled(format!("[{label}]"), style)]
    }

    pub(super) fn ui_tag(&mut self, n: N<'_>, w: usize) -> Lines {
        vec![Line::from(truncate_spans(self.tag_spans(n), w))]
    }

    /// A node as a run of spans on one line (a table cell).
    fn inline_spans(&mut self, n: N<'_>, path: &str) -> Vec<Span<'static>> {
        match n.name() {
            "Tag" => self.tag_spans(n),
            "Link" => {
                let text = n.line("text").or_else(|| n.line("url")).unwrap_or_default();
                vec![Span::styled(
                    text,
                    Style::new()
                        .fg(self.p.accent)
                        .add_modifier(Modifier::UNDERLINED),
                )]
            }
            _ => {
                let lines = self.quiet(|cx| cx.ui_node(n, path, 80));
                let mut spans: Vec<Span<'static>> = Vec::new();
                for (i, line) in lines.into_iter().enumerate() {
                    if i > 0 {
                        push_text(&mut spans, " ", Style::new());
                    }
                    spans.extend(line.spans);
                }
                spans
            }
        }
    }

    /// A header row and a rule, then the rows; numbers align to the right. Columns that don't
    /// fit are cut, the widest first.
    pub(super) fn ui_table(&mut self, n: N<'_>, path: &str, w: usize) -> Lines {
        let columns: Vec<String> = n
            .prop("columns")
            .and_then(Value::as_array)
            .map(|c| c.iter().map(scalar).collect())
            .unwrap_or_default();
        let raw_rows: Vec<&Vec<Value>> = n
            .prop("rows")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(|row| row.as_array()).collect())
            .unwrap_or_default();
        let cols = raw_rows
            .iter()
            .map(|r| r.len())
            .chain([columns.len()])
            .max()
            .unwrap_or(0);
        if cols == 0 {
            return Vec::new();
        }
        // (spans, is a number)
        let mut rows: Vec<Vec<(Vec<Span<'static>>, bool)>> = Vec::new();
        for (r, row) in raw_rows.iter().enumerate() {
            let mut cells = Vec::new();
            for c in 0..cols {
                let v = row.get(c).unwrap_or(&Value::Null);
                let cell = match N::new(v) {
                    Some(node) => {
                        let p = join(path, &format!("rows.{r}.{c}"));
                        (self.inline_spans(node, &p), false)
                    }
                    None => (vec![Span::raw(scalar(v))], v.is_number()),
                };
                cells.push(cell);
            }
            rows.push(cells);
        }
        let numeric: Vec<bool> = (0..cols)
            .map(|c| {
                rows.iter().all(|r| r[c].1 || r[c].0.is_empty()) && rows.iter().any(|r| r[c].1)
            })
            .collect();
        let head: Vec<Vec<Span<'static>>> = (0..cols)
            .map(|c| {
                vec![Span::styled(
                    columns.get(c).cloned().unwrap_or_default(),
                    Style::new().add_modifier(Modifier::BOLD),
                )]
            })
            .collect();
        let mut widths: Vec<usize> = (0..cols)
            .map(|c| {
                rows.iter()
                    .map(|r| line_width(&r[c].0))
                    .chain([line_width(&head[c])])
                    .max()
                    .unwrap_or(1)
                    .clamp(1, w)
            })
            .collect();
        fit_columns(&mut widths, w.saturating_sub(3 * (cols - 1)));
        let border = Style::new().fg(self.p.border);
        let line = |cells: Vec<(Vec<Span<'static>>, bool)>| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for (c, (cell, _)) in cells.into_iter().enumerate() {
                if c > 0 {
                    spans.push(Span::styled(" │ ", border));
                }
                let cell = truncate_spans(cell, widths[c]);
                let room = widths[c] - line_width(&cell);
                if numeric[c] {
                    spans.push(Span::raw(" ".repeat(room)));
                    spans.extend(cell);
                } else {
                    spans.extend(cell);
                    if c + 1 < cols {
                        spans.push(Span::raw(" ".repeat(room)));
                    }
                }
            }
            Line::from(spans)
        };
        let rule = sline(
            widths
                .iter()
                .map(|cw| "─".repeat(*cw))
                .collect::<Vec<_>>()
                .join("─┼─"),
            border,
        );
        let mut out = vec![line(head.into_iter().map(|h| (h, false)).collect()), rule];
        out.extend(rows.into_iter().map(line));
        out
    }

    /// `•` or `1.` and the item: a string, or any node.
    pub(super) fn ui_list(&mut self, n: N<'_>, path: &str, w: usize) -> Lines {
        let ordered = n.bool("ordered") == Some(true);
        let items: Vec<&Value> = n
            .prop("items")
            .and_then(Value::as_array)
            .map(|a| a.iter().collect())
            .unwrap_or_default();
        let digits = items.len().to_string().len();
        let mut out = Vec::new();
        for (i, item) in items.into_iter().enumerate() {
            let marker = if ordered {
                format!("{:>digits$}. ", i + 1)
            } else {
                "• ".to_string()
            };
            let pad = width(&marker);
            let inner = w.saturating_sub(pad).max(1);
            let mut body = match N::new(item) {
                Some(node) => self.ui_node(node, &join(path, &format!("items.{i}")), inner),
                None => wrap(&[Span::raw(clean(&scalar(item)).into_owned())], inner),
            };
            if body.is_empty() {
                body.push(Line::default());
            }
            out.extend(prefixed(
                body,
                vec![Span::styled(marker, self.p.muted_style())],
                vec![Span::raw(" ".repeat(pad))],
            ));
        }
        out
    }

    /// A value or a node as the spans of a trailing cell.
    fn trailing_spans(&mut self, v: &Value, path: &str) -> Vec<Span<'static>> {
        match N::new(v) {
            Some(node) => self.inline_spans(node, path),
            None => match scalar(v) {
                t if t.is_empty() => Vec::new(),
                t => vec![Span::raw(t)],
            },
        }
    }

    /// Rows of a bold title, a muted subtitle and a trailing value or tag, in columns:
    /// `Ada Lovelace  analyst        [admin]`. When the columns do not fit, the subtitle goes
    /// under its title.
    pub(super) fn ui_entity_list(&mut self, n: N<'_>, path: &str, w: usize) -> Lines {
        let mut rows: Vec<Entity> = Vec::new();
        for (i, item) in n.nodes("items").into_iter().enumerate() {
            let trailing = match item.prop("trailing") {
                Some(v) => self.trailing_spans(v, &join(path, &format!("items.{i}.trailing"))),
                None => Vec::new(),
            };
            rows.push(Entity {
                title: item.line("title").unwrap_or_default(),
                subtitle: item.line("subtitle"),
                trailing,
            });
        }
        let title_w = rows.iter().map(|r| width(&r.title)).max().unwrap_or(0);
        let sub_w = rows
            .iter()
            .filter_map(|r| r.subtitle.as_deref())
            .map(width)
            .max()
            .unwrap_or(0);
        let tail_w = rows
            .iter()
            .map(|r| line_width(&r.trailing))
            .max()
            .unwrap_or(0);
        // The trailing column with the gap before it.
        let tail = if tail_w > 0 { tail_w + 2 } else { 0 };
        let bold = Style::new().add_modifier(Modifier::BOLD);
        let muted = self.p.muted_style();
        let gap = if sub_w > 0 { 2 } else { 0 };
        let sub_room = w.saturating_sub(title_w + gap + tail);
        let columns = title_w + tail <= w && (sub_w == 0 || sub_room >= sub_w.min(12));
        let mut out: Lines = Vec::new();
        for row in rows {
            let right = |trailing: Vec<Span<'static>>| {
                let mut spans = vec![Span::raw(" ".repeat(tail_w - line_width(&trailing)))];
                spans.extend(trailing);
                spans
            };
            if columns {
                let sub = row.subtitle.as_deref().map(|s| truncate(s, sub_room));
                let sub_col = sub_w.min(sub_room);
                let mut spans = vec![Span::styled(row.title.clone(), bold)];
                spans.push(Span::raw(" ".repeat(title_w - width(&row.title))));
                if sub_w > 0 {
                    let used = sub.as_deref().map_or(0, width);
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(sub.unwrap_or_default(), muted));
                    spans.push(Span::raw(" ".repeat(sub_col - used)));
                }
                if tail_w > 0 {
                    spans.push(Span::raw("  "));
                    spans.extend(right(row.trailing));
                }
                out.push(Line::from(spans));
                continue;
            }
            // Stacked: the title and the trailing value on one line, the subtitle under.
            let head_w = w.min(title_w + tail);
            let title = truncate(&row.title, head_w.saturating_sub(tail).max(1));
            let pad = head_w.saturating_sub(width(&title) + tail);
            let mut spans = vec![Span::styled(title, bold), Span::raw(" ".repeat(pad))];
            if tail_w > 0 && head_w >= tail {
                spans.push(Span::raw("  "));
                spans.extend(right(row.trailing));
            }
            out.push(Line::from(spans));
            if let Some(sub) = row.subtitle {
                let body = wrap(&[Span::styled(sub, muted)], w.saturating_sub(2).max(1));
                out.extend(prefixed(body, vec![Span::raw("  ")], vec![Span::raw("  ")]));
            }
        }
        out
    }

    pub(super) fn ui_key_value(&mut self, n: N<'_>, w: usize) -> Lines {
        let items = n
            .nodes("pairs")
            .into_iter()
            .map(|kv| pb::KvItem {
                key: kv.str("key").unwrap_or("").to_string(),
                value: kv.prop("value").map(scalar).unwrap_or_default(),
                tone: kv.tone(),
            })
            .collect();
        self.kv(&pb::KvComponent { items }, w)
    }

    /// The value in bold with the delta after it in the tone's color, and the label.
    fn metric_parts(&self, n: N<'_>) -> (Vec<Span<'static>>, Option<String>) {
        let value = n.prop("value").map(scalar).unwrap_or_default();
        let mut spans = vec![Span::styled(
            value,
            Style::new().add_modifier(Modifier::BOLD),
        )];
        if let Some(delta) = n.line("delta") {
            let style = match n.tone() {
                Some(t) => Style::new().fg(self.p.tone(t)),
                None => self.p.muted_style(),
            };
            spans.push(Span::styled(format!(" {delta}"), style));
        }
        (spans, n.line("label"))
    }

    /// `1,284 ▲ 12% vs last week`: the value bold, the change with its arrow in the tone's color
    /// (up is success and down danger unless the node says otherwise), the note muted.
    pub(super) fn ui_metric_indicator(&mut self, n: N<'_>, w: usize) -> Lines {
        let (arrow, by_direction) = match n.str("direction") {
            Some("up") => ("▲", Some(pb::Tone::Success)),
            Some("down") => ("▼", Some(pb::Tone::Danger)),
            _ => ("", None),
        };
        let change = match (arrow, n.line("delta")) {
            ("", None) => None,
            ("", Some(d)) => Some(d),
            (a, None) => Some(a.to_string()),
            (a, Some(d)) => Some(format!("{a} {d}")),
        };
        let mut spans = vec![Span::styled(
            n.prop("value").map(scalar).unwrap_or_default(),
            Style::new().add_modifier(Modifier::BOLD),
        )];
        if let Some(change) = change {
            let style = match n.tone().or(by_direction) {
                Some(t) => Style::new().fg(self.p.tone(t)),
                None => self.p.muted_style(),
            };
            spans.push(Span::styled(format!(" {change}"), style));
        }
        if let Some(note) = n.line("subtext") {
            spans.push(Span::styled(format!(" {note}"), self.p.muted_style()));
        }
        wrap(&spans, w)
    }

    /// The value in bold with the delta in the tone's color, the label muted under them.
    pub(super) fn ui_metric(&mut self, n: N<'_>, w: usize) -> Lines {
        self.ui_metric_strip(&[n], w)
    }

    /// Metrics side by side, each a column: value and delta over the label, `│` between
    /// them. More than fit one line wrap onto further strips.
    pub(super) fn ui_metric_strip(&mut self, ms: &[N<'_>], w: usize) -> Lines {
        let w = w.max(1);
        let cols: Vec<(Vec<Span<'static>>, Option<String>, usize)> = ms
            .iter()
            .map(|m| {
                let (top, label) = self.metric_parts(*m);
                let top = truncate_spans(top, w);
                let label = label.map(|l| truncate(&l, w));
                let cell = line_width(&top).max(label.as_deref().map_or(0, width));
                (top, label, cell)
            })
            .collect();
        let total = |sep: usize| cols.iter().map(|c| c.2).sum::<usize>() + sep * (cols.len() - 1);
        // Roomy dividers when the strip is one line anyway.
        let sep = if total(5) <= w { 5 } else { 3 };
        let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
        let mut used = 0;
        for (i, c) in cols.iter().enumerate() {
            let row = rows.last_mut().expect("a row");
            let need = c.2 + if row.is_empty() { 0 } else { sep };
            if !row.is_empty() && used + need > w {
                used = 0;
                rows.push(vec![i]);
                used += c.2;
            } else {
                used += need;
                row.push(i);
            }
        }
        let border = Style::new().fg(self.p.border);
        let divider = |spans: &mut Vec<Span<'static>>| {
            let pad = " ".repeat(sep / 2);
            spans.push(Span::raw(pad.clone()));
            spans.push(Span::styled("│", border));
            spans.push(Span::raw(pad));
        };
        let mut out: Lines = Vec::new();
        for row in rows {
            if !out.is_empty() {
                out.push(Line::default());
            }
            let labelled = row.iter().any(|i| cols[*i].1.is_some());
            let (mut top, mut under): (Vec<Span<'static>>, Vec<Span<'static>>) =
                (Vec::new(), Vec::new());
            for (k, i) in row.iter().enumerate() {
                let (spans, label, cell) = &cols[*i];
                let last = k + 1 == row.len();
                if k > 0 {
                    divider(&mut top);
                    divider(&mut under);
                }
                top.extend(spans.iter().cloned());
                let label = label.clone().unwrap_or_default();
                let (top_pad, under_pad) = (cell - line_width(spans), cell - width(&label));
                if !label.is_empty() {
                    under.push(Span::styled(label, self.p.muted_style()));
                }
                if !last {
                    for (line, pad) in [(&mut top, top_pad), (&mut under, under_pad)] {
                        if pad > 0 {
                            line.push(Span::raw(" ".repeat(pad)));
                        }
                    }
                }
            }
            out.push(Line::from(top));
            if labelled {
                out.push(Line::from(under));
            }
        }
        out
    }

    /// `▰▰▰▱▱▱ 60% label`; the percent is left out when the label says it (`92% used`).
    pub(super) fn ui_progress(&mut self, n: N<'_>, w: usize) -> Lines {
        let value = n.num("value").filter(|v| !v.is_nan()).unwrap_or(0.0);
        let value = value.clamp(0.0, 1.0);
        let percent = (value * 100.0).round() as u32;
        let label = n.line("label");
        let said = label.as_deref().is_some_and(|l| says_percent(l, percent));
        let tail = match (&label, said) {
            (Some(l), true) => l.clone(),
            (Some(l), false) => format!("{percent}% {l}"),
            (None, _) => format!("{percent}%"),
        };
        let bar_for = |cells: usize| -> Vec<Span<'static>> {
            let filled = ((value * cells as f64).round() as usize).min(cells);
            vec![
                Span::styled("▰".repeat(filled), Style::new().fg(self.p.accent)),
                Span::styled("▱".repeat(cells - filled), self.p.muted_style()),
            ]
        };
        let tail_style = self.p.muted_style();
        let bar_max = 40;
        // The bar and its text on one line when the bar keeps at least ten cells.
        if w >= width(&tail) + 11 {
            let mut spans = bar_for((w - width(&tail) - 1).min(bar_max));
            spans.push(Span::styled(format!(" {tail}"), tail_style));
            return vec![Line::from(spans)];
        }
        let mut out = vec![Line::from(bar_for(w.min(bar_max)))];
        out.extend(wrap(&[Span::styled(tail, tail_style)], w));
        out
    }

    pub(super) fn ui_image(&mut self, n: N<'_>, w: usize) -> Lines {
        let c = pb::ImageComponent {
            src: n.str("src").unwrap_or("").to_string(),
            alt: n.line("alt"),
            caption: None,
            width: None,
            height: None,
        };
        self.image(&c, w)
    }

    pub(super) fn ui_link(&mut self, n: N<'_>, w: usize) -> Lines {
        let url = n.str("url").unwrap_or("");
        let text = n.str("text");
        wrap(&link(url, text, Style::new(), &self.p), w)
    }
}

/// Whether `label` has a number with a `%` after it that rounds to `percent`.
fn says_percent(label: &str, percent: u32) -> bool {
    let chars: Vec<char> = label.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let number: String = chars[start..i].iter().collect();
            if chars.get(i) == Some(&'%')
                && number
                    .parse::<f64>()
                    .is_ok_and(|v| v.round() as u32 == percent)
            {
                return true;
            }
        } else {
            i += 1;
        }
    }
    false
}
