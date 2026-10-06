//! Drawing of the diagram vocabulary on the canvas (§3.9.3): the Mermaid
//! flowchart / state shapes (stadium, circle, hexagon, cylinder,
//! parallelogram, subroutine, `[*]` start/end), ER entity tables, UML
//! class boxes, dashed connectors and relation end markers (crow's feet,
//! triangles, diamonds). Screen-space only; the caller maps world → screen.
//! Callers: `canvas::painter`.

use egui::{Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2};

use super::diagram_kinds::{ClassRelKind, EdgeRelation, EntityAttr, ErCardinality};
use super::element::ShapeKind;
use crate::ui::theme;

fn quantize(size: f32) -> f32 {
    if size <= 24.0 { size.round().max(1.0) } else { (size / 4.0).round() * 4.0 }
}

fn ellipse_points(c: Pos2, rx: f32, ry: f32, n: usize) -> Vec<Pos2> {
    (0..n)
        .map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            Pos2::new(c.x + rx * a.cos(), c.y + ry * a.sin())
        })
        .collect()
}

/// Outline + fill of the shapes `canvas::painter` doesn't draw itself.
/// Returns the rect the label should use (`Rect::NOTHING` = no label).
pub fn draw_shape(painter: &Painter, kind: ShapeKind, r: Rect, fill: Color32, stroke: Stroke) -> Rect {
    match kind {
        ShapeKind::Stadium => {
            let radius = (r.height() / 2.0).min(r.width() / 2.0);
            painter.rect(r, radius, fill, stroke, StrokeKind::Middle);
            r.shrink2(Vec2::new(radius * 0.6, 0.0))
        }
        ShapeKind::Circle => {
            let rad = r.width().min(r.height()) / 2.0;
            painter.circle(r.center(), rad, fill, stroke);
            Rect::from_center_size(r.center(), Vec2::splat(rad * 1.4))
        }
        ShapeKind::Hexagon => {
            let inset = (r.height() * 0.3).min(r.width() * 0.25);
            let pts = vec![
                Pos2::new(r.min.x + inset, r.min.y),
                Pos2::new(r.max.x - inset, r.min.y),
                Pos2::new(r.max.x, r.center().y),
                Pos2::new(r.max.x - inset, r.max.y),
                Pos2::new(r.min.x + inset, r.max.y),
                Pos2::new(r.min.x, r.center().y),
            ];
            painter.add(Shape::convex_polygon(pts, fill, stroke));
            r.shrink2(Vec2::new(inset, 0.0))
        }
        ShapeKind::Parallelogram => {
            let skew = (r.height() * 0.35).min(r.width() * 0.25);
            let pts = vec![
                Pos2::new(r.min.x + skew, r.min.y),
                Pos2::new(r.max.x, r.min.y),
                Pos2::new(r.max.x - skew, r.max.y),
                Pos2::new(r.min.x, r.max.y),
            ];
            painter.add(Shape::convex_polygon(pts, fill, stroke));
            r.shrink2(Vec2::new(skew, 0.0))
        }
        ShapeKind::Subroutine => {
            painter.rect(r, 0.0, fill, stroke, StrokeKind::Middle);
            let inset = (r.width() * 0.06).clamp(4.0, 14.0);
            painter.line_segment([Pos2::new(r.min.x + inset, r.min.y), Pos2::new(r.min.x + inset, r.max.y)], stroke);
            painter.line_segment([Pos2::new(r.max.x - inset, r.min.y), Pos2::new(r.max.x - inset, r.max.y)], stroke);
            r.shrink2(Vec2::new(inset, 0.0))
        }
        ShapeKind::Cylinder => {
            let ry = (r.height() * 0.12).min(r.width() * 0.2).max(2.0);
            let rx = r.width() / 2.0;
            let top = Pos2::new(r.center().x, r.min.y + ry);
            let bottom = Pos2::new(r.center().x, r.max.y - ry);
            painter.add(Shape::convex_polygon(ellipse_points(bottom, rx, ry, 32), fill, stroke));
            painter.rect_filled(Rect::from_min_max(Pos2::new(r.min.x, top.y), Pos2::new(r.max.x, bottom.y)), 0.0, fill);
            painter.line_segment([Pos2::new(r.min.x, top.y), Pos2::new(r.min.x, bottom.y)], stroke);
            painter.line_segment([Pos2::new(r.max.x, top.y), Pos2::new(r.max.x, bottom.y)], stroke);
            painter.add(Shape::convex_polygon(ellipse_points(top, rx, ry, 32), fill, stroke));
            Rect::from_min_max(Pos2::new(r.min.x, top.y + ry), Pos2::new(r.max.x, bottom.y))
        }
        ShapeKind::StateStart => {
            let rad = r.width().min(r.height()) / 2.0;
            painter.circle_filled(r.center(), rad, stroke.color);
            Rect::NOTHING
        }
        ShapeKind::StateEnd => {
            let rad = r.width().min(r.height()) / 2.0;
            painter.circle_stroke(r.center(), rad, Stroke::new(stroke.width.max(1.5), stroke.color));
            painter.circle_filled(r.center(), rad * 0.6, stroke.color);
            Rect::NOTHING
        }
        // Drawn by `canvas::painter`.
        ShapeKind::Rectangle
        | ShapeKind::RoundedRect
        | ShapeKind::Ellipse
        | ShapeKind::Diamond
        | ShapeKind::CalloutBubble => r,
    }
}

fn header_colors(color: [f32; 3], is_dark: bool) -> (Color32, Color32, Color32) {
    let accent = Color32::from_rgb((color[0] * 255.0) as u8, (color[1] * 255.0) as u8, (color[2] * 255.0) as u8);
    let body = if is_dark { Color32::from_rgb(30, 33, 41) } else { Color32::from_rgb(252, 252, 254) };
    let text = if is_dark { Color32::from_gray(230) } else { Color32::from_rgb(28, 30, 36) };
    (accent, body, text)
}

fn header_text_color(color: [f32; 3]) -> Color32 {
    let lum = 0.2126 * color[0] + 0.7152 * color[1] + 0.0722 * color[2];
    if lum > 0.55 { Color32::from_rgb(25, 27, 32) } else { Color32::WHITE }
}

/// ER entity: coloured title bar, then `type · name · keys` rows.
pub fn draw_entity(painter: &Painter, r: Rect, name: &str, attrs: &[EntityAttr], color: [f32; 3], zoom: f32, is_dark: bool) {
    let (accent, body, text) = header_colors(color, is_dark);
    let head_h = 28.0 * zoom;
    let row_h = 22.0 * zoom;
    painter.rect(r, CornerRadius::same(theme::RADIUS_SM), body, Stroke::new(1.2 * zoom, accent), StrokeKind::Middle);
    let head = Rect::from_min_size(r.min, Vec2::new(r.width(), head_h.min(r.height())));
    painter.rect_filled(head, CornerRadius { nw: theme::RADIUS_SM, ne: theme::RADIUS_SM, sw: 0, se: 0 }, accent);
    let font = quantize(13.0 * zoom);
    if font < 4.0 {
        return;
    }
    let clip = painter.with_clip_rect(r.intersect(painter.clip_rect()));
    clip.text(head.center(), Align2::CENTER_CENTER, name, theme::semibold(font), header_text_color(color));
    let small = FontId::proportional(quantize(12.0 * zoom));
    let faint = text.gamma_multiply(0.65);
    for (i, a) in attrs.iter().enumerate() {
        let y = head.max.y + i as f32 * row_h;
        if y + row_h > r.max.y + 1.0 {
            break;
        }
        if i > 0 {
            clip.line_segment([Pos2::new(r.min.x, y), Pos2::new(r.max.x, y)], Stroke::new(0.6, faint.gamma_multiply(0.4)));
        }
        let cy = y + row_h / 2.0;
        let pad = 8.0 * zoom;
        clip.text(Pos2::new(r.min.x + pad, cy), Align2::LEFT_CENTER, &a.ty, small.clone(), faint);
        let name_font = if a.keys.contains("PK") { theme::semibold(small.size) } else { small.clone() };
        clip.text(Pos2::new(r.min.x + r.width() * 0.38, cy), Align2::LEFT_CENTER, &a.name, name_font, text);
        if !a.keys.is_empty() {
            clip.text(Pos2::new(r.max.x - pad, cy), Align2::RIGHT_CENTER, &a.keys, small.clone(), accent);
        }
    }
}

/// UML class: annotation + name, attributes, methods in three compartments.
#[allow(clippy::too_many_arguments)]
pub fn draw_class(
    painter: &Painter,
    r: Rect,
    name: &str,
    annotation: &str,
    attributes: &[String],
    methods: &[String],
    color: [f32; 3],
    zoom: f32,
    is_dark: bool,
) {
    let (accent, body, text) = header_colors(color, is_dark);
    painter.rect(r, CornerRadius::same(theme::RADIUS_SM), body, Stroke::new(1.2 * zoom, accent), StrokeKind::Middle);
    let font = quantize(13.0 * zoom);
    if font < 4.0 {
        return;
    }
    let clip = painter.with_clip_rect(r.intersect(painter.clip_rect()));
    let row_h = 20.0 * zoom;
    let mut y = r.min.y + 6.0 * zoom;
    if !annotation.is_empty() {
        clip.text(
            Pos2::new(r.center().x, y + row_h / 2.0),
            Align2::CENTER_CENTER,
            format!("«{annotation}»"),
            FontId::proportional(quantize(11.0 * zoom)),
            text.gamma_multiply(0.7),
        );
        y += row_h;
    }
    clip.text(Pos2::new(r.center().x, y + row_h / 2.0), Align2::CENTER_CENTER, name, theme::semibold(font), text);
    y += row_h + 4.0 * zoom;
    let small = FontId::monospace(quantize(11.5 * zoom));
    let pad = 8.0 * zoom;
    for items in [attributes, methods] {
        clip.line_segment([Pos2::new(r.min.x, y), Pos2::new(r.max.x, y)], Stroke::new(1.0 * zoom, accent));
        y += 3.0 * zoom;
        for item in items {
            clip.text(Pos2::new(r.min.x + pad, y + row_h / 2.0), Align2::LEFT_CENTER, item, small.clone(), text);
            y += row_h;
        }
        if items.is_empty() {
            y += 6.0 * zoom;
        }
    }
}

/// A polyline drawn as dashes.
pub fn draw_dashed(painter: &Painter, path: &[Pos2], stroke: Stroke, zoom: f32) {
    let dash = (7.0 * zoom).max(3.0);
    let gap = (5.0 * zoom).max(2.0);
    for w in path.windows(2) {
        painter.extend(Shape::dashed_line(&[w[0], w[1]], stroke, dash, gap));
    }
}

/// Unit direction pointing *into* the endpoint `tip` from `toward`.
fn dir_into(tip: Pos2, toward: Pos2) -> Vec2 {
    let d = tip - toward;
    if d.length_sq() < 1e-6 { Vec2::X } else { d.normalized() }
}

fn bar(painter: &Painter, tip: Pos2, d: Vec2, at: f32, half: f32, stroke: Stroke) {
    let n = Vec2::new(-d.y, d.x);
    let c = tip - d * at;
    painter.line_segment([c + n * half, c - n * half], stroke);
}

fn er_end(painter: &Painter, tip: Pos2, d: Vec2, card: ErCardinality, stroke: Stroke, bg: Color32, zoom: f32) {
    let s = zoom.max(0.4);
    let n = Vec2::new(-d.y, d.x);
    let crow = |at: f32| {
        let root = tip - d * at;
        for k in [-1.0, 0.0, 1.0] {
            painter.line_segment([root, tip + n * (k * 7.0 * s)], stroke);
        }
    };
    let ring = |at: f32| painter.circle(tip - d * at, 4.5 * s, bg, stroke);
    match card {
        ErCardinality::ExactlyOne => {
            bar(painter, tip, d, 8.0 * s, 7.0 * s, stroke);
            bar(painter, tip, d, 14.0 * s, 7.0 * s, stroke);
        }
        ErCardinality::ZeroOrOne => {
            bar(painter, tip, d, 8.0 * s, 7.0 * s, stroke);
            ring(18.0 * s);
        }
        ErCardinality::OneOrMore => {
            crow(14.0 * s);
            bar(painter, tip, d, 18.0 * s, 7.0 * s, stroke);
        }
        ErCardinality::ZeroOrMore => {
            crow(14.0 * s);
            ring(23.0 * s);
        }
    }
}

fn triangle(painter: &Painter, tip: Pos2, d: Vec2, fill: Color32, stroke: Stroke, zoom: f32) {
    let s = zoom.max(0.4);
    let n = Vec2::new(-d.y, d.x);
    let base = tip - d * 14.0 * s;
    painter.add(Shape::convex_polygon(vec![tip, base + n * 8.0 * s, base - n * 8.0 * s], fill, stroke));
}

fn diamond(painter: &Painter, tip: Pos2, d: Vec2, fill: Color32, stroke: Stroke, zoom: f32) {
    let s = zoom.max(0.4);
    let n = Vec2::new(-d.y, d.x);
    let mid = tip - d * 9.0 * s;
    painter.add(Shape::convex_polygon(vec![tip, mid + n * 6.0 * s, tip - d * 18.0 * s, mid - n * 6.0 * s], fill, stroke));
}

fn open_arrow(painter: &Painter, tip: Pos2, d: Vec2, stroke: Stroke, zoom: f32) {
    let s = zoom.max(0.4);
    let n = Vec2::new(-d.y, d.x);
    let base = tip - d * 12.0 * s;
    painter.line_segment([tip, base + n * 6.0 * s], stroke);
    painter.line_segment([tip, base - n * 6.0 * s], stroke);
}

/// End markers for a relation along screen `path` (from → to); the caller
/// then skips its own arrowhead.
pub fn draw_relation_markers(painter: &Painter, path: &[Pos2], rel: &EdgeRelation, stroke: Stroke, bg: Color32, zoom: f32) {
    if path.len() < 2 {
        return;
    }
    let (from, to) = (path[0], path[path.len() - 1]);
    let d_from = dir_into(from, path[1]);
    let d_to = dir_into(to, path[path.len() - 2]);
    match rel {
        EdgeRelation::Er { from: cf, to: ct, .. } => {
            er_end(painter, from, d_from, *cf, stroke, bg, zoom);
            er_end(painter, to, d_to, *ct, stroke, bg, zoom);
        }
        EdgeRelation::Class { kind, card_from, card_to } => {
            match kind {
                ClassRelKind::Inheritance => triangle(painter, from, d_from, bg, stroke, zoom),
                ClassRelKind::Composition => diamond(painter, from, d_from, stroke.color, stroke, zoom),
                ClassRelKind::Aggregation => diamond(painter, from, d_from, bg, stroke, zoom),
                ClassRelKind::Association => triangle(painter, to, d_to, stroke.color, stroke, zoom),
                ClassRelKind::Dependency => open_arrow(painter, to, d_to, stroke, zoom),
                ClassRelKind::Realization => triangle(painter, to, d_to, bg, stroke, zoom),
                ClassRelKind::Link => {}
            }
            if 11.0 * zoom >= 4.0 {
                let font = FontId::proportional(quantize(11.0 * zoom));
                let off = |tip: Pos2, d: Vec2| tip - d * 24.0 * zoom + Vec2::new(-d.y, d.x) * 10.0 * zoom;
                if !card_from.is_empty() {
                    painter.text(off(from, d_from), Align2::CENTER_CENTER, card_from, font.clone(), stroke.color);
                }
                if !card_to.is_empty() {
                    painter.text(off(to, d_to), Align2::CENTER_CENTER, card_to, font, stroke.color);
                }
            }
        }
    }
}
