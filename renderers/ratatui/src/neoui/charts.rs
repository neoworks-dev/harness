//! Charts, drawn by meaning rather than by pixels: eighth-block bars, braille lines and areas,
//! a stacked bar for pie and donut charts, block sparklines.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

use super::{N, number};
use crate::chart_image::{ChartKind, ChartSpec, SpecSeries};
use crate::palette::blend;
use crate::render::{Cx, Lines};
use crate::text::{push_text, truncate, width, wrap};

/// Heights of a column cell, one eighth to a whole.
const UP: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
/// Rows of a column chart and of a line chart's canvas.
const ROWS: usize = 6;
/// Rows of a chart drawn as a picture: the finer drawing takes a little more room.
const IMAGE_ROWS: usize = 8;
/// The most cells a bar or a stacked bar takes.
const MAX_BAR: usize = 48;
/// A line chart's canvas is wider: its points spread out and its slopes read better. Its height
/// grows with its width, one row for every [`CELLS_PER_ROW`] cells, from [`ROWS`] to
/// [`MAX_ROWS`], so a wide chart is not a flat strip.
const MAX_LINE_CANVAS: usize = 120;
const CELLS_PER_ROW: usize = 8;
const MAX_ROWS: usize = 12;
/// From this many rows up the value axis says the middle value too.
const MID_LABEL_ROWS: usize = 9;

struct Series {
    name: String,
    values: Vec<f64>,
}

fn series_of(n: N<'_>) -> Vec<Series> {
    n.nodes("series")
        .into_iter()
        .map(|s| Series {
            name: s.line("name").unwrap_or_default(),
            values: nums(s.prop("values")),
        })
        .collect()
}

pub(super) fn nums(v: Option<&Value>) -> Vec<f64> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|x| x.as_f64().filter(|f| f.is_finite()).unwrap_or(0.0))
                .collect()
        })
        .unwrap_or_default()
}

fn labels_of(n: N<'_>) -> Vec<String> {
    n.prop("labels")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(super::scalar).collect())
        .unwrap_or_default()
}

/// A true color as its bytes.
fn rgb(color: Color) -> Option<[u8; 3]> {
    match color {
        Color::Rgb(r, g, b) => Some([r, g, b]),
        _ => None,
    }
}

/// The series of a chart picture, in their colors.
fn spec_series(series: &[Series], colors: &[[u8; 3]]) -> Vec<SpecSeries> {
    series
        .iter()
        .zip(colors)
        .map(|(s, &color)| SpecSeries {
            color,
            values: s.values.clone(),
        })
        .collect()
}

/// `value` out of `max` as a count of `of` units; a value above zero is at least one.
fn scaled(value: f64, max: f64, of: usize) -> usize {
    if value <= 0.0 || max <= 0.0 {
        return 0;
    }
    (((value / max) * of as f64).round() as usize).clamp(1, of)
}

impl<'a> Cx<'a> {
    /// Series colors: the theme's accent, success and danger first, then mixes of them; a color
    /// the theme repeats (an accent that is its success green) once, so no two series look alike.
    pub(super) fn series_color(&self, i: usize) -> Color {
        let p = &self.p;
        let mixes = [
            blend(p.success, p.danger, 50),
            blend(p.accent, p.danger, 50),
            blend(p.accent, p.success, 50),
        ];
        let mut all = vec![p.accent, p.success];
        all.extend(mixes[0]);
        all.push(p.danger);
        all.extend(mixes[1]);
        all.extend(mixes[2]);
        all.push(p.muted);
        let mut distinct: Vec<Color> = Vec::new();
        for c in all {
            if !distinct.contains(&c) {
                distinct.push(c);
            }
        }
        distinct[i % distinct.len()]
    }

    /// `■ name  ■ name`, wrapped.
    fn legend(&self, items: &[(String, Color)], glyph: &str, w: usize) -> Lines {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (i, (name, color)) in items.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(format!("{glyph} "), Style::new().fg(*color)));
            spans.push(Span::raw(name.clone()));
        }
        wrap(&spans, w)
    }

    fn series_legend(&self, series: &[Series], glyph: &str, w: usize) -> Lines {
        let items: Vec<(String, Color)> = series
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name.clone(), self.series_color(i)))
            .collect();
        self.legend(&items, glyph, w)
    }

    /// Columns of eighth blocks, a value axis on the left and the labels under them; bars
    /// lying down (value at the end) when asked for or when the labels don't fit under columns.
    pub(super) fn ui_bar_chart(&mut self, n: N<'_>, w: usize) -> Lines {
        let series = series_of(n);
        let mut labels = labels_of(n);
        let count = series
            .iter()
            .map(|s| s.values.len())
            .chain([labels.len()])
            .max()
            .unwrap_or(0);
        if series.is_empty() || count == 0 {
            return Vec::new();
        }
        labels.resize(count, String::new());
        let stacked = n.bool("stacked") == Some(true);
        let value = |s: usize, i: usize| series[s].values.get(i).copied().unwrap_or(0.0).max(0.0);
        let total = |i: usize| -> f64 {
            if stacked {
                (0..series.len()).map(|s| value(s, i)).sum()
            } else {
                (0..series.len()).map(|s| value(s, i)).fold(0.0, f64::max)
            }
        };
        let max = (0..count).map(total).fold(0.0, f64::max);
        let columns = if n.bool("horizontal") == Some(true) {
            None
        } else {
            self.columns_chart(&series, &labels, stacked, max, w)
        };
        let mut out = columns.unwrap_or_else(|| self.bars_chart(&series, &labels, stacked, max, w));
        if series.len() > 1 {
            out.extend(self.series_legend(&series, "■", w));
        }
        out
    }

    /// None when the columns and their labels don't fit the width.
    fn columns_chart(
        &mut self,
        series: &[Series],
        labels: &[String],
        stacked: bool,
        max: f64,
        w: usize,
    ) -> Option<Lines> {
        let count = labels.len();
        let k = if stacked { 1 } else { series.len() };
        let axis = width(&number(max)).max(1);
        let label_w = labels.iter().map(|l| width(l)).max().unwrap_or(0).min(8);
        let fits =
            |bar: usize, label: usize| axis + 1 + count * (bar * k).max(label) + (count - 1) <= w;
        let (bar, label_w) = [3, 2, 1]
            .into_iter()
            .map(|b| (b, label_w))
            .chain([(1, label_w.min(3))])
            .find(|&(b, l)| fits(b, l))?;
        let group = (bar * k).max(label_w);
        let mut out: Lines = Vec::new();
        if let Some((colors, axis_color)) = self.chart_colors(series.len()) {
            let cols = (count * group + count - 1) as u16;
            let spec = ChartSpec {
                kind: ChartKind::Columns,
                cols,
                rows: IMAGE_ROWS as u16,
                series: spec_series(series, &colors),
                lo: 0.0,
                hi: max,
                group: group as u16,
                bar: bar as u16,
                stacked,
                donut: false,
                axis: axis_color,
            };
            let room = self.reserve_cells(&spec.src(), cols, IMAGE_ROWS as u16);
            for (r, line) in room.into_iter().enumerate() {
                let text = match r {
                    0 => number(max),
                    _ if r == IMAGE_ROWS - 1 => "0".to_string(),
                    _ => String::new(),
                };
                let mut spans = vec![Span::styled(
                    format!("{text:>axis$} "),
                    self.p.muted_style(),
                )];
                spans.extend(line.spans);
                out.push(Line::from(spans));
            }
            out.push(self.column_labels(labels, axis, group));
            return Some(out);
        }
        let unit = ROWS * 8;
        let value = |s: usize, i: usize| series[s].values.get(i).copied().unwrap_or(0.0).max(0.0);
        let eighths = |v: f64| scaled(v, max, unit);
        for r in 0..ROWS {
            let from_bottom = ROWS - 1 - r;
            let (lo, hi) = (from_bottom * 8, from_bottom * 8 + 8);
            let axis_text = if r == 0 {
                number(max)
            } else if r == ROWS - 1 {
                "0".to_string()
            } else {
                String::new()
            };
            let mut spans: Vec<Span<'static>> = vec![Span::styled(
                format!("{axis_text:>axis$} "),
                self.p.muted_style(),
            )];
            for i in 0..count {
                if i > 0 {
                    spans.push(Span::raw(" "));
                }
                let mut cells: Vec<(char, Option<usize>)> = Vec::new();
                if stacked {
                    let mut edges = Vec::new();
                    let mut acc = 0.0;
                    for s in 0..series.len() {
                        acc += value(s, i);
                        edges.push(eighths(acc).min(unit));
                    }
                    let top = edges.last().copied().unwrap_or(0);
                    let owner_of = |at: usize| edges.iter().position(|&e| e > at).unwrap_or(0);
                    let cell = if top <= lo {
                        (' ', None)
                    } else if top < hi {
                        (UP[top - lo - 1], Some(owner_of(top - 1)))
                    } else {
                        // The series that covers most of this row.
                        let mut best = (0, 0);
                        let mut start = 0;
                        for (s, &e) in edges.iter().enumerate() {
                            let covered = e.min(hi).saturating_sub(start.max(lo));
                            if covered > best.1 {
                                best = (s, covered);
                            }
                            start = e;
                        }
                        ('█', Some(best.0))
                    };
                    cells.push(cell);
                } else {
                    for s in 0..series.len() {
                        let e = eighths(value(s, i));
                        cells.push(if e >= hi {
                            ('█', Some(s))
                        } else if e > lo {
                            (UP[e - lo - 1], Some(s))
                        } else {
                            (' ', None)
                        });
                    }
                }
                let mut used = 0;
                for (ch, owner) in cells {
                    let text: String = std::iter::repeat_n(ch, bar).collect();
                    used += bar;
                    let style =
                        owner.map_or(Style::new(), |s| Style::new().fg(self.series_color(s)));
                    spans.push(Span::styled(text, style));
                }
                if used < group {
                    spans.push(Span::raw(" ".repeat(group - used)));
                }
            }
            out.push(Line::from(spans));
        }
        out.push(self.column_labels(labels, axis, group));
        Some(out)
    }

    /// Each label in the middle of its group of `group` cells, a cell between groups.
    fn column_labels(&self, labels: &[String], axis: usize, group: usize) -> Line<'static> {
        let mut spans = vec![Span::raw(" ".repeat(axis + 1))];
        for (i, label) in labels.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(" "));
            }
            let text = truncate(label, group);
            let room = group - width(&text);
            let left = room / 2;
            spans.push(Span::styled(
                format!("{}{text}{}", " ".repeat(left), " ".repeat(room - left)),
                self.p.muted_style(),
            ));
        }
        Line::from(spans)
    }

    /// One row per bar: `label ▄▄▄▄▖ 12`; stacked, one row per label with the segments.
    fn bars_chart(
        &self,
        series: &[Series],
        labels: &[String],
        stacked: bool,
        max: f64,
        w: usize,
    ) -> Lines {
        let count = labels.len();
        let value = |s: usize, i: usize| series[s].values.get(i).copied().unwrap_or(0.0).max(0.0);
        let label_w = labels
            .iter()
            .map(|l| width(l))
            .max()
            .unwrap_or(0)
            .min((w / 3).max(1));
        let shown = |v: f64| number(v);
        let value_w = (0..count)
            .flat_map(|i| {
                if stacked {
                    vec![(0..series.len()).map(|s| value(s, i)).sum::<f64>()]
                } else {
                    (0..series.len()).map(|s| value(s, i)).collect()
                }
            })
            .map(|v| width(&shown(v)))
            .max()
            .unwrap_or(1);
        let lead = if label_w > 0 { label_w + 1 } else { 0 };
        let bar_w = w.saturating_sub(lead + 1 + value_w).clamp(1, MAX_BAR);
        let unit = bar_w * 2;
        let head = |i: usize, first: bool| -> Span<'static> {
            if label_w == 0 {
                return Span::raw("");
            }
            let text = if first {
                truncate(&labels[i], label_w)
            } else {
                String::new()
            };
            Span::styled(format!("{text:<label_w$} "), self.p.muted_style())
        };
        let mut out = Vec::new();
        for i in 0..count {
            if stacked {
                let mut spans = vec![head(i, true)];
                let mut acc = 0.0;
                let mut done = 0;
                for s in 0..series.len() {
                    acc += value(s, i);
                    let edge = scaled(acc, max, unit);
                    if edge > done {
                        spans.push(bar_cells(edge - done, self.series_color(s)));
                        done = edge;
                    }
                }
                spans.push(Span::raw(format!(" {}", shown(acc))));
                out.push(Line::from(spans));
            } else {
                for s in 0..series.len() {
                    let v = value(s, i);
                    out.push(Line::from(vec![
                        head(i, s == 0),
                        bar_cells(scaled(v, max, unit), self.series_color(s)),
                        Span::raw(format!(" {}", shown(v))),
                    ]));
                }
            }
        }
        out
    }

    /// Lines (or filled areas) on a braille canvas with a value axis, the first and last
    /// labels under it, and a legend.
    pub(super) fn ui_line_chart(&mut self, n: N<'_>, area: bool, w: usize) -> Lines {
        let series = series_of(n);
        let labels = labels_of(n);
        let count = series
            .iter()
            .map(|s| s.values.len())
            .chain([labels.len()])
            .max()
            .unwrap_or(0);
        if series.is_empty() || count == 0 {
            return Vec::new();
        }
        let all = series.iter().flat_map(|s| s.values.iter().copied());
        let (mut lo, mut hi) = all.fold((f64::MAX, f64::MIN), |(l, h), v| (l.min(v), h.max(v)));
        if lo > hi {
            (lo, hi) = (0.0, 1.0);
        }
        if area {
            lo = lo.min(0.0);
        }
        if (hi - lo).abs() < f64::EPSILON {
            hi = lo + 1.0;
        }
        let ends = width(&number(hi)).max(width(&number(lo)));
        let text_rows = (w.saturating_sub(ends + 1) / CELLS_PER_ROW).clamp(ROWS, MAX_ROWS);
        let image_rows = text_rows.max(IMAGE_ROWS);
        let mid = lo + (hi - lo) / 2.0;
        let axis = if text_rows.max(image_rows) >= MID_LABEL_ROWS {
            ends.max(width(&number(mid)))
        } else {
            ends
        };
        let cells = w.saturating_sub(axis + 1).clamp(1, MAX_LINE_CANVAS);
        let gw = cells * 2;
        let at = |i: usize| -> usize {
            if count == 1 {
                gw / 2
            } else {
                (i * (gw - 1) + (count - 1) / 2) / (count - 1)
            }
        };
        let (rows, plot) = match self.chart_colors(series.len()) {
            Some((colors, axis_color)) => {
                let spec = ChartSpec {
                    kind: if area {
                        ChartKind::Area
                    } else {
                        ChartKind::Line
                    },
                    cols: cells as u16,
                    rows: image_rows as u16,
                    series: spec_series(&series, &colors),
                    lo,
                    hi,
                    group: 0,
                    bar: 0,
                    stacked: false,
                    donut: false,
                    axis: axis_color,
                };
                let room = self.reserve_cells(&spec.src(), cells as u16, image_rows as u16);
                (image_rows, room.into_iter().map(|l| l.spans).collect())
            }
            None => (
                text_rows,
                self.braille_plot(&series, area, (lo, hi), (cells, text_rows), &at),
            ),
        };
        // The middle value too once there is room between the ends, on the row it is drawn in.
        let mid_row = (rows >= MID_LABEL_ROWS).then(|| (rows * 4 - 1) / 2 / 4);
        let mut out: Lines = Vec::new();
        for (r, row) in plot.into_iter().enumerate() {
            let text = if r == 0 {
                number(hi)
            } else if r == rows - 1 {
                number(lo)
            } else if Some(r) == mid_row {
                number(mid)
            } else {
                String::new()
            };
            let mut spans = vec![Span::styled(
                format!("{text:>axis$} "),
                self.p.muted_style(),
            )];
            spans.extend(row);
            out.push(Line::from(spans));
        }
        self.x_labels(&mut out, &labels, count, axis, cells, &at);
        out.extend(self.series_legend(&series, "━", w));
        out
    }

    /// Whether charts are pictures here ([`crate::ImageOptions::charts`], not while the doc
    /// streams in), and then the colors of `n` series and of the base line.
    fn chart_colors(&self, n: usize) -> Option<(Vec<[u8; 3]>, [u8; 3])> {
        let on = self.o.images.as_ref().is_some_and(|i| i.charts) && !self.ui.streaming;
        if !on {
            return None;
        }
        let colors = (0..n)
            .map(|i| rgb(self.series_color(i)))
            .collect::<Option<Vec<_>>>()?;
        Some((colors, rgb(self.p.border)?))
    }

    /// The lines (or areas) on a braille canvas `cells` wide, `rows` high.
    fn braille_plot(
        &self,
        series: &[Series],
        area: bool,
        (lo, hi): (f64, f64),
        (cells, rows): (usize, usize),
        at: &dyn Fn(usize) -> usize,
    ) -> Vec<Vec<Span<'static>>> {
        let (gw, gh) = (cells * 2, rows * 4);
        let mut grid = Braille::new(cells, rows);
        let level = |v: f64| -> usize {
            let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
            gh - 1 - (t * (gh - 1) as f64).round() as usize
        };
        // Areas: the first series on top.
        let order: Vec<usize> = if area {
            (0..series.len()).rev().collect()
        } else {
            (0..series.len()).collect()
        };
        for s in order {
            let pts: Vec<(usize, usize)> = series[s]
                .values
                .iter()
                .enumerate()
                .map(|(i, &v)| (at(i), level(v)))
                .collect();
            let color = self.series_color(s);
            if pts.len() == 1 {
                grid.dot(pts[0].0, pts[0].1, color);
            }
            for pair in pts.windows(2) {
                grid.line(pair[0], pair[1], color);
            }
            if area {
                // Everything under the line, column by column.
                let mut tops = vec![gh; gw];
                for pair in pts.windows(2) {
                    let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
                    for (x, top) in tops.iter_mut().enumerate().take(x1 + 1).skip(x0) {
                        let t = if x1 == x0 {
                            0.0
                        } else {
                            (x - x0) as f64 / (x1 - x0) as f64
                        };
                        let y = (y0 as f64 + (y1 as f64 - y0 as f64) * t).round() as usize;
                        *top = (*top).min(y);
                    }
                }
                if let [(x, y)] = pts[..] {
                    tops[x] = y;
                }
                for (x, &top) in tops.iter().enumerate() {
                    for y in top..gh {
                        grid.dot(x, y, color);
                    }
                }
            }
        }
        grid.rows()
    }

    /// The labels under a canvas `cells` wide past the axis: the first and the last, then those
    /// in between that fit, each under its value.
    fn x_labels(
        &self,
        out: &mut Lines,
        labels: &[String],
        count: usize,
        axis: usize,
        cells: usize,
        at: &dyn Fn(usize) -> usize,
    ) {
        if labels.iter().any(|l| !l.is_empty()) {
            let xs: Vec<usize> = (0..labels.len().min(count)).map(|i| at(i) / 2).collect();
            let mut row = vec![' '; cells];
            let mut taken: Vec<(usize, usize)> = Vec::new();
            let mut order: Vec<usize> = vec![0, labels.len() - 1];
            order.dedup();
            order.extend(1..labels.len().saturating_sub(1));
            for j in order {
                let text = truncate(&labels[j], cells);
                let lw = width(&text);
                if lw == 0 {
                    continue;
                }
                let start = if j == 0 {
                    0
                } else if j == labels.len() - 1 {
                    cells.saturating_sub(lw)
                } else {
                    xs[j].saturating_sub(lw / 2).min(cells.saturating_sub(lw))
                };
                let end = start + lw;
                if end > cells || taken.iter().any(|&(a, b)| start < b + 1 && a < end + 1) {
                    continue;
                }
                taken.push((start, end));
                let mut col = start;
                for ch in text.chars() {
                    if col < cells {
                        row[col] = ch;
                    }
                    col += unicode_width::UnicodeWidthChar::width(ch)
                        .unwrap_or(0)
                        .max(1);
                }
            }
            let text: String = row.into_iter().collect();
            out.push(Line::from(vec![
                Span::raw(" ".repeat(axis + 1)),
                Span::styled(text, self.p.muted_style()),
            ]));
        }
    }

    /// One stacked bar of the slices, then `■ label   42%` for each.
    pub(super) fn ui_pie_chart(&mut self, n: N<'_>, w: usize) -> Lines {
        let slices: Vec<(String, f64)> = n
            .nodes("slices")
            .into_iter()
            .map(|s| {
                (
                    s.line("label").unwrap_or_default(),
                    s.num("value")
                        .filter(|v| v.is_finite())
                        .unwrap_or(0.0)
                        .max(0.0),
                )
            })
            .collect();
        let total: f64 = slices.iter().map(|s| s.1).sum();
        if slices.is_empty() || total <= 0.0 {
            return Vec::new();
        }
        if let Some(out) = self.pie_picture(n, &slices, total, w) {
            return out;
        }
        let bar_w = w.min(MAX_BAR);
        // Largest remainder; every slice with something in it gets a cell.
        let exact: Vec<f64> = slices.iter().map(|s| s.1 / total * bar_w as f64).collect();
        let mut cells: Vec<usize> = exact.iter().map(|e| *e as usize).collect();
        let mut left = bar_w.saturating_sub(cells.iter().sum());
        let mut by_rest: Vec<usize> = (0..slices.len()).collect();
        by_rest.sort_by(|&a, &b| {
            (exact[b] - exact[b].floor())
                .partial_cmp(&(exact[a] - exact[a].floor()))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for i in by_rest {
            if left == 0 {
                break;
            }
            cells[i] += 1;
            left -= 1;
        }
        for i in 0..cells.len() {
            if cells[i] == 0
                && slices[i].1 > 0.0
                && let Some(big) = (0..cells.len())
                    .max_by_key(|&j| cells[j])
                    .filter(|&j| cells[j] > 1)
            {
                cells[big] -= 1;
                cells[i] = 1;
            }
        }
        let spans: Vec<Span<'static>> = cells
            .iter()
            .enumerate()
            .filter(|(_, c)| **c > 0)
            .map(|(i, &c)| Span::styled("█".repeat(c), Style::new().fg(self.series_color(i))))
            .collect();
        let mut out = vec![Line::from(spans)];
        let label_w = slices
            .iter()
            .map(|s| width(&s.0))
            .max()
            .unwrap_or(0)
            .min(w.saturating_sub(8).max(1));
        for (i, (label, value)) in slices.iter().enumerate() {
            let pct = format!("{:>3}%", (value / total * 100.0).round() as u32);
            let text = truncate(label, label_w);
            let pad = " ".repeat(label_w - width(&text));
            out.push(Line::from(vec![
                Span::styled("■ ", Style::new().fg(self.series_color(i))),
                Span::raw(format!("{text}{pad}  ")),
                Span::styled(pct, self.p.muted_style()),
            ]));
        }
        out
    }

    /// The pie as a round picture, `■ label  42%` beside it for each slice (middled against
    /// it); None when charts are characters or the legend has no room.
    fn pie_picture(
        &mut self,
        n: N<'_>,
        slices: &[(String, f64)],
        total: f64,
        w: usize,
    ) -> Option<Lines> {
        let (colors, axis_color) = self.chart_colors(slices.len())?;
        let (cw, ch) = self.o.images.as_ref()?.cell;
        let rows = slices.len().clamp(5, IMAGE_ROWS);
        // As many cells across as make it round.
        let cols = ((rows * usize::from(ch.max(1))) as f64 / f64::from(cw.max(1))).round() as usize;
        let label_w = slices.iter().map(|s| width(&s.0)).max().unwrap_or(0);
        let label_w = label_w.min(w.saturating_sub(cols + 2 + 8));
        if label_w == 0 {
            return None;
        }
        let spec = ChartSpec {
            kind: ChartKind::Pie,
            cols: cols as u16,
            rows: rows as u16,
            series: colors
                .iter()
                .zip(slices)
                .map(|(&color, s)| SpecSeries {
                    color,
                    values: vec![s.1],
                })
                .collect(),
            lo: 0.0,
            hi: 0.0,
            group: 0,
            bar: 0,
            stacked: false,
            donut: n.bool("donut") == Some(true),
            axis: axis_color,
        };
        let entry = |i: usize| -> Vec<Span<'static>> {
            let (label, value) = &slices[i];
            let pct = format!("{:>3}%", (value / total * 100.0).round() as u32);
            let text = truncate(label, label_w);
            let pad = " ".repeat(label_w - width(&text));
            vec![
                Span::raw("  "),
                Span::styled("■ ", Style::new().fg(self.series_color(i))),
                Span::raw(format!("{text}{pad}  ")),
                Span::styled(pct, self.p.muted_style()),
            ]
        };
        let entries: Vec<Vec<Span<'static>>> = (0..slices.len()).map(entry).collect();
        let entry = |i: usize| entries[i].clone();
        let first = rows.saturating_sub(slices.len()) / 2;
        let room = self.reserve_cells(&spec.src(), cols as u16, rows as u16);
        let mut out: Lines = Vec::new();
        for (r, line) in room.into_iter().enumerate() {
            let mut spans = line.spans;
            if let Some(i) = r.checked_sub(first).filter(|&i| i < slices.len()) {
                spans.push(Span::raw(" ".repeat(cols)));
                spans.extend(entry(i));
            }
            out.push(Line::from(spans));
        }
        // More slices than rows: the rest under the picture, in the legend's column.
        for i in rows.min(slices.len())..slices.len() {
            let mut spans = vec![Span::raw(" ".repeat(cols))];
            spans.extend(entry(i));
            out.push(Line::from(spans));
        }
        Some(out)
    }

    /// `label ▁▂▃▅▇█ 25`: the shape of the values and the last one.
    pub(super) fn ui_sparkline(&mut self, n: N<'_>, w: usize) -> Lines {
        let values = nums(n.prop("values"));
        if values.is_empty() {
            return Vec::new();
        }
        let label = n.line("label");
        let last = number(*values.last().unwrap_or(&0.0));
        let lead = label.as_ref().map_or(0, |l| width(l) + 1);
        let tail = width(&last) + 1;
        let room = w.saturating_sub(lead + tail).max(1);
        // More values than room: each cell stands for a run of them (their mean).
        let cells: Vec<f64> = if values.len() > room {
            (0..room)
                .map(|c| {
                    let a = c * values.len() / room;
                    let b = ((c + 1) * values.len() / room).max(a + 1);
                    values[a..b].iter().sum::<f64>() / (b - a) as f64
                })
                .collect()
        } else {
            values.clone()
        };
        let (lo, hi) = cells
            .iter()
            .fold((f64::MAX, f64::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
        let text: String = cells
            .iter()
            .map(|v| {
                if (hi - lo).abs() < f64::EPSILON {
                    UP[3]
                } else {
                    UP[(((v - lo) / (hi - lo)) * 7.0).round() as usize]
                }
            })
            .collect();
        let mut spans: Vec<Span<'static>> = Vec::new();
        if let Some(l) = label {
            spans.push(Span::styled(format!("{l} "), self.p.muted_style()));
        }
        spans.push(Span::styled(text, Style::new().fg(self.p.accent)));
        spans.push(Span::raw(format!(" {last}")));
        vec![Line::from(spans)]
    }
}

/// A lying bar of `halves` half cells, half a line tall: the upper half of its line stays
/// empty, so bars on neighbouring lines never touch.
fn bar_cells(halves: usize, color: Color) -> Span<'static> {
    let mut text = "▄".repeat(halves / 2);
    if halves % 2 == 1 {
        text.push('▖');
    }
    Span::styled(text, Style::new().fg(color))
}

/// A canvas of braille cells (2x4 dots each), every cell in the color of what drew last.
pub(super) struct Braille {
    cols: usize,
    bits: Vec<Vec<u8>>,
    color: Vec<Vec<Option<Color>>>,
}

impl Braille {
    pub(super) fn new(cols: usize, rows: usize) -> Braille {
        Braille {
            cols,
            bits: vec![vec![0; cols]; rows],
            color: vec![vec![None; cols]; rows],
        }
    }

    pub(super) fn dot(&mut self, x: usize, y: usize, color: Color) {
        let (cx, cy) = (x / 2, y / 4);
        if cx >= self.cols || cy >= self.bits.len() {
            return;
        }
        // Left column top to bottom, then the right one; the bottom row of each last.
        let bit = match (x % 2, y % 4) {
            (0, 0) => 0x01,
            (0, 1) => 0x02,
            (0, 2) => 0x04,
            (0, _) => 0x40,
            (_, 0) => 0x08,
            (_, 1) => 0x10,
            (_, 2) => 0x20,
            (_, _) => 0x80,
        };
        self.bits[cy][cx] |= bit;
        self.color[cy][cx] = Some(color);
    }

    /// Bresenham.
    pub(super) fn line(&mut self, from: (usize, usize), to: (usize, usize), color: Color) {
        let (mut x, mut y) = (from.0 as i64, from.1 as i64);
        let (x1, y1) = (to.0 as i64, to.1 as i64);
        let (dx, dy) = ((x1 - x).abs(), -(y1 - y).abs());
        let (sx, sy) = (if x < x1 { 1 } else { -1 }, if y < y1 { 1 } else { -1 });
        let mut err = dx + dy;
        loop {
            self.dot(x as usize, y as usize, color);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Like [`Braille::rows`], but a cell drawn all over is a block: whole, or a half of it.
    pub(super) fn block_rows(&self) -> Vec<Vec<Span<'static>>> {
        self.cells(|bits| match bits {
            0xff => '█',
            // The upper two dots of each column, the lower two, the left column, the right one.
            0x1b => '▀',
            0xe4 => '▄',
            0x47 => '▌',
            0xb8 => '▐',
            b => char::from_u32(0x2800 + u32::from(b)).unwrap_or(' '),
        })
    }

    fn rows(&self) -> Vec<Vec<Span<'static>>> {
        self.cells(|b| char::from_u32(0x2800 + u32::from(b)).unwrap_or(' '))
    }

    /// Each row as spans, a cell with nothing drawn a space and the rest `glyph` of their dots.
    fn cells(&self, glyph: impl Fn(u8) -> char) -> Vec<Vec<Span<'static>>> {
        self.bits
            .iter()
            .zip(&self.color)
            .map(|(bits, colors)| {
                let mut spans: Vec<Span<'static>> = Vec::new();
                for (b, c) in bits.iter().zip(colors) {
                    let ch = if *b == 0 { ' ' } else { glyph(*b) };
                    let style = c.map_or(Style::new(), |c| Style::new().fg(c));
                    push_text(&mut spans, &ch.to_string(), style);
                }
                spans
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Options;

    #[test]
    fn a_theme_whose_accent_is_its_success_still_gives_the_first_series_their_own_colors() {
        let mut o = Options::default();
        o.theme.success = o.theme.accent;
        let cx = Cx::new(&o);
        // Accent, its mix with danger, danger, muted; then round again.
        let colors: Vec<Color> = (0..4).map(|i| cx.series_color(i)).collect();
        for (i, a) in colors.iter().enumerate() {
            assert!(!colors[i + 1..].contains(a), "{colors:?}");
        }
    }
}
