//! Layout components: stacks (with a measured row), cards, headers, tabs, accordions, steps.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use suite_protocol as pb;

use super::{N, UiKind, join};
use crate::render::{Cx, Lines, card_inner, prefixed, stack};
use crate::text::{clip, sline, truncate, width};

/// The id of the target for the node at `path`.
fn path_id(path: &str) -> String {
    if path.is_empty() {
        "/".into()
    } else {
        path.to_string()
    }
}

fn space(gap: Option<&str>) -> Option<pb::Space> {
    (gap == Some("l")).then_some(pb::Space::L)
}

/// The widest line.
pub(super) fn max_width(lines: &[Line<'_>]) -> usize {
    lines.iter().map(Line::width).max().unwrap_or(0)
}

/// `lines` shifted right by `by` cells.
fn shift(lines: Lines, by: usize) -> Lines {
    if by == 0 {
        return lines;
    }
    let pad = vec![Span::raw(" ".repeat(by))];
    prefixed(lines, pad.clone(), pad)
}

fn pad_to(line: &mut Line<'static>, w: usize) {
    let used = line.width();
    if used < w {
        line.spans.push(Span::raw(" ".repeat(w - used)));
    }
}

/// One row of items, each already drawn at its width, side by side. `valign` places items
/// that are shorter than the tallest.
fn columns(parts: Vec<(Lines, usize)>, gaps: &[usize], valign: &str) -> Lines {
    let height = parts.iter().map(|(l, _)| l.len()).max().unwrap_or(0);
    let mut cols: Vec<(Lines, usize)> = Vec::new();
    for (mut lines, w) in parts {
        let missing = height - lines.len();
        let top = match valign {
            "center" => missing / 2,
            "end" => missing,
            _ => 0,
        };
        let mut padded: Lines = vec![Line::default(); top];
        padded.append(&mut lines);
        padded.resize(height, Line::default());
        cols.push((padded, w));
    }
    let last = cols.len().saturating_sub(1);
    (0..height)
        .map(|i| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for (c, (lines, w)) in cols.iter().enumerate() {
                let mut line = lines[i].clone();
                if c < last {
                    pad_to(&mut line, *w);
                }
                spans.extend(line.spans);
                if c < last {
                    spans.push(Span::raw(" ".repeat(gaps[c])));
                }
            }
            Line::from(spans)
        })
        .collect()
}

impl<'a> Cx<'a> {
    pub(crate) fn ui_node(&mut self, n: N<'_>, path: &str, w: usize) -> Lines {
        let w = w.max(1);
        let mut lines = match n.name() {
            "Stack" => self.ui_stack(n, path, w),
            "Card" => self.ui_card(n, path, w),
            "Header" => self.ui_header(n, w),
            "Separator" => vec![sline("─".repeat(w), Style::new().fg(self.p.border))],
            "Tabs" => self.ui_tabs(n, path, w),
            "Accordion" => self.ui_accordion(n, path, w),
            "Steps" => self.ui_steps(n, w),
            "Text" => self.ui_text(n, w),
            "Callout" => self.ui_callout(n, w),
            "Code" => self.ui_code(n, w),
            "Table" => self.ui_table(n, path, w),
            "List" => self.ui_list(n, path, w),
            "KeyValue" => self.ui_key_value(n, w),
            "Tag" => self.ui_tag(n, w),
            "Metric" => self.ui_metric(n, w),
            "Progress" => self.ui_progress(n, w),
            "Image" => self.ui_image(n, w),
            "Link" => self.ui_link(n, w),
            "BarChart" => self.ui_bar_chart(n, w),
            "LineChart" => self.ui_line_chart(n, false, w),
            "AreaChart" => self.ui_line_chart(n, true, w),
            "PieChart" => self.ui_pie_chart(n, w),
            "Sparkline" => self.ui_sparkline(n, w),
            "Form" => self.ui_form(n, path, w),
            "FormControl" => self.ui_form_control(n, path, w),
            "Input" | "TextArea" | "DatePicker" => self.ui_text_field(n, w),
            "Select" => self.ui_select(n, w),
            "RadioGroup" => self.ui_radio(n, w),
            "CheckBox" | "Switch" => self.ui_toggle(n, w),
            "Slider" => self.ui_slider(n, w),
            "Button" => self.ui_button_line(n, w),
            "Buttons" => self.ui_buttons(n, w),
            other => vec![self.placeholder(
                &format!("[unsupported: {}]", crate::text::one_line(other)),
                w,
            )],
        };
        clip(&mut lines, w);
        lines
    }

    /// The children of the node's `prop` array, stacked.
    pub(super) fn ui_children(
        &mut self,
        n: N<'_>,
        prop: &str,
        path: &str,
        w: usize,
        gap: Option<&str>,
    ) -> Lines {
        let mut parts = Vec::new();
        for (i, kid) in n.nodes(prop).into_iter().enumerate() {
            let lines = self.ui_node(kid, &join(path, &format!("{prop}.{i}")), w);
            if !lines.is_empty() {
                parts.push(lines);
            }
        }
        stack(parts, space(gap))
    }

    fn ui_stack(&mut self, n: N<'_>, path: &str, w: usize) -> Lines {
        if n.str("direction") == Some("row") {
            return self.ui_row(n, path, w);
        }
        let gap = n.str("gap");
        let align = n.str("align").unwrap_or("start");
        let mut parts = Vec::new();
        for (i, kid) in n.nodes("children").into_iter().enumerate() {
            let lines = self.ui_node(kid, &join(path, &format!("children.{i}")), w);
            if lines.is_empty() {
                continue;
            }
            let room = w.saturating_sub(max_width(&lines));
            parts.push(match align {
                "center" => shift(lines, room / 2),
                "end" => shift(lines, room),
                _ => lines,
            });
        }
        stack(parts, space(gap))
    }

    /// The narrowest a node is drawn in. Text breaks at words and containers give way; what
    /// can't (a metric, a tag, a button) keeps its width.
    fn ui_min(&mut self, n: N<'_>, pref: usize) -> usize {
        let word = |s: &str| s.split_whitespace().map(width).max().unwrap_or(0);
        let min = match n.name() {
            "Metric" | "Tag" | "Link" | "Button" => pref,
            "Header" => word(n.str("title").unwrap_or("")),
            "Text" | "Callout" => {
                let blocks = n
                    .prop("blocks")
                    .and_then(|b| serde_json::from_value::<Vec<pb::Block>>(b.clone()).ok());
                let text = match blocks {
                    Some(b) => crate::blocks_text(&b),
                    None => n.str("text").unwrap_or("").to_string(),
                };
                word(&text) + if n.name() == "Callout" { 2 } else { 0 }
            }
            _ => 12,
        };
        min.clamp(1, pref.max(1))
    }

    /// Children side by side at their preferred widths; when they don't fit, squeezed down to
    /// their minimum, else wrapped onto further lines (`wrap`) or stacked.
    fn ui_row(&mut self, n: N<'_>, path: &str, w: usize) -> Lines {
        let gap = match n.str("gap") {
            Some("none") => 0,
            Some("s") => 2,
            Some("l") => 5,
            _ => 3,
        };
        let wrap = n.bool("wrap") == Some(true);
        let valign = n.str("align").unwrap_or("start");
        let justify = n.str("justify").unwrap_or("start");
        struct Item<'n> {
            node: N<'n>,
            path: String,
            min: usize,
            pref: usize,
        }
        let mut items: Vec<Item<'_>> = Vec::new();
        for (i, node) in n.nodes("children").into_iter().enumerate() {
            let p = join(path, &format!("children.{i}"));
            let lines = self.quiet(|cx| cx.ui_node(node, &p, w));
            if lines.is_empty() {
                continue;
            }
            let pref = max_width(&lines).clamp(1, w);
            let min = self.ui_min(node, pref);
            items.push(Item {
                node,
                path: p,
                min,
                pref,
            });
        }
        let count = items.len();
        if count == 0 {
            return Vec::new();
        }
        let gaps_total = gap * (count - 1);
        let pref_total: usize = items.iter().map(|i| i.pref).sum::<usize>() + gaps_total;
        let min_total: usize = items.iter().map(|i| i.min).sum::<usize>() + gaps_total;

        // The widths of each line of items.
        let mut rows: Vec<Vec<(usize, usize)>> = Vec::new(); // (item index, width)
        if pref_total <= w {
            rows.push(
                items
                    .iter()
                    .enumerate()
                    .map(|(i, it)| (i, it.pref))
                    .collect(),
            );
        } else if wrap {
            let mut row: Vec<(usize, usize)> = Vec::new();
            let mut used = 0;
            for (i, it) in items.iter().enumerate() {
                let need = it.pref + if row.is_empty() { 0 } else { gap };
                if !row.is_empty() && used + need > w {
                    rows.push(std::mem::take(&mut row));
                    used = 0;
                }
                used += it.pref + if row.is_empty() { 0 } else { gap };
                row.push((i, it.pref));
            }
            rows.push(row);
        } else if min_total <= w {
            let mut widths: Vec<usize> = items.iter().map(|i| i.min).collect();
            let mut room = w - min_total;
            while room > 0 {
                let mut grew = false;
                for (i, it) in items.iter().enumerate() {
                    if room > 0 && widths[i] < it.pref {
                        widths[i] += 1;
                        room -= 1;
                        grew = true;
                    }
                }
                if !grew {
                    break;
                }
            }
            rows.push(widths.into_iter().enumerate().collect());
        } else {
            // Not even squeezed: one under the other.
            let mut parts = Vec::new();
            for it in &items {
                parts.push(self.ui_node(it.node, &it.path, w));
            }
            return stack(parts, None);
        }

        let mut out: Lines = Vec::new();
        for row in rows {
            let used: usize = row.iter().map(|(_, rw)| rw).sum::<usize>() + gap * (row.len() - 1);
            let spare = w.saturating_sub(used);
            let (lead, gaps): (usize, Vec<usize>) = match justify {
                "center" => (spare / 2, vec![gap; row.len() - 1]),
                "end" => (spare, vec![gap; row.len() - 1]),
                "between" if row.len() > 1 => {
                    let slots = row.len() - 1;
                    let each = spare / slots;
                    let extra = spare % slots;
                    (
                        0,
                        (0..slots)
                            .map(|s| gap + each + usize::from(s < extra))
                            .collect(),
                    )
                }
                _ => (0, vec![gap; row.len().saturating_sub(1)]),
            };
            let mut parts = Vec::new();
            for (i, rw) in row {
                let it = &items[i];
                parts.push((self.ui_node(it.node, &it.path, rw), rw));
            }
            out.extend(shift(columns(parts, &gaps, valign), lead));
        }
        out
    }

    fn ui_card(&mut self, n: N<'_>, path: &str, w: usize) -> Lines {
        let title = n.line("title");
        let tone = n.tone();
        match n.str("variant") {
            Some("clear") => {
                let mut out = Vec::new();
                if let Some(t) = &title {
                    let mut st = Style::new().add_modifier(Modifier::BOLD);
                    if let Some(tone) = tone {
                        st = st.fg(self.p.tone(tone));
                    }
                    out.push(sline(truncate(t, w), st));
                }
                out.extend(self.ui_children(n, "children", path, w, None));
                out
            }
            Some("sunk") => {
                let inner = if w < 4 { w } else { w - 2 };
                let mut body = self.ui_children(n, "children", path, inner, None);
                if let Some(t) = &title {
                    let mut st = Style::new().add_modifier(Modifier::BOLD);
                    if let Some(tone) = tone {
                        st = st.fg(self.p.tone(tone));
                    }
                    body.insert(0, sline(truncate(t, inner), st));
                }
                let surface = self.p.surface;
                body.into_iter()
                    .map(|mut line| {
                        if w >= 4 {
                            line.spans.insert(0, Span::raw(" "));
                        }
                        let used = line.width();
                        line.spans
                            .push(Span::raw(" ".repeat(w.saturating_sub(used))));
                        for s in &mut line.spans {
                            s.style = s.style.bg(surface);
                        }
                        line
                    })
                    .collect()
            }
            _ => {
                let body = self.ui_children(n, "children", path, card_inner(w), None);
                self.card_frame(title.as_deref(), tone, body, w)
            }
        }
    }

    /// The title in bold; the subtitle muted after it, or under it when they do not fit.
    fn ui_header(&mut self, n: N<'_>, w: usize) -> Lines {
        let bold = Style::new().add_modifier(Modifier::BOLD);
        let title = n.line("title").unwrap_or_default();
        let Some(sub) = n.line("subtitle") else {
            return vec![sline(truncate(&title, w), bold)];
        };
        if title.is_empty() {
            return vec![sline(truncate(&sub, w), self.p.muted_style())];
        }
        if width(&title) + 2 + width(&sub) <= w {
            return vec![Line::from(vec![
                Span::styled(title, bold),
                Span::styled(format!("  {sub}"), self.p.muted_style()),
            ])];
        }
        vec![
            sline(truncate(&title, w), bold),
            sline(truncate(&sub, w), self.p.muted_style()),
        ]
    }

    /// `│ Overview │ Billing │` with the selected tab reversed (and in the accent color while
    /// the strip has focus), over the children of the selected tab.
    fn ui_tabs(&mut self, n: N<'_>, path: &str, w: usize) -> Lines {
        let tabs = n.nodes("tabs");
        if tabs.is_empty() {
            return Vec::new();
        }
        let id = path_id(path);
        let selected = self
            .ui_state()
            .and_then(|s| s.tabs.get(&id))
            .copied()
            .unwrap_or(0)
            .min(tabs.len() - 1);
        let labels: Vec<String> = tabs
            .iter()
            .map(|t| t.line("label").unwrap_or_default())
            .collect();
        self.ui_target(
            &id,
            UiKind::Tabs { count: tabs.len() },
            labels[selected].clone(),
            Value::from(selected),
        );
        let focused = self.ui_focused(&id);
        let mut strip = self.tab_strip(&labels, selected, focused, w);
        if focused {
            self.mark_focus(&mut strip);
        }
        let mut out = vec![strip];
        let tab_path = join(path, &format!("tabs.{selected}"));
        out.extend(self.ui_children(tabs[selected], "children", &tab_path, w, n.str("gap")));
        out
    }

    fn tab_strip(
        &self,
        labels: &[String],
        selected: usize,
        focused: bool,
        w: usize,
    ) -> Line<'static> {
        let border = Style::new().fg(self.p.border);
        let on = if focused {
            Style::new()
                .fg(self.p.accent)
                .add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::new().add_modifier(Modifier::REVERSED)
        };
        // Width of a tab with its bars: " label │".
        let cell = |i: usize| width(&labels[i]) + 3;
        let mut first = 0;
        let fits = |first: usize| {
            let mut used = 1 + if first > 0 { 2 } else { 0 };
            for i in first..=selected {
                used += cell(i);
            }
            used <= w
        };
        while first < selected && !fits(first) {
            first += 1;
        }
        let mut spans = Vec::new();
        let mut used = 1;
        if first > 0 {
            spans.push(Span::styled("‹ ", self.p.muted_style()));
            used += 2;
        }
        spans.push(Span::styled("│", border));
        let mut last = first;
        for i in first..labels.len() {
            let label = if i == selected && cell(i) + used > w {
                truncate(&labels[i], w.saturating_sub(used + 3).max(1))
            } else {
                labels[i].clone()
            };
            let need = width(&label) + 3;
            let more = i + 1 < labels.len();
            if i > selected && used + need + if more { 2 } else { 0 } > w {
                break;
            }
            let style = if i == selected {
                on
            } else {
                self.p.muted_style()
            };
            spans.push(Span::styled(format!(" {label} "), style));
            spans.push(Span::styled("│", border));
            used += need;
            last = i;
        }
        if last + 1 < labels.len() {
            spans.push(Span::styled(" ›", self.p.muted_style()));
        }
        Line::from(spans)
    }

    /// `▸ title` closed, `▾ title` open (children under it, indented). Each item can be focused.
    fn ui_accordion(&mut self, n: N<'_>, path: &str, w: usize) -> Lines {
        let mut out: Lines = Vec::new();
        for (i, item) in n.nodes("items").into_iter().enumerate() {
            let ipath = join(path, &format!("items.{i}"));
            let open = self
                .ui_state()
                .and_then(|s| s.open.get(&ipath))
                .copied()
                .unwrap_or_else(|| item.bool("open") == Some(true));
            let title = item.line("title").unwrap_or_default();
            self.ui_target(&ipath, UiKind::Accordion, title.clone(), Value::Bool(open));
            let focused = self.ui_focused(&ipath);
            let chevron = if open { "▾ " } else { "▸ " };
            let mut head = if focused {
                let st = Style::new()
                    .fg(self.p.accent)
                    .add_modifier(Modifier::REVERSED | Modifier::BOLD);
                Line::from(Span::styled(truncate(&format!("{chevron}{title}"), w), st))
            } else {
                Line::from(vec![
                    Span::styled(chevron, self.p.muted_style()),
                    Span::styled(
                        truncate(&title, w.saturating_sub(2)),
                        Style::new().add_modifier(if open {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                    ),
                ])
            };
            if focused {
                self.mark_focus(&mut head);
            }
            out.push(head);
            if open {
                let inner = w.saturating_sub(2).max(1);
                let body = self.ui_children(item, "children", &ipath, inner, None);
                out.extend(shift(body, if w > 2 { 2 } else { 0 }));
            }
        }
        out
    }

    /// `✓ done`, `● current`, `○ pending`, `✗ failed`, the detail muted after the title.
    fn ui_steps(&mut self, n: N<'_>, w: usize) -> Lines {
        let mut out = Vec::new();
        for step in n.nodes("steps") {
            let (glyph, color) = match step.str("status") {
                Some("done") => ("✓", Some(self.p.success)),
                Some("current") => ("●", Some(self.p.accent)),
                Some("failed") => ("✗", Some(self.p.danger)),
                _ => ("○", Some(self.p.muted)),
            };
            let pending = !matches!(step.str("status"), Some("done" | "current" | "failed"));
            let title = step.line("title").unwrap_or_default();
            let mut title_style = Style::new();
            if step.str("status") == Some("current") {
                title_style = title_style.add_modifier(Modifier::BOLD);
            }
            if pending {
                title_style = self.p.muted_style();
            }
            let lead = vec![Span::styled(
                format!("{glyph} "),
                Style::new().fg(color.unwrap_or(self.p.fg)),
            )];
            let inner = w.saturating_sub(2).max(1);
            let mut spans = vec![Span::styled(title.clone(), title_style)];
            match step.line("detail") {
                Some(d) if width(&title) + 2 + width(&d) <= inner => {
                    spans.push(Span::styled(format!("  {d}"), self.p.muted_style()));
                    out.extend(prefixed(
                        vec![Line::from(spans)],
                        lead,
                        vec![Span::raw("  ")],
                    ));
                }
                Some(d) => {
                    let mut body = crate::text::wrap(&spans, inner);
                    body.extend(crate::text::wrap(
                        &[Span::styled(d, self.p.muted_style())],
                        inner,
                    ));
                    out.extend(prefixed(body, lead, vec![Span::raw("  ")]));
                }
                None => {
                    let body = crate::text::wrap(&spans, inner);
                    out.extend(prefixed(body, lead, vec![Span::raw("  ")]));
                }
            }
        }
        out
    }
}
