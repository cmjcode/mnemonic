//! Backend-neutral display list for a laid-out Mermaid diagram (§3.7.3).
//! Every diagram type builds a `Scene`; `mermaid::paint` draws it with
//! egui, `mermaid::svg` serialises it, thumbnails sample it. Hit regions
//! carry the node id and source line so a click can jump to the text.
//! Geometry helpers shared by all diagram types (B-spline sampling, arrow
//! markers, shape clipping) live here too. Callers: every diagram
//! builder, `mermaid::paint`, `mermaid::svg`.

use super::text::{TextMeasure, line_height};
use super::theme::Color;

pub type P = [f32; 2];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stroke {
    pub color: Color,
    pub width: f32,
    pub dash: Option<[f32; 2]>,
}

impl Stroke {
    pub fn new(color: Color, width: f32) -> Stroke {
        Stroke { color, width, dash: None }
    }

    pub fn dashed(color: Color, width: f32, dash: [f32; 2]) -> Stroke {
        Stroke { color, width, dash: Some(dash) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Start,
    Middle,
    End,
}

/// Line-end decorations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    /// Filled triangle (flowchart `-->`, sequence `->>`).
    Arrow,
    /// Two strokes (sequence `-)` async / open arrow).
    OpenArrow,
    Circle,
    Cross,
    /// Hollow triangle (class inheritance / realization).
    Triangle,
    DiamondFilled,
    DiamondHollow,
    /// ER cardinalities (crow's foot).
    ExactlyOne,
    ZeroOrOne,
    OneOrMore,
    ZeroOrMore,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Prim {
    Rect { min: P, size: P, radius: f32, fill: Color, stroke: Option<Stroke> },
    Ellipse { center: P, radius: P, fill: Color, stroke: Option<Stroke> },
    Polygon { points: Vec<P>, fill: Color, stroke: Option<Stroke> },
    /// Open polyline.
    Line { points: Vec<P>, stroke: Stroke },
    /// `at` is the tip; `angle` is the direction the line travels into the
    /// tip (radians). `bg` fills hollow markers.
    Marker { at: P, angle: f32, kind: Marker, color: Color, bg: Color, size: f32 },
    /// Multi-line text; `pos` is the anchor point at the vertical centre
    /// of the whole block; `extent` its measured size (for bounds/culling).
    Text {
        pos: P,
        text: String,
        size: f32,
        color: Color,
        anchor: Anchor,
        bold: bool,
        italic: bool,
        extent: P,
        /// Rotation in radians around `pos` (axis titles).
        angle: f32,
    },
}

/// Clickable region → diagram element + source line (1-based within the
/// diagram source).
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub rect: [f32; 4],
    pub id: String,
    pub line: usize,
    pub link: Option<String>,
    pub tooltip: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scene {
    pub width: f32,
    pub height: f32,
    pub background: Color,
    pub prims: Vec<Prim>,
    pub hits: Vec<Hit>,
}

impl Scene {
    pub fn new(background: Color) -> Scene {
        Scene { background, ..Scene::default() }
    }

    pub fn push(&mut self, prim: Prim) {
        self.prims.push(prim);
    }

    pub fn rect(&mut self, min: P, size: P, radius: f32, fill: Color, stroke: Option<Stroke>) {
        self.prims.push(Prim::Rect { min, size, radius, fill, stroke });
    }

    pub fn line(&mut self, points: Vec<P>, stroke: Stroke) {
        if points.len() >= 2 {
            self.prims.push(Prim::Line { points, stroke });
        }
    }

    pub fn polygon(&mut self, points: Vec<P>, fill: Color, stroke: Option<Stroke>) {
        if points.len() >= 3 {
            self.prims.push(Prim::Polygon { points, fill, stroke });
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        measure: &dyn TextMeasure,
        pos: P,
        text: &str,
        size: f32,
        color: Color,
        anchor: Anchor,
        bold: bool,
    ) {
        if text.is_empty() {
            return;
        }
        let (w, h) = measure.size(text, size);
        self.prims.push(Prim::Text {
            pos,
            text: text.to_string(),
            size,
            color,
            anchor,
            bold,
            italic: false,
            extent: [w * if bold { 1.06 } else { 1.0 }, h],
            angle: 0.0,
        });
    }

    /// A polyline with optional markers at either end; the line itself is
    /// shortened so a filled marker's tip lands exactly on the endpoint.
    pub fn edge(&mut self, mut points: Vec<P>, stroke: Stroke, start: Option<Marker>, end: Option<Marker>, bg: Color) {
        if points.len() < 2 {
            return;
        }
        let size = marker_size(stroke.width);
        let n = points.len();
        let end_angle = angle(points[n - 2], points[n - 1]);
        let start_angle = angle(points[1], points[0]);
        let (tip_end, tip_start) = (points[n - 1], points[0]);
        if let Some(m) = end {
            points[n - 1] = retreat(points[n - 2], points[n - 1], marker_inset(m, size));
        }
        if let Some(m) = start {
            points[0] = retreat(points[1], points[0], marker_inset(m, size));
        }
        self.prims.push(Prim::Line { points, stroke });
        for (m, at, a) in [(end, tip_end, end_angle), (start, tip_start, start_angle)] {
            if let Some(kind) = m {
                self.prims.push(Prim::Marker { at, angle: a, kind, color: stroke.color, bg, size });
            }
        }
    }

    pub fn hit(&mut self, rect: [f32; 4], id: impl Into<String>, line: usize) {
        self.hits.push(Hit { rect, id: id.into(), line, link: None, tooltip: None });
    }

    /// Bounding box of everything drawn: `[x0, y0, x1, y1]`.
    pub fn bounds(&self) -> Option<[f32; 4]> {
        let mut b: Option<[f32; 4]> = None;
        for p in &self.prims {
            let r = prim_bounds(p);
            b = Some(match b {
                None => r,
                Some(o) => [o[0].min(r[0]), o[1].min(r[1]), o[2].max(r[2]), o[3].max(r[3])],
            });
        }
        b
    }

    /// Translate so the drawing starts at (`pad`, `pad`) and set the size.
    pub fn fit(&mut self, pad: f32) {
        let Some(b) = self.bounds() else {
            self.width = 2.0 * pad;
            self.height = 2.0 * pad;
            return;
        };
        self.translate(pad - b[0], pad - b[1]);
        self.width = (b[2] - b[0]) + 2.0 * pad;
        self.height = (b[3] - b[1]) + 2.0 * pad;
    }

    pub fn translate(&mut self, dx: f32, dy: f32) {
        let t = |p: &mut P| {
            p[0] += dx;
            p[1] += dy;
        };
        for prim in &mut self.prims {
            match prim {
                Prim::Rect { min, .. } => t(min),
                Prim::Ellipse { center, .. } => t(center),
                Prim::Polygon { points, .. } | Prim::Line { points, .. } => points.iter_mut().for_each(t),
                Prim::Marker { at, .. } => t(at),
                Prim::Text { pos, .. } => t(pos),
            }
        }
        for h in &mut self.hits {
            h.rect[0] += dx;
            h.rect[2] += dx;
            h.rect[1] += dy;
            h.rect[3] += dy;
        }
    }
}

pub fn prim_bounds(p: &Prim) -> [f32; 4] {
    match p {
        Prim::Rect { min, size, stroke, .. } => {
            let s = stroke.map_or(0.0, |s| s.width / 2.0);
            [min[0] - s, min[1] - s, min[0] + size[0] + s, min[1] + size[1] + s]
        }
        Prim::Ellipse { center, radius, .. } => {
            [center[0] - radius[0], center[1] - radius[1], center[0] + radius[0], center[1] + radius[1]]
        }
        Prim::Polygon { points, .. } | Prim::Line { points, .. } => points_bounds(points),
        Prim::Marker { at, size, .. } => [at[0] - size, at[1] - size, at[0] + size, at[1] + size],
        Prim::Text { pos, anchor, extent, angle, .. } => {
            if angle.abs() > 0.1 {
                // Rotated (vertical) text: centred on `pos`.
                let (w, h) = (extent[1], extent[0]);
                return [pos[0] - w / 2.0, pos[1] - h / 2.0, pos[0] + w / 2.0, pos[1] + h / 2.0];
            }
            let (w, h) = (extent[0], extent[1]);
            let x0 = match anchor {
                Anchor::Start => pos[0],
                Anchor::Middle => pos[0] - w / 2.0,
                Anchor::End => pos[0] - w,
            };
            [x0, pos[1] - h / 2.0, x0 + w, pos[1] + h / 2.0]
        }
    }
}

pub fn points_bounds(points: &[P]) -> [f32; 4] {
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for p in points {
        b[0] = b[0].min(p[0]);
        b[1] = b[1].min(p[1]);
        b[2] = b[2].max(p[0]);
        b[3] = b[3].max(p[1]);
    }
    if b[0] > b[2] { [0.0; 4] } else { b }
}

/// Top-left corner of text line `line_idx`, given the block's anchor.
pub fn text_origin(pos: P, anchor: Anchor, line_w: f32, total_h: f32, line_idx: usize, size: f32) -> P {
    let x = match anchor {
        Anchor::Start => pos[0],
        Anchor::Middle => pos[0] - line_w / 2.0,
        Anchor::End => pos[0] - line_w,
    };
    [x, pos[1] - total_h / 2.0 + line_idx as f32 * line_height(size)]
}

pub fn angle(from: P, to: P) -> f32 {
    (to[1] - from[1]).atan2(to[0] - from[0])
}

/// Move `to` back towards `from` by `d` (not past it).
pub fn retreat(from: P, to: P, d: f32) -> P {
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    let len = (dx * dx + dy * dy).sqrt();
    if len <= d + 0.5 {
        return to;
    }
    let k = (len - d) / len;
    [from[0] + dx * k, from[1] + dy * k]
}

pub fn marker_size(stroke_width: f32) -> f32 {
    (7.0 + stroke_width * 1.5).min(14.0)
}

fn marker_inset(m: Marker, size: f32) -> f32 {
    match m {
        Marker::Arrow | Marker::Triangle => size * 0.9,
        Marker::DiamondFilled | Marker::DiamondHollow => size * 1.6,
        Marker::Circle => size * 0.9,
        _ => 0.0,
    }
}

/// Concrete geometry of a marker, shared by the egui and SVG backends.
#[derive(Debug, Clone, PartialEq)]
pub enum MarkShape {
    Poly { points: Vec<P>, filled: bool },
    Circle { center: P, r: f32, filled: bool },
    Lines(Vec<[P; 2]>),
}

pub fn marker_shapes(kind: Marker, at: P, angle: f32, size: f32) -> Vec<MarkShape> {
    let (c, s) = (angle.cos(), angle.sin());
    // Local frame: x along the line (towards the tip), y across.
    let l = |x: f32, y: f32| -> P { [at[0] + x * c - y * s, at[1] + x * s + y * c] };
    let w = size * 0.5;
    match kind {
        Marker::Arrow => vec![MarkShape::Poly { points: vec![l(0.0, 0.0), l(-size, -w), l(-size, w)], filled: true }],
        Marker::Triangle => vec![MarkShape::Poly {
            points: vec![l(0.0, 0.0), l(-size * 1.2, -w * 1.2), l(-size * 1.2, w * 1.2)],
            filled: false,
        }],
        Marker::OpenArrow => vec![MarkShape::Lines(vec![[l(0.0, 0.0), l(-size, -w)], [l(0.0, 0.0), l(-size, w)]])],
        Marker::Circle => vec![MarkShape::Circle { center: l(-w * 0.9, 0.0), r: w * 0.9, filled: true }],
        Marker::Cross => vec![MarkShape::Lines(vec![
            [l(-w * 1.6, -w * 0.7), l(-w * 0.2, w * 0.7)],
            [l(-w * 1.6, w * 0.7), l(-w * 0.2, -w * 0.7)],
        ])],
        Marker::DiamondFilled | Marker::DiamondHollow => vec![MarkShape::Poly {
            points: vec![l(0.0, 0.0), l(-size * 0.8, -w), l(-size * 1.6, 0.0), l(-size * 0.8, w)],
            filled: kind == Marker::DiamondFilled,
        }],
        Marker::ExactlyOne => vec![MarkShape::Lines(vec![
            [l(-size * 0.6, -w), l(-size * 0.6, w)],
            [l(-size * 1.0, -w), l(-size * 1.0, w)],
        ])],
        Marker::ZeroOrOne => vec![
            MarkShape::Lines(vec![[l(-size * 0.5, -w), l(-size * 0.5, w)]]),
            MarkShape::Circle { center: l(-size * 1.3, 0.0), r: w * 0.6, filled: false },
        ],
        Marker::OneOrMore => vec![MarkShape::Lines(vec![
            [l(0.0, -w), l(-size, 0.0)],
            [l(0.0, w), l(-size, 0.0)],
            [l(0.0, 0.0), l(-size, 0.0)],
            [l(-size * 1.2, -w), l(-size * 1.2, w)],
        ])],
        Marker::ZeroOrMore => vec![
            MarkShape::Lines(vec![[l(0.0, -w), l(-size, 0.0)], [l(0.0, w), l(-size, 0.0)], [l(0.0, 0.0), l(-size, 0.0)]]),
            MarkShape::Circle { center: l(-size * 1.5, 0.0), r: w * 0.6, filled: false },
        ],
    }
}

/// Uniform cubic B-spline through the control polygon, clamped to both
/// endpoints — the same curve as d3's `curveBasis` that Mermaid uses for
/// flowchart edges by default.
pub fn basis_curve(points: &[P], samples_per_segment: usize) -> Vec<P> {
    if points.len() < 3 {
        return points.to_vec();
    }
    // Triple the end points so the curve starts and ends on them.
    let mut ctrl = Vec::with_capacity(points.len() + 4);
    ctrl.push(points[0]);
    ctrl.push(points[0]);
    ctrl.extend_from_slice(points);
    ctrl.push(points[points.len() - 1]);
    ctrl.push(points[points.len() - 1]);
    let mut out: Vec<P> = Vec::with_capacity((ctrl.len() - 3) * samples_per_segment + 1);
    for i in 0..ctrl.len() - 3 {
        let (p0, p1, p2, p3) = (ctrl[i], ctrl[i + 1], ctrl[i + 2], ctrl[i + 3]);
        for k in 0..samples_per_segment {
            let t = k as f32 / samples_per_segment as f32;
            let (t2, t3) = (t * t, t * t * t);
            let b0 = (1.0 - t).powi(3) / 6.0;
            let b1 = (3.0 * t3 - 6.0 * t2 + 4.0) / 6.0;
            let b2 = (-3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0) / 6.0;
            let b3 = t3 / 6.0;
            out.push([
                b0 * p0[0] + b1 * p1[0] + b2 * p2[0] + b3 * p3[0],
                b0 * p0[1] + b1 * p1[1] + b2 * p2[1] + b3 * p3[1],
            ]);
        }
    }
    out.push(points[points.len() - 1]);
    out.dedup_by(|a, b| (a[0] - b[0]).abs() < 0.01 && (a[1] - b[1]).abs() < 0.01);
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipShape {
    Rect,
    Diamond,
    Ellipse,
}

/// Where the ray from a shape's centre towards `outside` crosses the
/// boundary of the shape (centre `center`, half-size `half`).
pub fn clip_to_box(center: P, half: P, outside: P, shape: ClipShape) -> P {
    let (dx, dy) = (outside[0] - center[0], outside[1] - center[1]);
    if dx.abs() < 1e-4 && dy.abs() < 1e-4 {
        return center;
    }
    let t = match shape {
        ClipShape::Rect => {
            let tx = if dx.abs() > 1e-6 { half[0] / dx.abs() } else { f32::MAX };
            let ty = if dy.abs() > 1e-6 { half[1] / dy.abs() } else { f32::MAX };
            tx.min(ty)
        }
        ClipShape::Diamond => 1.0 / (dx.abs() / half[0].max(1e-3) + dy.abs() / half[1].max(1e-3)),
        ClipShape::Ellipse => {
            let (a, b) = (half[0].max(1e-3), half[1].max(1e-3));
            1.0 / ((dx * dx) / (a * a) + (dy * dy) / (b * b)).sqrt()
        }
    };
    let t = t.min(1.0);
    [center[0] + dx * t, center[1] + dy * t]
}

/// Point halfway along a polyline, by length.
pub fn polyline_midpoint(points: &[P]) -> P {
    let seg = |a: P, b: P| ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    let total: f32 = points.windows(2).map(|w| seg(w[0], w[1])).sum();
    let mut remaining = total / 2.0;
    for w in points.windows(2) {
        let len = seg(w[0], w[1]);
        if len >= remaining && len > 0.0 {
            let k = remaining / len;
            return [w[0][0] + (w[1][0] - w[0][0]) * k, w[0][1] + (w[1][1] - w[0][1]) * k];
        }
        remaining -= len;
    }
    points.first().copied().unwrap_or([0.0, 0.0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basis_curve_starts_and_ends_on_endpoints() {
        let pts = [[0.0, 0.0], [50.0, 100.0], [100.0, 0.0]];
        let c = basis_curve(&pts, 8);
        assert_eq!(c.first(), Some(&[0.0, 0.0]));
        assert_eq!(c.last(), Some(&[100.0, 0.0]));
        assert!(c.len() > 8);
        // Smooth: passes near, not through, the middle control point.
        assert!(c.iter().all(|p| p[1] < 100.0));
    }

    #[test]
    fn clip_hits_shape_boundaries() {
        let r = clip_to_box([0.0, 0.0], [10.0, 5.0], [100.0, 0.0], ClipShape::Rect);
        assert!((r[0] - 10.0).abs() < 1e-3 && r[1].abs() < 1e-3);
        let d = clip_to_box([0.0, 0.0], [10.0, 10.0], [10.0, 10.0], ClipShape::Diamond);
        assert!((d[0] - 5.0).abs() < 1e-3);
        let e = clip_to_box([0.0, 0.0], [10.0, 10.0], [0.0, 50.0], ClipShape::Ellipse);
        assert!((e[1] - 10.0).abs() < 1e-3);
    }

    #[test]
    fn fit_translates_to_padding() {
        let mut s = Scene::new([255; 4]);
        s.rect([50.0, 60.0], [10.0, 10.0], 0.0, [0; 4], None);
        s.hit([50.0, 60.0, 60.0, 70.0], "a", 1);
        s.fit(8.0);
        assert_eq!(s.width, 26.0);
        assert_eq!(s.hits[0].rect[0], 8.0);
    }

    #[test]
    fn edge_shortens_line_for_arrow_and_keeps_tip() {
        let mut s = Scene::new([255; 4]);
        s.edge(vec![[0.0, 0.0], [100.0, 0.0]], Stroke::new([0, 0, 0, 255], 2.0), None, Some(Marker::Arrow), [255; 4]);
        let Prim::Line { points, .. } = &s.prims[0] else { panic!("expected line") };
        assert!(points[1][0] < 100.0);
        let Prim::Marker { at, .. } = &s.prims[1] else { panic!("expected marker") };
        assert_eq!(*at, [100.0, 0.0]);
    }
}
