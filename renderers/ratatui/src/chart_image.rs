//! Charts as pictures ([`ImageOptions::charts`](crate::ImageOptions::charts)): the renderer
//! reserves room for a chart's plot under a `chart:` source that holds a [`ChartSpec`], and keeps
//! its axis, labels and legend as text around it; the app draws the spec with [`draw_chart`] at
//! the size of the cells, on a transparent background. Everything is laid out in cells, so the
//! picture lines up with the text the way the character chart does.

use serde::{Deserialize, Serialize};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

/// What a chart's source starts with; the spec as JSON follows.
pub const CHART_SCHEME: &str = "chart:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChartKind {
    Line,
    Area,
    Columns,
    Pie,
}

/// One series (one slice of a pie): its color and values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecSeries {
    pub color: [u8; 3],
    pub values: Vec<f64>,
}

/// A chart's plot, in the cells it covers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartSpec {
    pub kind: ChartKind,
    pub cols: u16,
    pub rows: u16,
    pub series: Vec<SpecSeries>,
    /// The values at the bottom and the top (lines and areas; columns go from 0 to `hi`).
    #[serde(default)]
    pub lo: f64,
    #[serde(default)]
    pub hi: f64,
    /// Columns: the cells of each label's group (a cell between groups) and of each bar.
    #[serde(default)]
    pub group: u16,
    #[serde(default)]
    pub bar: u16,
    #[serde(default)]
    pub stacked: bool,
    #[serde(default)]
    pub donut: bool,
    /// The color of the base line under lines, areas and columns.
    pub axis: [u8; 3],
}

impl ChartSpec {
    /// The source the renderer reserves room under.
    pub fn src(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_default();
        format!("{CHART_SCHEME}{json}")
    }

    /// The spec in a source made by [`ChartSpec::src`].
    pub fn from_src(src: &str) -> Option<ChartSpec> {
        serde_json::from_str(src.strip_prefix(CHART_SCHEME)?).ok()
    }

    fn count(&self) -> usize {
        self.series
            .iter()
            .map(|s| s.values.len())
            .max()
            .unwrap_or(0)
    }
}

/// The chart drawn `width` × `height` pixels: RGBA, not premultiplied, transparent where nothing
/// is drawn. None for a size of nothing.
pub fn draw_chart(spec: &ChartSpec, width: u32, height: u32) -> Option<Vec<u8>> {
    let mut pixmap = Pixmap::new(width, height)?;
    let cell = (
        width as f32 / f32::from(spec.cols.max(1)),
        height as f32 / f32::from(spec.rows.max(1)),
    );
    let mut canvas = Canvas {
        pixmap: &mut pixmap,
        cell,
    };
    match spec.kind {
        ChartKind::Line | ChartKind::Area => canvas.lines(spec),
        ChartKind::Columns => canvas.columns(spec),
        ChartKind::Pie => canvas.pie(spec),
    }
    Some(
        pixmap
            .pixels()
            .iter()
            .flat_map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect(),
    )
}

fn paint(color: [u8; 3], alpha: f32) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(color[0], color[1], color[2], (alpha * 255.0).round() as u8);
    paint.anti_alias = true;
    paint
}

struct Canvas<'a> {
    pixmap: &'a mut Pixmap,
    /// A cell's size in pixels.
    cell: (f32, f32),
}

impl Canvas<'_> {
    fn size(&self) -> (f32, f32) {
        (self.pixmap.width() as f32, self.pixmap.height() as f32)
    }

    /// A hairline across the bottom.
    fn base(&mut self, color: [u8; 3]) {
        let (w, h) = self.size();
        if let Some(rect) = Rect::from_xywh(0.0, h - 1.0, w, 1.0) {
            self.pixmap
                .fill_rect(rect, &paint(color, 0.8), Transform::identity(), None);
        }
    }

    /// Each series a line through its values, areas filled to the bottom under it; the first
    /// series on top. x is the middle of the cell the text labels put the value under.
    fn lines(&mut self, spec: &ChartSpec) {
        let (w, h) = self.size();
        let count = spec.count();
        if count == 0 {
            return;
        }
        let stroke_w = (self.cell.1 * 0.12).clamp(1.5, 4.0);
        let pad = stroke_w;
        let (lo, hi) = (
            spec.lo,
            if spec.hi > spec.lo {
                spec.hi
            } else {
                spec.lo + 1.0
            },
        );
        let cols = f32::from(spec.cols.max(1));
        let x = |i: usize| -> f32 {
            if count == 1 {
                w / 2.0
            } else {
                (i as f32 * (cols - 1.0) / (count - 1) as f32 + 0.5) * self.cell.0
            }
        };
        let y = |v: f64| -> f32 {
            let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0) as f32;
            pad + (1.0 - t) * (h - 2.0 * pad)
        };
        let area = spec.kind == ChartKind::Area;
        for s in spec.series.iter().rev() {
            let pts: Vec<(f32, f32)> = s
                .values
                .iter()
                .enumerate()
                .map(|(i, &v)| (x(i), y(v)))
                .collect();
            let Some(&(x0, y0)) = pts.first() else {
                continue;
            };
            let mut line = PathBuilder::new();
            line.move_to(x0, y0);
            for &(px, py) in &pts[1..] {
                line.line_to(px, py);
            }
            if area {
                let mut fill = PathBuilder::new();
                fill.move_to(x0, h);
                for &(px, py) in &pts {
                    fill.line_to(px, py);
                }
                let last = pts[pts.len() - 1].0;
                fill.line_to(last, h);
                fill.close();
                if let Some(path) = fill.finish() {
                    self.pixmap.fill_path(
                        &path,
                        &paint(s.color, 0.22),
                        FillRule::Winding,
                        Transform::identity(),
                        None,
                    );
                }
            }
            if pts.len() == 1 {
                if let Some(dot) = PathBuilder::from_circle(x0, y0, stroke_w * 1.5) {
                    self.pixmap.fill_path(
                        &dot,
                        &paint(s.color, 1.0),
                        FillRule::Winding,
                        Transform::identity(),
                        None,
                    );
                }
                continue;
            }
            if let Some(path) = line.finish() {
                let stroke = Stroke {
                    width: stroke_w,
                    line_cap: LineCap::Round,
                    line_join: LineJoin::Round,
                    ..Stroke::default()
                };
                self.pixmap.stroke_path(
                    &path,
                    &paint(s.color, 1.0),
                    &stroke,
                    Transform::identity(),
                    None,
                );
            }
        }
        self.base(spec.axis);
    }

    /// For each label a group of `group` cells (a cell between groups) with its bars side by
    /// side, or one bar of stacked segments, in the middle of the group; rounded tops.
    fn columns(&mut self, spec: &ChartSpec) {
        let (_, h) = self.size();
        let count = spec.count();
        let max = spec.hi;
        if count == 0 || max <= 0.0 {
            return;
        }
        let (cw, top) = (self.cell.0, 1.0);
        let k = if spec.stacked { 1 } else { spec.series.len() };
        let bar_w = f32::from(spec.bar.max(1)) * cw;
        let inset = (bar_w * 0.12).min(cw * 0.25);
        let height = |v: f64| (v.max(0.0) / max).min(1.0) as f32 * (h - top);
        for i in 0..count {
            let group_x = i as f32 * f32::from(spec.group + 1) * cw;
            let left = group_x + (f32::from(spec.group) * cw - bar_w * k as f32) / 2.0;
            if spec.stacked {
                let mut bottom = h;
                let last = spec
                    .series
                    .iter()
                    .rposition(|s| s.values.get(i).copied().unwrap_or(0.0) > 0.0);
                for (s, series) in spec.series.iter().enumerate() {
                    let v = series.values.get(i).copied().unwrap_or(0.0);
                    let seg = height(v);
                    if seg <= 0.0 {
                        continue;
                    }
                    let rounded = Some(s) == last;
                    self.bar(
                        left + inset,
                        bottom - seg,
                        bar_w - 2.0 * inset,
                        seg,
                        series.color,
                        rounded,
                    );
                    bottom -= seg;
                }
            } else {
                for (s, series) in spec.series.iter().enumerate() {
                    let v = series.values.get(i).copied().unwrap_or(0.0);
                    let tall = height(v);
                    if tall <= 0.0 {
                        continue;
                    }
                    let x = left + s as f32 * bar_w + inset;
                    self.bar(x, h - tall, bar_w - 2.0 * inset, tall, series.color, true);
                }
            }
        }
        self.base(spec.axis);
    }

    fn bar(&mut self, x: f32, y: f32, w: f32, h: f32, color: [u8; 3], rounded: bool) {
        let r = if rounded {
            (w * 0.3).min(h).min(4.0)
        } else {
            0.0
        };
        let mut pb = PathBuilder::new();
        pb.move_to(x, y + h);
        pb.line_to(x, y + r);
        pb.quad_to(x, y, x + r, y);
        pb.line_to(x + w - r, y);
        pb.quad_to(x + w, y, x + w, y + r);
        pb.line_to(x + w, y + h);
        pb.close();
        if let Some(path) = pb.finish() {
            self.pixmap.fill_path(
                &path,
                &paint(color, 1.0),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    /// The slices clockwise from the top, a hair apart; a donut leaves the middle out.
    fn pie(&mut self, spec: &ChartSpec) {
        let (w, h) = self.size();
        let values: Vec<f64> = spec
            .series
            .iter()
            .map(|s| s.values.first().copied().unwrap_or(0.0).max(0.0))
            .collect();
        let total: f64 = values.iter().sum();
        if total <= 0.0 {
            return;
        }
        let (cx, cy) = (w / 2.0, h / 2.0);
        let outer = w.min(h) / 2.0 - 1.0;
        let inner = if spec.donut { outer * 0.58 } else { 0.0 };
        let gap = if values.iter().filter(|v| **v > 0.0).count() > 1 {
            (1.2 / outer).min(0.02)
        } else {
            0.0
        };
        let point = |r: f32, a: f32| (cx + r * a.sin(), cy - r * a.cos());
        let mut from = 0.0_f32;
        for (s, &v) in values.iter().enumerate() {
            let sweep = (v / total) as f32 * std::f32::consts::TAU;
            let (a0, a1) = (from + gap, from + sweep - gap);
            from += sweep;
            if a1 <= a0 {
                continue;
            }
            let steps = ((a1 - a0) / 0.05).ceil().max(2.0) as usize;
            let mut pb = PathBuilder::new();
            let start = point(outer, a0);
            pb.move_to(start.0, start.1);
            for k in 1..=steps {
                let p = point(outer, a0 + (a1 - a0) * k as f32 / steps as f32);
                pb.line_to(p.0, p.1);
            }
            if inner > 0.0 {
                for k in (0..=steps).rev() {
                    let p = point(inner, a0 + (a1 - a0) * k as f32 / steps as f32);
                    pb.line_to(p.0, p.1);
                }
            } else {
                pb.line_to(cx, cy);
            }
            pb.close();
            if let Some(path) = pb.finish() {
                self.pixmap.fill_path(
                    &path,
                    &paint(spec.series[s].color, 1.0),
                    FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(kind: ChartKind) -> ChartSpec {
        ChartSpec {
            kind,
            cols: 10,
            rows: 4,
            series: vec![
                SpecSeries {
                    color: [255, 0, 0],
                    values: vec![1.0, 3.0, 2.0],
                },
                SpecSeries {
                    color: [0, 0, 255],
                    values: vec![2.0, 1.0, 3.0],
                },
            ],
            lo: 0.0,
            hi: 3.0,
            group: 2,
            bar: 1,
            stacked: false,
            donut: false,
            axis: [128, 128, 128],
        }
    }

    fn alpha_at(px: &[u8], w: u32, x: u32, y: u32) -> u8 {
        px[((y * w + x) * 4 + 3) as usize]
    }

    #[test]
    fn a_spec_goes_through_its_source() {
        let s = spec(ChartKind::Columns);
        let src = s.src();
        assert!(src.starts_with("chart:{"));
        assert_eq!(ChartSpec::from_src(&src), Some(s));
        assert_eq!(ChartSpec::from_src("/tmp/a.png"), None);
    }

    #[test]
    fn every_kind_draws_on_a_transparent_background() {
        for kind in [
            ChartKind::Line,
            ChartKind::Area,
            ChartKind::Columns,
            ChartKind::Pie,
        ] {
            let (w, h) = (100, 64);
            let px = draw_chart(&spec(kind), w, h).unwrap();
            assert_eq!(px.len(), (w * h * 4) as usize);
            let drawn = px.chunks(4).filter(|p| p[3] > 0).count();
            assert!(drawn > 50, "{kind:?} drew {drawn} pixels");
            assert!(drawn < (w * h) as usize, "{kind:?} filled everything");
            // The top left corner is above every value: nothing there.
            assert_eq!(alpha_at(&px, w, 0, 0), 0, "{kind:?}");
        }
    }

    #[test]
    fn columns_stand_in_their_groups() {
        // Groups of 2 cells, a cell between: the first group is cells 0-1, the gap cell 2.
        let mut s = spec(ChartKind::Columns);
        s.series.truncate(1);
        s.bar = 2;
        let (w, h) = (100, 40);
        let px = draw_chart(&s, w, h).unwrap();
        // Cell 1 (x 10..20) holds the first bar near the bottom; cell 2 (x 20..30) is the gap.
        assert!(alpha_at(&px, w, 15, h - 3) > 0);
        assert_eq!(alpha_at(&px, w, 25, h - 3), 0);
    }

    #[test]
    fn nothing_to_draw_is_no_picture() {
        assert!(draw_chart(&spec(ChartKind::Line), 0, 10).is_none());
    }
}
