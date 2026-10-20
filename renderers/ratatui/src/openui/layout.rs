//! Layout components: stacks (with a measured row), cards, headers, tabs, accordions, steps.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use suite_protocol as pb;

use super::{N, UiKind, join};
use crate::render::{Cx, Lines, prefixed, stack};
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

/// A child of a container: the node and its path in the doc.
type Kid<'n> = (N<'n>, String);

/// The fewest cells a grid column gets before the grid takes one column less.
const GRID_MIN: usize = 30;

fn kids_of<'n>(n: N<'n>, prop: &str, path: &str) -> Vec<Kid<'n>> {
    n.nodes(prop)
        .into_iter()
        .enumerate()
        .map(|(i, k)| (k, join(path, &format!("{prop}.{i}"))))
        .collect()
}

/// `lines` with blank lines added up to `height`.
fn grow(lines: &mut Lines, height: usize) {
    if lines.len() < height {
        lines.resize(height, Line::default());
    }
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
pub(super) fn columns(parts: Vec<(Lines, usize)>, gaps: &[usize], valign: &str) -> Lines {
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
            "Card" => self.ui_card_h(n, path, w, 0),
            "Grid" => self.ui_grid(n, path, w),
            "Carousel" => self.ui_carousel(n, path, w),
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
            "EntityList" => self.ui_entity_list(n, path, w),
            "Metric" => self.ui_metric(n, w),
            "MetricIndicator" => self.ui_metric_indicator(n, w),
            "Progress" => self.ui_progress(n, w),
            "Image" => self.ui_image(n, w),
            "ImageGallery" => self.ui_gallery(n, w),
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
            "OptionCards" => self.ui_option_cards(n, w),
            "CheckBoxGroup" => self.ui_toggle_group(n, w, false),
            "SwitchGroup" => self.ui_toggle_group(n, w, true),
            "CheckBox" | "Switch" => self.ui_toggle(n, w),
            "Slider" => self.ui_slider(n, w),
            "Button" => self.ui_button_line(n, w),
            "Buttons" => self.ui_buttons(n, w),
            "FollowUpBlock" => self.ui_follow_ups(n, w),
            "FollowUpItem" => {
                let (spans, focused) = self.follow_up_spans(n, w);
                let mut line = Line::from(spans);
                if focused {
                    self.mark_focus(&mut line);
                }
                vec![line]
            }
            other => vec![self.placeholder(
                &format!("[unsupported: {}]", crate::text::one_line(other)),
                w,
            )],
        };
        clip(&mut lines, w);
        lines
    }

    /// The children of the node's `prop` array, stacked (see [`Cx::ui_flow`]).
    pub(super) fn ui_children(
        &mut self,
        n: N<'_>,
        prop: &str,
        path: &str,
        w: usize,
        gap: Option<&str>,
    ) -> Lines {
        let kids = kids_of(n, prop, path);
        let parts = self.ui_flow(&kids, w);
        stack(parts, space(gap))
    }

    /// Draws siblings that follow each other down the page, one part each. A run of two or
    /// more `Card`s is one `Grid` and a run of two or more `Metric`s one strip: the terminal
    /// lays a page out, whatever column the program asked for.
    fn ui_flow(&mut self, kids: &[Kid<'_>], w: usize) -> Vec<Lines> {
        let mut parts = Vec::new();
        let mut i = 0;
        while i < kids.len() {
            let name = kids[i].0.name();
            let mut j = i + 1;
            if matches!(name, "Card" | "Metric") {
                while j < kids.len() && kids[j].0.name() == name {
                    j += 1;
                }
            }
            let run = &kids[i..j];
            let lines = match (name, run.len()) {
                ("Card", 2..) => self.grid_of(run, None, w),
                ("Metric", 2..) => {
                    let ms: Vec<N<'_>> = run.iter().map(|k| k.0).collect();
                    self.ui_metric_strip(&ms, w)
                }
                _ => self.ui_node(run[0].0, &run[0].1, w),
            };
            if !lines.is_empty() {
                parts.push(lines);
            }
            i = j;
        }
        parts
    }

    fn ui_stack(&mut self, n: N<'_>, path: &str, w: usize) -> Lines {
        if n.str("direction") == Some("row") {
            return self.ui_row(n, path, w);
        }
        let gap = n.str("gap");
        let align = n.str("align").unwrap_or("start");
        let kids = kids_of(n, "children", path);
        let parts = self
            .ui_flow(&kids, w)
            .into_iter()
            .map(|lines| {
                let room = w.saturating_sub(max_width(&lines));
                match align {
                    "center" => shift(lines, room / 2),
                    "end" => shift(lines, room),
                    _ => lines,
                }
            })
            .collect();
        stack(parts, space(gap))
    }

    /// An explicit `Grid`: its children in as many columns as fit, at most `columns`.
    fn ui_grid(&mut self, n: N<'_>, path: &str, w: usize) -> Lines {
        let kids = kids_of(n, "children", path);
        let max = n.num("columns").filter(|c| *c >= 1.0).map(|c| c as usize);
        self.grid_of(&kids, max, w)
    }

    /// Cells left to right, then down. Every column is as wide as the others (the spare cells
    /// go to the first ones), a cell of a row is as tall as the row's tallest.
    fn grid_of(&mut self, kids: &[Kid<'_>], max: Option<usize>, w: usize) -> Lines {
        if kids.is_empty() {
            return Vec::new();
        }
        let fit = (w + 1) / (GRID_MIN + 1);
        let cols = fit.max(1).min(max.unwrap_or(usize::MAX)).min(kids.len());
        self.grid_cols(kids, cols, w)
    }

    /// `kids` in rows of `cols` cells (see [`Cx::grid_of`]).
    fn grid_cols(&mut self, kids: &[Kid<'_>], cols: usize, w: usize) -> Lines {
        let room = w.saturating_sub(cols - 1);
        let widths: Vec<usize> = (0..cols)
            .map(|c| (room / cols + usize::from(c < room % cols)).max(1))
            .collect();
        let mut out: Lines = Vec::new();
        let mut loose = false;
        for row in kids.chunks(cols) {
            let heights: Vec<usize> = row
                .iter()
                .zip(&widths)
                .map(|(k, cw)| self.quiet(|cx| cx.grid_cell(k, *cw, 0)).len())
                .collect();
            let height = heights.iter().copied().max().unwrap_or(0);
            let mut parts = Vec::new();
            for (k, cw) in row.iter().zip(&widths) {
                parts.push((self.grid_cell(k, *cw, height), *cw));
            }
            let cards = row.iter().all(|k| k.0.name() == "Card");
            if !out.is_empty() && (loose || !cards) {
                out.push(Line::default());
            }
            loose = !cards;
            out.extend(columns(parts, &vec![1; cols - 1], "start"));
        }
        out
    }

    /// The children side by side as many as fit (at most `visible`), from the first one the
    /// state names; `‹ 2/5 ›` under them says where, and Left/Right scroll while it has focus.
    fn ui_carousel(&mut self, n: N<'_>, path: &str, w: usize) -> Lines {
        let kids = kids_of(n, "children", path);
        let count = kids.len();
        if count == 0 {
            return Vec::new();
        }
        let max = n.num("visible").filter(|v| *v >= 1.0).map(|v| v as usize);
        let cols = ((w + 1) / (GRID_MIN + 1))
            .max(1)
            .min(max.unwrap_or(usize::MAX))
            .min(count);
        let id = path_id(path);
        let first = self
            .ui_state()
            .and_then(|s| s.slides.get(&id))
            .copied()
            .unwrap_or(0)
            .min(count - 1);
        let shown = &kids[first..(first + cols).min(count)];
        let multiple = count > 1;
        if multiple {
            self.ui_target(
                &id,
                UiKind::Carousel { count },
                format!("{}/{count}", first + 1),
                Value::from(first),
            );
        }
        let mut out = self.grid_cols(shown, cols, w);
        if multiple {
            let focused = self.ui_focused(&id);
            let mut bar = self.carousel_bar(first, shown.len(), count, focused, w);
            if focused {
                self.mark_focus(&mut bar);
            }
            out.push(bar);
        }
        out
    }

    /// `‹ 2/5 ›`, or `‹ 2–3/5 ›` with several slides shown, centered; an arrow is lit when there
    /// is more on that side. Reversed while the carousel has focus.
    fn carousel_bar(
        &self,
        first: usize,
        shown: usize,
        count: usize,
        focused: bool,
        w: usize,
    ) -> Line<'static> {
        let at = if shown > 1 {
            format!("{}–{}", first + 1, first + shown)
        } else {
            format!("{}", first + 1)
        };
        let lit = Style::new().fg(self.p.accent).add_modifier(Modifier::BOLD);
        let arrow = |on: bool| if on { lit } else { self.p.muted_style() };
        let middle = if focused {
            lit.add_modifier(Modifier::REVERSED)
        } else {
            self.p.muted_style()
        };
        let text = format!(" {at}/{count} ");
        let pad = w.saturating_sub(width(&text) + 2) / 2;
        Line::from(vec![
            Span::raw(" ".repeat(pad)),
            Span::styled("‹", arrow(first > 0)),
            Span::styled(text, middle),
            Span::styled("›", arrow(first + shown < count)),
        ])
    }

    fn grid_cell(&mut self, k: &Kid<'_>, w: usize, height: usize) -> Lines {
        if k.0.name() == "Card" {
            self.ui_card_h(k.0, &k.1, w, height)
        } else {
            self.ui_node(k.0, &k.1, w)
        }
    }

    /// The narrowest a node is drawn in. Text breaks at words and containers give way; what
    /// can't (a metric, a tag, a button) keeps its width.
    fn ui_min(&mut self, n: N<'_>, pref: usize) -> usize {
        let word = |s: &str| s.split_whitespace().map(width).max().unwrap_or(0);
        let min = match n.name() {
            "Metric" | "MetricIndicator" | "Tag" | "Link" | "Button" | "FollowUpItem" => pref,
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
        let kids = n.nodes("children");
        if kids.len() >= 2 && kids.iter().all(|k| k.name() == "Metric") {
            return self.ui_metric_strip(&kids, w);
        }
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

    fn title_style(&self, tone: Option<pb::Tone>) -> Style {
        let st = Style::new().add_modifier(Modifier::BOLD);
        match tone {
            Some(t) => st.fg(self.p.tone(t)),
            None => st,
        }
    }

    /// A card at least `min_h` lines tall (cards of a grid row are stretched to the tallest).
    /// `card` (the default) floats: a rounded outline in the tone (or the border color), the
    /// title in the top edge, a surface-colored fill and one cell of padding inside. `sunk`
    /// is the fill without the outline, `clear` neither.
    pub(super) fn ui_card_h(&mut self, n: N<'_>, path: &str, w: usize, min_h: usize) -> Lines {
        let title = n.line("title");
        let tone = n.tone();
        let variant = n.str("variant");
        let boxed = !matches!(variant, Some("clear" | "sunk")) && w >= 6;
        if variant == Some("sunk") {
            let pad = usize::from(w >= 4);
            let inner = w - 2 * pad;
            let mut body = self.ui_children(n, "children", path, inner, None);
            if let Some(t) = &title {
                body.insert(0, sline(truncate(t, inner), self.title_style(tone)));
            }
            grow(&mut body, min_h);
            return body
                .into_iter()
                .map(|line| self.surface_row(line, w, pad))
                .collect();
        }
        if !boxed {
            let mut out = Vec::new();
            if let Some(t) = &title {
                out.push(sline(truncate(t, w), self.title_style(tone)));
            }
            out.extend(self.ui_children(n, "children", path, w, None));
            grow(&mut out, min_h);
            return out;
        }
        let inner = w - 4;
        let mut body = self.ui_children(n, "children", path, inner, None);
        grow(&mut body, min_h.saturating_sub(2));
        let edge = Style::new().fg(tone.map_or(self.p.border, |t| self.p.tone(t)));
        let mut out = Vec::with_capacity(body.len() + 2);
        let title = title.map(|t| truncate(&t, w - 6)).filter(|t| !t.is_empty());
        let mut top = Vec::new();
        match title {
            Some(t) => {
                let rest = w - width(&t) - 5;
                top.push(Span::styled("╭─ ", edge));
                top.push(Span::styled(t, self.title_style(tone)));
                top.push(Span::styled(format!(" {}╮", "─".repeat(rest)), edge));
            }
            None => top.push(Span::styled(format!("╭{}╮", "─".repeat(w - 2)), edge)),
        }
        out.push(self.fill(Line::from(top)));
        for line in body {
            let used = line.width();
            let mut spans = vec![Span::styled("│ ", edge)];
            spans.extend(line.spans);
            if used < inner {
                spans.push(Span::raw(" ".repeat(inner - used)));
            }
            spans.push(Span::styled(" │", edge));
            out.push(self.fill(Line::from(spans)));
        }
        let bottom = Span::styled(format!("╰{}╯", "─".repeat(w - 2)), edge);
        out.push(self.fill(Line::from(bottom)));
        out
    }

    /// The surface color behind every span that has no background of its own (a reversed
    /// span swaps its colors, so it keeps the terminal's).
    fn fill(&self, mut line: Line<'static>) -> Line<'static> {
        for s in &mut line.spans {
            if s.style.bg.is_none() && !s.style.add_modifier.contains(Modifier::REVERSED) {
                s.style = s.style.bg(self.p.surface);
            }
        }
        line
    }

    /// `line` between `pad` cells on each side, filled to `w` on the surface color.
    fn surface_row(&self, mut line: Line<'static>, w: usize, pad: usize) -> Line<'static> {
        line.spans.insert(0, Span::raw(" ".repeat(pad)));
        let used = line.width();
        if used < w {
            line.spans.push(Span::raw(" ".repeat(w - used)));
        }
        self.fill(line)
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
