//! The generic components: `Box`, a column or row with padding, a tone fill and a tone border
//! whose width is a share of its parent's, and `Shape`, a rect, line, path or arc in a box of
//! 0 to 1 drawn on a braille canvas (cells covered all over are blocks).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

use super::charts::{Braille, nums};
use super::layout::max_width;
use super::{N, tone_of};
use crate::render::{Cx, Lines};

/// The most rows a canvas takes, whatever its aspect.
const MAX_ROWS: usize = 16;

/// What a `Box` asks of the width its parent has.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Ask {
    /// What the siblings leave (all of a column).
    Fill,
    /// The width of its content.
    Fit,
    /// A share of the parent's width, 0 to 1.
    Share(f64),
}

/// The `width` of a `Box`; None when it has none (a column fills, a row fits).
pub(super) fn ask(n: N<'_>) -> Option<Ask> {
    match n.prop("width")? {
        Value::String(s) if s == "fill" => Some(Ask::Fill),
        Value::String(s) if s == "fit" => Some(Ask::Fit),
        Value::Number(x) => x
            .as_f64()
            .filter(|s| s.is_finite() && *s > 0.0)
            .map(|s| Ask::Share(s.min(1.0))),
        _ => None,
    }
}

/// `share` of `room` cells, at least one.
pub(super) fn share_of(share: f64, room: usize) -> usize {
    ((room as f64 * share).round() as usize).clamp(1, room.max(1))
}

/// A box's padding in cells, across and down.
fn pad(n: N<'_>) -> (usize, usize) {
    match n.str("pad") {
        Some("s") => (1, 0),
        Some("m") => (2, 0),
        Some("l") => (3, 1),
        _ => (0, 0),
    }
}

impl<'a> Cx<'a> {
    /// `n` at the width it asks of a column: a share of it, its content's, else all of it.
    pub(super) fn ui_sized(&mut self, n: N<'_>, path: &str, w: usize) -> Lines {
        if n.name() == "Box" {
            let narrower = match ask(n) {
                Some(Ask::Share(s)) => share_of(s, w),
                Some(Ask::Fit) => self.quiet(|cx| cx.box_natural(n, path, w)),
                _ => w,
            };
            return self.ui_node(n, path, narrower);
        }
        self.ui_node(n, path, w)
    }

    /// Whether the frame is drawn, the padding across and down, and the width inside both.
    fn box_metrics(n: N<'_>, w: usize) -> (bool, usize, usize, usize) {
        let framed = n.str("border").and_then(|t| tone_of(Some(t))).is_some() && w >= 4;
        let frame = usize::from(framed);
        let (px, py) = pad(n);
        let px = px.min(w.saturating_sub(2 * frame + 1) / 2);
        (framed, px, py, w.saturating_sub(2 * (frame + px)).max(1))
    }

    /// The children of a box, in a column or a row.
    fn box_body(&mut self, n: N<'_>, path: &str, inner: usize) -> Lines {
        let gap = n.str("gap").or(Some("s"));
        if n.str("dir") == Some("row") {
            self.ui_row(n, path, inner, gap)
        } else {
            self.ui_column(n, path, inner, gap)
        }
    }

    /// How wide a box is when it only takes what its content needs.
    pub(super) fn box_natural(&mut self, n: N<'_>, path: &str, w: usize) -> usize {
        let (_, _, _, inner) = Self::box_metrics(n, w);
        let body = self.box_body(n, path, inner);
        (max_width(&body) + w.saturating_sub(inner)).clamp(1, w.max(1))
    }

    /// A box drawn `w` cells wide: the frame in the border's tone, padding, and a wash of the
    /// fill's tone behind everything. Without any of them it is just its children.
    pub(super) fn ui_box(&mut self, n: N<'_>, path: &str, w: usize) -> Lines {
        let (framed, px, py, inner) = Self::box_metrics(n, w);
        let body = self.box_body(n, path, inner);
        let border = n.str("border").and_then(|t| tone_of(Some(t)));
        let fill = tone_of(n.str("fill"));
        if !framed && fill.is_none() && px == 0 && py == 0 {
            return body;
        }
        if body.is_empty() && !framed && fill.is_none() {
            return body;
        }
        let edge = Style::new().fg(border.map_or(self.p.border, |t| self.p.tone(t)));
        let mut inside: Lines = vec![Line::default(); py];
        inside.extend(body);
        inside.extend(vec![Line::default(); py]);
        let (tl, tr, bl, br) = if n.bool("rounded") == Some(true) {
            ('╭', '╮', '╰', '╯')
        } else {
            ('┌', '┐', '└', '┘')
        };
        let rule = "─".repeat(w.saturating_sub(2));
        let mut out: Lines = Vec::with_capacity(inside.len() + 2);
        if framed {
            out.push(Line::from(Span::styled(format!("{tl}{rule}{tr}"), edge)));
        }
        for line in inside {
            let used = line.width();
            let mut spans: Vec<Span<'static>> = Vec::new();
            if framed {
                spans.push(Span::styled("│", edge));
            }
            if px > 0 {
                spans.push(Span::raw(" ".repeat(px)));
            }
            spans.extend(line.spans);
            let spare = inner.saturating_sub(used) + px;
            if spare > 0 {
                spans.push(Span::raw(" ".repeat(spare)));
            }
            if framed {
                spans.push(Span::styled("│", edge));
            }
            out.push(Line::from(spans));
        }
        if framed {
            out.push(Line::from(Span::styled(format!("{bl}{rule}{br}"), edge)));
        }
        if let Some(tone) = fill {
            let bg = self.p.tint(Some(self.p.tone(tone)));
            for line in &mut out {
                for s in &mut line.spans {
                    if s.style.bg.is_none() && !s.style.add_modifier.contains(Modifier::REVERSED) {
                        s.style = s.style.bg(bg);
                    }
                }
            }
        }
        out
    }

    /// Shapes that follow each other, drawn over each other in one box `w` cells wide whose
    /// height the first one's `aspect` (width over height, default 2) gives.
    pub(super) fn ui_canvas(&mut self, shapes: &[N<'_>], w: usize) -> Lines {
        let Some(first) = shapes.first() else {
            return Vec::new();
        };
        if w == 0 {
            return Vec::new();
        }
        let aspect = first
            .num("aspect")
            .filter(|a| a.is_finite() && *a > 0.0)
            .unwrap_or(2.0);
        // A dot is as wide as it is tall: two to a cell across, four down.
        let dots_w = w * 2;
        let rows = ((dots_w as f64 / aspect / 4.0).ceil() as usize).clamp(1, MAX_ROWS);
        let mut canvas = Braille::new(w, rows);
        let mut raster = Raster {
            canvas: &mut canvas,
            w: dots_w,
            h: rows * 4,
        };
        for shape in shapes {
            let color = shape.tone().map_or(self.p.fg, |t| self.p.tone(t));
            let points = nums(shape.prop("points"));
            let fill = shape.bool("fill") == Some(true);
            raster.draw(shape.str("kind").unwrap_or(""), &points, fill, color);
        }
        canvas.block_rows().into_iter().map(Line::from).collect()
    }
}

/// A canvas of `w` by `h` dots that shapes in a box of 0 to 1 are drawn on.
struct Raster<'c> {
    canvas: &'c mut Braille,
    w: usize,
    h: usize,
}

type Dot = (i64, i64);

impl Raster<'_> {
    /// The dot a point of the box is at, held inside the canvas.
    fn at(&self, x: f64, y: f64) -> Dot {
        let to = |v: f64, size: usize| (v.clamp(0.0, 1.0) * size.saturating_sub(1) as f64).round();
        (to(x, self.w) as i64, to(y, self.h) as i64)
    }

    fn dot(&mut self, (x, y): Dot, color: Color) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.canvas.dot(x as usize, y as usize, color);
        }
    }

    fn line(&mut self, from: Dot, to: Dot, color: Color) {
        let held = |(x, y): Dot| {
            (
                x.clamp(0, self.w as i64 - 1) as usize,
                y.clamp(0, self.h as i64 - 1) as usize,
            )
        };
        let (from, to) = (held(from), held(to));
        self.canvas.line(from, to, color);
    }

    fn draw(&mut self, kind: &str, p: &[f64], fill: bool, color: Color) {
        match kind {
            "rect" if p.len() >= 4 => self.rect(p[0], p[1], p[2], p[3], fill, color),
            "line" if p.len() >= 2 => {
                let pts = self.pairs(p);
                match pts.as_slice() {
                    [only] => self.dot(*only, color),
                    _ => pts.windows(2).for_each(|s| self.line(s[0], s[1], color)),
                }
            }
            "path" if p.len() >= 4 => self.polygon(&self.pairs(p), fill, color),
            "arc" if p.len() >= 5 => self.arc(p, fill, color),
            _ => {}
        }
    }

    fn pairs(&self, p: &[f64]) -> Vec<Dot> {
        p.as_chunks::<2>()
            .0
            .iter()
            .map(|[x, y]| self.at(*x, *y))
            .collect()
    }

    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, fill: bool, color: Color) {
        // The dots it covers: half the box is half the dots, and none is thinner than one.
        let span = |from: f64, len: f64, size: usize| {
            let start = (from.clamp(0.0, 1.0) * size as f64).round() as i64;
            let end = ((from + len).clamp(0.0, 1.0) * size as f64).round() as i64;
            let last = size as i64 - 1;
            (start.min(last), (end - 1).max(start).min(last))
        };
        let (x0, x1) = span(x, w, self.w);
        let (y0, y1) = span(y, h, self.h);
        if fill {
            for yy in y0..=y1 {
                for xx in x0..=x1 {
                    self.dot((xx, yy), color);
                }
            }
        } else {
            self.polygon(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)], false, color);
        }
    }

    /// A closed shape through the corners, with its inside filled when asked (even-odd).
    fn polygon(&mut self, pts: &[Dot], fill: bool, color: Color) {
        let n = pts.len();
        if fill && n >= 3 {
            let (top, bottom) = pts.iter().fold((i64::MAX, i64::MIN), |(lo, hi), p| {
                (lo.min(p.1), hi.max(p.1))
            });
            for y in top..=bottom {
                let mut xs: Vec<f64> = Vec::new();
                for i in 0..n {
                    let (a, b) = (pts[i], pts[(i + 1) % n]);
                    if (a.1 <= y) != (b.1 <= y) {
                        let t = (y - a.1) as f64 / (b.1 - a.1) as f64;
                        xs.push(a.0 as f64 + t * (b.0 - a.0) as f64);
                    }
                }
                xs.sort_by(f64::total_cmp);
                for [from, to] in xs.as_chunks::<2>().0 {
                    for x in from.ceil() as i64..=to.floor() as i64 {
                        self.dot((x, y), color);
                    }
                }
            }
        }
        for i in 0..n {
            self.line(pts[i], pts[(i + 1) % n], color);
        }
    }

    /// `cx, cy, r, from, to`: `r` a share of the shorter side, `from` and `to` turns clockwise
    /// from the top. Filled it is a wedge to the center.
    fn arc(&mut self, p: &[f64], fill: bool, color: Color) {
        let (cx, cy) = self.at(p[0], p[1]);
        let r = p[2].max(0.0) * self.w.min(self.h) as f64;
        let span = p[4] - p[3];
        let sweep = if span.abs() >= 1.0 {
            1.0
        } else {
            span.rem_euclid(1.0)
        };
        let tau = std::f64::consts::TAU;
        if fill {
            let (cxf, cyf) = (cx as f64, cy as f64);
            for y in (cyf - r).floor() as i64..=(cyf + r).ceil() as i64 {
                for x in (cxf - r).floor() as i64..=(cxf + r).ceil() as i64 {
                    let (dx, dy) = (x as f64 - cxf, y as f64 - cyf);
                    if dx.hypot(dy) > r {
                        continue;
                    }
                    let turn = (dx.atan2(-dy) / tau - p[3]).rem_euclid(1.0);
                    if sweep >= 1.0 || turn <= sweep {
                        self.dot((x, y), color);
                    }
                }
            }
        }
        let steps = ((tau * r * sweep * 1.5).ceil() as usize).max(8);
        let mut last: Option<Dot> = None;
        for i in 0..=steps {
            let t = (p[3] + sweep * i as f64 / steps as f64) * tau;
            let at = (
                (cx as f64 + r * t.sin()).round() as i64,
                (cy as f64 - r * t.cos()).round() as i64,
            );
            match last {
                Some(from) => self.line(from, at, color),
                None => self.dot(at, color),
            }
            last = Some(at);
        }
    }
}
