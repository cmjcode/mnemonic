//! egui backend for Mermaid scenes (§3.7.5) — the only `mermaid` file that
//! touches egui. Draws a `Scene` straight to epaint shapes (no SVG, no
//! textures): primitives outside the clip rect are culled, font sizes are
//! quantised so smooth zooming doesn't thrash the glyph atlas (same trick
//! as `canvas::painter`), and non-convex polygons are triangulated
//! (ear clipping) because epaint only fills convex ones. `show` wraps it
//! as a widget with hover tooltips and click-through for `click` links.
//! Callers: `markdown::renderer`.

use std::collections::HashMap;
use std::sync::Arc;

use egui::epaint::{Mesh, TextShape};
use egui::{Color32, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke as EStroke, StrokeKind, Vec2};

use super::scene::{Anchor, Hit, MarkShape, P, Prim, Scene, Stroke, marker_shapes, prim_bounds};
use super::text::{GlyphTable, line_height};
use super::theme::Color;

/// Real glyph widths of every character in `text`, at `GlyphTable::BASE_SIZE`.
pub fn glyph_table(ctx: &egui::Context, text: &str) -> GlyphTable {
    let mut chars: Vec<char> = text.chars().filter(|c| !c.is_control()).collect();
    chars.sort_unstable();
    chars.dedup();
    let font = FontId::proportional(GlyphTable::BASE_SIZE);
    let widths: HashMap<char, f32> =
        ctx.fonts_mut(|f| chars.iter().map(|&c| (c, f.glyph_width(&font, c))).collect());
    GlyphTable::new(widths)
}

fn c32(c: Color) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

fn quantize(size: f32) -> f32 {
    if size <= 24.0 { size.round().max(1.0) } else { (size / 4.0).round() * 4.0 }
}

/// Paint `scene` with its top-left at `origin`, scaled by `scale`.
pub fn paint(painter: &Painter, scene: &Scene, origin: Pos2, scale: f32) {
    let clip = painter.clip_rect();
    let map = |p: P| Pos2::new(origin.x + p[0] * scale, origin.y + p[1] * scale);
    let stroke_of = |s: &Stroke| EStroke::new((s.width * scale).max(0.5), c32(s.color));
    for prim in &scene.prims {
        let b = prim_bounds(prim);
        let screen = Rect::from_min_max(map([b[0], b[1]]), map([b[2], b[3]])).expand(2.0);
        if !screen.intersects(clip) {
            continue;
        }
        match prim {
            Prim::Rect { min, size, radius, fill, stroke } => {
                let r = Rect::from_min_size(map(*min), Vec2::new(size[0], size[1]) * scale);
                let dashed_border = stroke.as_ref().is_some_and(|s| s.dash.is_some());
                let st = match stroke {
                    Some(s) if !dashed_border => stroke_of(s),
                    _ => EStroke::NONE,
                };
                let radius = (radius * scale).min(r.height() / 2.0).min(r.width() / 2.0);
                painter.rect(r, radius, c32(*fill), st, StrokeKind::Middle);
                if let Some(s) = stroke.as_ref().filter(|_| dashed_border) {
                    let pts = vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
                    dashed(painter, &pts, s, scale);
                }
            }
            Prim::Ellipse { center, radius, fill, stroke } => {
                let (c, r) = (map(*center), Vec2::new(radius[0], radius[1]) * scale);
                painter.add(Shape::ellipse_filled(c, r, c32(*fill)));
                if let Some(s) = stroke {
                    painter.add(Shape::ellipse_stroke(c, r, stroke_of(s)));
                }
            }
            Prim::Polygon { points, fill, stroke } => {
                let pts: Vec<Pos2> = points.iter().map(|p| map(*p)).collect();
                fill_polygon(painter, &pts, c32(*fill));
                if let Some(s) = stroke {
                    if s.dash.is_some() {
                        let mut closed = pts.clone();
                        closed.push(pts[0]);
                        dashed(painter, &closed, s, scale);
                    } else {
                        painter.add(Shape::closed_line(pts, stroke_of(s)));
                    }
                }
            }
            Prim::Line { points, stroke } => {
                let pts: Vec<Pos2> = points.iter().map(|p| map(*p)).collect();
                if stroke.dash.is_some() {
                    dashed(painter, &pts, stroke, scale);
                } else {
                    painter.add(Shape::line(pts, stroke_of(stroke)));
                }
            }
            Prim::Marker { at, angle, kind, color, bg, size } => {
                let st = EStroke::new((1.3 * scale).max(0.6), c32(*color));
                for shape in marker_shapes(*kind, *at, *angle, *size) {
                    match shape {
                        MarkShape::Poly { points, filled } => {
                            let pts: Vec<Pos2> = points.iter().map(|p| map(*p)).collect();
                            let fill = if filled { c32(*color) } else { c32(*bg) };
                            painter.add(Shape::convex_polygon(pts, fill, st));
                        }
                        MarkShape::Circle { center, r, filled } => {
                            let fill = if filled { c32(*color) } else { c32(*bg) };
                            painter.circle(map(center), r * scale, fill, st);
                        }
                        MarkShape::Lines(lines) => {
                            for [a, b] in lines {
                                painter.line_segment([map(a), map(b)], st);
                            }
                        }
                    }
                }
            }
            Prim::Text { pos, text, size, color, anchor, bold, angle, .. } => {
                paint_text(painter, map(*pos), text, *size, scale, c32(*color), *anchor, *bold, *angle);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_text(
    painter: &Painter,
    anchor_pt: Pos2,
    text: &str,
    size: f32,
    scale: f32,
    color: Color32,
    anchor: Anchor,
    bold: bool,
    angle: f32,
) {
    let px = quantize(size * scale);
    if px < 4.0 {
        return;
    }
    let font = FontId::proportional(px);
    let lh = line_height(size) * scale;
    let lines: Vec<&str> = text.split('\n').collect();
    let total_h = lh * lines.len() as f32;
    for (i, line) in lines.iter().enumerate() {
        let galley = painter.layout_no_wrap((*line).to_string(), font.clone(), color);
        let w = galley.size().x;
        if angle.abs() > 1e-3 {
            let shape = TextShape::new(anchor_pt, galley, color).with_angle_and_anchor(angle, egui::Align2::CENTER_CENTER);
            painter.add(shape);
            continue;
        }
        let x = match anchor {
            Anchor::Start => anchor_pt.x,
            Anchor::Middle => anchor_pt.x - w / 2.0,
            Anchor::End => anchor_pt.x - w,
        };
        let y = anchor_pt.y - total_h / 2.0 + i as f32 * lh + (lh - galley.size().y) / 2.0;
        if bold {
            // Faux bold: egui's default fonts have no bold face.
            painter.galley(Pos2::new(x + 0.5 * scale.max(0.8), y), Arc::clone(&galley), color);
        }
        painter.galley(Pos2::new(x, y), galley, color);
    }
}

fn dashed(painter: &Painter, pts: &[Pos2], s: &Stroke, scale: f32) {
    let [on, off] = s.dash.unwrap_or([4.0, 4.0]);
    let st = EStroke::new((s.width * scale).max(0.5), c32(s.color));
    painter.extend(Shape::dashed_line(pts, st, (on * scale).max(1.0), (off * scale).max(1.0)));
}

fn fill_polygon(painter: &Painter, pts: &[Pos2], fill: Color32) {
    if pts.len() < 3 || fill.a() == 0 {
        return;
    }
    if is_convex(pts) {
        painter.add(Shape::convex_polygon(pts.to_vec(), fill, EStroke::NONE));
        return;
    }
    let mut mesh = Mesh::default();
    for p in pts {
        mesh.colored_vertex(*p, fill);
    }
    for [a, b, c] in triangulate(pts) {
        mesh.add_triangle(a as u32, b as u32, c as u32);
    }
    painter.add(Shape::mesh(mesh));
}

fn cross(o: Pos2, a: Pos2, b: Pos2) -> f32 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

fn is_convex(pts: &[Pos2]) -> bool {
    let n = pts.len();
    let mut sign = 0.0f32;
    for i in 0..n {
        let c = cross(pts[i], pts[(i + 1) % n], pts[(i + 2) % n]);
        if c.abs() < 1e-3 {
            continue;
        }
        if sign == 0.0 {
            sign = c.signum();
        } else if c.signum() != sign {
            return false;
        }
    }
    true
}

/// Ear clipping for a simple polygon (either winding).
fn triangulate(pts: &[Pos2]) -> Vec<[usize; 3]> {
    let n = pts.len();
    let area: f32 = (0..n).map(|i| cross(Pos2::ZERO, pts[i], pts[(i + 1) % n])).sum();
    let ccw = area > 0.0;
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity(n.saturating_sub(2));
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let (a, b, c) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let turn = cross(pts[a], pts[b], pts[c]);
            if (turn > 0.0) != ccw || turn.abs() < 1e-6 {
                continue;
            }
            let contains = idx.iter().any(|&p| {
                p != a && p != b && p != c && {
                    let d1 = cross(pts[a], pts[b], pts[p]);
                    let d2 = cross(pts[b], pts[c], pts[p]);
                    let d3 = cross(pts[c], pts[a], pts[p]);
                    (d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0) || (d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0)
                }
            });
            if !contains {
                out.push([a, b, c]);
                idx.remove(i);
                clipped = true;
                break;
            }
        }
        if !clipped {
            break;
        }
    }
    if idx.len() == 3 {
        out.push([idx[0], idx[1], idx[2]]);
    } else if idx.len() > 3 {
        // Degenerate input: fan the rest so something is drawn.
        for i in 1..idx.len() - 1 {
            out.push([idx[0], idx[i], idx[i + 1]]);
        }
    }
    out
}

/// What the user did with a drawn diagram this frame.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DiagramInteraction {
    /// A node with a `click … href` link was clicked.
    pub clicked_link: Option<String>,
    /// A node without a link was clicked: its 1-based source line.
    pub clicked_line: Option<usize>,
}

/// Draw `scene` as a widget, fitted to the available width (never
/// enlarged), with hover highlight/tooltip and click handling.
pub fn show(ui: &mut egui::Ui, scene: &Scene) -> DiagramInteraction {
    let avail = ui.available_width().max(40.0);
    let scale = (avail / scene.width.max(1.0)).min(1.0);
    let size = Vec2::new(scene.width * scale, scene.height * scale);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(avail, size.y), Sense::click());
    let origin = Pos2::new(rect.center().x - size.x / 2.0, rect.top());
    let painter = ui.painter_at(rect);
    paint(&painter, scene, origin, scale);

    let mut out = DiagramInteraction::default();
    let Some(pointer) = response.hover_pos() else { return out };
    let local = [(pointer.x - origin.x) / scale, (pointer.y - origin.y) / scale];
    // Last hit wins: nodes are registered after their clusters.
    let hit: Option<&Hit> = scene
        .hits
        .iter()
        .rev()
        .find(|h| local[0] >= h.rect[0] && local[0] <= h.rect[2] && local[1] >= h.rect[1] && local[1] <= h.rect[3]);
    if let Some(h) = hit {
        let r = Rect::from_min_max(
            Pos2::new(origin.x + h.rect[0] * scale, origin.y + h.rect[1] * scale),
            Pos2::new(origin.x + h.rect[2] * scale, origin.y + h.rect[3] * scale),
        );
        painter.rect_stroke(r.expand(2.0), 3.0, ui.visuals().selection.stroke, StrokeKind::Outside);
        if h.link.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let clicked = response.clicked();
        if let Some(tip) = h.tooltip.clone().or_else(|| h.link.clone()) {
            response.on_hover_text(tip);
        }
        if clicked {
            match &h.link {
                Some(link) => out.clicked_link = Some(link.clone()),
                None => out.clicked_line = Some(h.line),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangulates_concave_polygons() {
        // The flowchart "odd" shape: a concave pentagon.
        let pts = [
            Pos2::new(0.0, 0.0),
            Pos2::new(10.0, 0.0),
            Pos2::new(10.0, 10.0),
            Pos2::new(0.0, 10.0),
            Pos2::new(3.0, 5.0),
        ];
        assert!(!is_convex(&pts));
        let tris = triangulate(&pts);
        assert_eq!(tris.len(), 3);
        let area: f32 = tris.iter().map(|[a, b, c]| cross(pts[*a], pts[*b], pts[*c]).abs() / 2.0).sum();
        assert!((area - 85.0).abs() < 0.01, "{area}");
        assert!(is_convex(&[Pos2::new(0.0, 0.0), Pos2::new(1.0, 0.0), Pos2::new(1.0, 1.0)]));
    }
}
