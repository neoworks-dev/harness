//! Content components: text, code, tables, lists, key/values, tags, metrics, progress, images,
//! links.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use suite_protocol as pb;

use super::{N, join, scalar};
use crate::render::{Cx, Lines, fit_columns, link, prefixed, tint};
use crate::text::{clean, line_width, push_text, sline, truncate, truncate_spans, width, wrap};

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

    /// The label muted over the value in bold, with the delta in the tone's color.
    pub(super) fn ui_metric(&mut self, n: N<'_>, w: usize) -> Lines {
        let mut out = Vec::new();
        if let Some(label) = n.line("label") {
            out.push(sline(truncate(&label, w), self.p.muted_style()));
        }
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
        out.push(Line::from(truncate_spans(spans, w)));
        out
    }

    pub(super) fn ui_progress(&mut self, n: N<'_>, w: usize) -> Lines {
        let c = pb::ProgressComponent {
            value: n.num("value").unwrap_or(0.0),
            label: n.line("label"),
        };
        self.progress(&c, w)
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
