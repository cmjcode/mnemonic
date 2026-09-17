//! Rendering engine for whiteboard elements.

use egui::{
    Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, StrokeKind,
    Vec2,
};

use super::element::{CanvasElement, ConnectorRouting, ShapeKind};
use super::viewport::Viewport;
use crate::ui::theme;

fn color_from_rgb(rgb: [f32; 3], alpha: u8) -> Color32 {
    Color32::from_rgba_premultiplied(
        (rgb[0] * 255.0) as u8,
        (rgb[1] * 255.0) as u8,
        (rgb[2] * 255.0) as u8,
        alpha,
    )
}

fn color_from_rgba_unmultiplied(rgb: [f32; 3], alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (rgb[0] * 255.0) as u8,
        (rgb[1] * 255.0) as u8,
        (rgb[2] * 255.0) as u8,
        alpha,
    )
}

/// Labels smaller than this (in screen pixels) are unreadable and skipped.
const MIN_READABLE_FONT: f32 = 4.0;

/// Black or white, whichever reads better on the given fill.
fn contrasting_text_color(fill: [f32; 3]) -> Color32 {
    let luminance = 0.2126 * fill[0] + 0.7152 * fill[1] + 0.0722 * fill[2];
    if luminance > 0.55 {
        Color32::from_rgb(25, 27, 32)
    } else {
        Color32::WHITE
    }
}

/// Draw text centered in `rect`, wrapped to its width and clipped to its bounds.
fn draw_wrapped_label(painter: &Painter, rect: Rect, text: &str, font_size: f32, color: Color32) {
    if font_size < MIN_READABLE_FONT || rect.width() < 2.0 || rect.height() < 2.0 {
        return;
    }
    let padding = (4.0 * font_size / 13.5).min(rect.width() * 0.1);
    let wrap_width = (rect.width() - padding * 2.0).max(1.0);
    let galley = painter.layout(text.to_owned(), FontId::proportional(font_size), color, wrap_width);
    let text_rect = Align2::CENTER_CENTER.anchor_size(rect.center(), galley.size());
    painter
        .with_clip_rect(rect.intersect(painter.clip_rect()))
        .galley(text_rect.min, galley, color);
}

/// Insert elbow points so every segment is horizontal or vertical.
fn orthogonal_path(points: &[Pos2]) -> Vec<Pos2> {
    if points.len() == 2 {
        let (a, b) = (points[0], points[1]);
        if (a.x - b.x).abs() < 0.5 || (a.y - b.y).abs() < 0.5 {
            return vec![a, b];
        }
        return if (b.x - a.x).abs() >= (b.y - a.y).abs() {
            let mid_x = (a.x + b.x) * 0.5;
            vec![a, Pos2::new(mid_x, a.y), Pos2::new(mid_x, b.y), b]
        } else {
            let mid_y = (a.y + b.y) * 0.5;
            vec![a, Pos2::new(a.x, mid_y), Pos2::new(b.x, mid_y), b]
        };
    }

    let mut out = Vec::with_capacity(points.len() * 2);
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if out.is_empty() {
            out.push(a);
        }
        if (a.x - b.x).abs() >= 0.5 && (a.y - b.y).abs() >= 0.5 {
            out.push(Pos2::new(b.x, a.y));
        }
        out.push(b);
    }
    out
}

/// Point halfway along a polyline, by length.
fn polyline_midpoint(points: &[Pos2]) -> Pos2 {
    let total: f32 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
    let mut remaining = total * 0.5;
    for w in points.windows(2) {
        let len = w[0].distance(w[1]);
        if len >= remaining && len > 0.0 {
            return w[0] + (w[1] - w[0]) * (remaining / len);
        }
        remaining -= len;
    }
    points.first().copied().unwrap_or(Pos2::ZERO)
}

/// Draw an element on the canvas given the viewport and screen origin.
pub fn draw_element(
    painter: &Painter,
    viewport: &Viewport,
    origin: Pos2,
    element: &CanvasElement,
    is_selected: bool,
    is_dark: bool,
) {
    match element {
        CanvasElement::Frame {
            rect, title, color, ..
        } => {
            let world_rect = Rect::from_min_max(Pos2::new(rect[0], rect[1]), Pos2::new(rect[2], rect[3]));
            let screen_rect = viewport.world_rect_to_screen(world_rect, origin);
            let frame_color = color_from_rgba_unmultiplied(*color, if is_dark { 30 } else { 20 });
            let border_color = color_from_rgba_unmultiplied(*color, if is_dark { 200 } else { 170 });

            painter.rect_filled(
                screen_rect,
                CornerRadius::same(theme::RADIUS_SM),
                frame_color,
            );
            painter.rect_stroke(
                screen_rect,
                CornerRadius::same(theme::RADIUS_SM),
                (1.5 * viewport.zoom, border_color),
                StrokeKind::Middle,
            );

            // Title label at top-left of frame, clipped to the frame width
            let font_size = 14.0 * viewport.zoom;
            if font_size >= MIN_READABLE_FONT && !title.is_empty() {
                let inset = Vec2::new(10.0, 6.0) * viewport.zoom;
                let title_rect = Rect::from_min_max(screen_rect.min + inset, screen_rect.max - Vec2::new(inset.x, 0.0));
                painter
                    .with_clip_rect(title_rect.intersect(painter.clip_rect()))
                    .text(
                        title_rect.min,
                        Align2::LEFT_TOP,
                        title,
                        FontId::proportional(font_size),
                        border_color,
                    );
            }
        }

        CanvasElement::Shape {
            kind,
            rect,
            stroke_color,
            stroke_width,
            fill_color,
            text,
            text_color,
            ..
        } => {
            let world_rect = Rect::from_min_max(Pos2::new(rect[0], rect[1]), Pos2::new(rect[2], rect[3]));
            let screen_rect = viewport.world_rect_to_screen(world_rect, origin);
            let stroke_c = color_from_rgb(*stroke_color, 255);
            let stroke_w = (*stroke_width * viewport.zoom).max(1.0);
            let fill = fill_color
                .map(|c| color_from_rgb(c, 255))
                .unwrap_or(Color32::TRANSPARENT);
            // A zero stroke width means "no border" (e.g. Draw.io text cells).
            let stroke_w = if *stroke_width <= 0.0 { 0.0 } else { stroke_w };

            match kind {
                ShapeKind::Rectangle => {
                    painter.rect_filled(screen_rect, CornerRadius::ZERO, fill);
                    painter.rect_stroke(
                        screen_rect,
                        CornerRadius::ZERO,
                        (stroke_w, stroke_c),
                        StrokeKind::Middle,
                    );
                }
                ShapeKind::RoundedRect | ShapeKind::CalloutBubble => {
                    painter.rect_filled(
                        screen_rect,
                        CornerRadius::same(theme::RADIUS_MD),
                        fill,
                    );
                    painter.rect_stroke(
                        screen_rect,
                        CornerRadius::same(theme::RADIUS_MD),
                        (stroke_w, stroke_c),
                        StrokeKind::Middle,
                    );
                }
                ShapeKind::Ellipse => {
                    let center = screen_rect.center();
                    let radius_x = screen_rect.width() * 0.5;
                    let radius_y = screen_rect.height() * 0.5;
                    let n = 32;
                    let points: Vec<Pos2> = (0..n)
                        .map(|i| {
                            let angle = (i as f32 / n as f32) * std::f32::consts::TAU;
                            Pos2::new(
                                center.x + radius_x * angle.cos(),
                                center.y + radius_y * angle.sin(),
                            )
                        })
                        .collect();
                    painter.add(egui::Shape::convex_polygon(
                        points,
                        fill,
                        (stroke_w, stroke_c),
                    ));
                }
                ShapeKind::Diamond => {
                    let points = vec![
                        Pos2::new(screen_rect.center().x, screen_rect.min.y),
                        Pos2::new(screen_rect.max.x, screen_rect.center().y),
                        Pos2::new(screen_rect.center().x, screen_rect.max.y),
                        Pos2::new(screen_rect.min.x, screen_rect.center().y),
                    ];
                    painter.add(egui::Shape::convex_polygon(
                        points,
                        fill,
                        (stroke_w, stroke_c),
                    ));
                }
            }

            // Shape inner text: wrapped to the shape width and clipped to its bounds.
            if !text.is_empty() {
                let text_color = text_color
                    .map(|c| color_from_rgb(c, 255))
                    .or_else(|| fill_color.map(contrasting_text_color))
                    .unwrap_or(if is_dark { Color32::WHITE } else { Color32::BLACK });
                draw_wrapped_label(painter, screen_rect, text, 13.5 * viewport.zoom, text_color);
            }
        }

        CanvasElement::StickyNote {
            pos,
            size,
            text,
            color,
            ..
        } => {
            let world_rect =
                Rect::from_min_size(Pos2::new(pos[0], pos[1]), Vec2::new(size[0], size[1]));
            let screen_rect = viewport.world_rect_to_screen(world_rect, origin);
            let fill = color_from_rgb(*color, 240);
            let shadow_color = Color32::from_black_alpha(45);

            // Subtle drop shadow
            let shadow_rect = screen_rect.translate(Vec2::new(2.0, 3.0) * viewport.zoom);
            painter.rect_filled(
                shadow_rect,
                CornerRadius::same(theme::RADIUS_SM),
                shadow_color,
            );

            // Note body
            painter.rect_filled(
                screen_rect,
                CornerRadius::same(theme::RADIUS_SM),
                fill,
            );
            let border_color = color_from_rgb(*color, 255);
            painter.rect_stroke(
                screen_rect,
                CornerRadius::same(theme::RADIUS_SM),
                (1.0 * viewport.zoom, border_color),
                StrokeKind::Middle,
            );

            // Text inside sticky note
            let text_rect = screen_rect.shrink(10.0 * viewport.zoom);
            let text_color = Color32::from_rgb(35, 38, 45);
            let font_size = (13.0 * viewport.zoom).clamp(8.0, 20.0);
            painter.text(
                text_rect.min,
                Align2::LEFT_TOP,
                text,
                FontId::proportional(font_size),
                text_color,
            );
        }

        CanvasElement::DocCard {
            pos,
            size,
            title,
            snippet,
            doc_type,
            ..
        } => {
            let world_rect =
                Rect::from_min_size(Pos2::new(pos[0], pos[1]), Vec2::new(size[0], size[1]));
            let screen_rect = viewport.world_rect_to_screen(world_rect, origin);

            let bg_color = if is_dark {
                Color32::from_rgba_premultiplied(35, 40, 52, 230)
            } else {
                Color32::from_rgba_premultiplied(245, 248, 255, 240)
            };
            let stroke_color = if is_dark {
                Color32::from_rgba_premultiplied(90, 110, 145, 180)
            } else {
                Color32::from_rgba_premultiplied(180, 200, 230, 200)
            };

            painter.rect_filled(
                screen_rect,
                CornerRadius::same(theme::RADIUS_MD),
                bg_color,
            );
            painter.rect_stroke(
                screen_rect,
                CornerRadius::same(theme::RADIUS_MD),
                (1.2 * viewport.zoom, stroke_color),
                StrokeKind::Middle,
            );

            // Badge icon & Title
            let icon = if doc_type == "pdf" { "📄 PDF" } else { "📝 Note" };
            let title_pos = screen_rect.min + Vec2::new(12.0, 10.0) * viewport.zoom;
            painter.text(
                title_pos,
                Align2::LEFT_TOP,
                format!("{} {}", icon, title),
                FontId::proportional((14.0 * viewport.zoom).clamp(9.0, 20.0)),
                if is_dark {
                    Color32::WHITE
                } else {
                    Color32::BLACK
                },
            );

            // Snippet body
            let snippet_pos = title_pos + Vec2::new(0.0, 22.0 * viewport.zoom);
            painter.text(
                snippet_pos,
                Align2::LEFT_TOP,
                snippet,
                FontId::proportional((11.5 * viewport.zoom).clamp(7.5, 16.0)),
                if is_dark {
                    Color32::from_gray(190)
                } else {
                    Color32::from_gray(90)
                },
            );
        }

        CanvasElement::Connector {
            from_pos,
            to_pos,
            routing,
            stroke_color,
            stroke_width,
            label,
            arrow_end,
            waypoints,
            ..
        } => {
            let to_screen = |p: &[f32; 2]| viewport.world_to_screen(Pos2::new(p[0], p[1]), origin);
            let start = to_screen(from_pos);
            let end = to_screen(to_pos);
            let stroke_c = color_from_rgb(*stroke_color, 255);
            let stroke_w = (*stroke_width * viewport.zoom).max(1.5);

            let mut path: Vec<Pos2> = Vec::with_capacity(waypoints.len() + 4);
            path.push(start);
            path.extend(waypoints.iter().map(to_screen));
            path.push(end);

            match routing {
                ConnectorRouting::Curved if waypoints.is_empty() => {
                    let ctrl = Pos2::new(
                        (start.x + end.x) * 0.5,
                        start.y.min(end.y) - 30.0 * viewport.zoom,
                    );
                    let shape = egui::epaint::QuadraticBezierShape::from_points_stroke(
                        [start, ctrl, end],
                        false,
                        Color32::TRANSPARENT,
                        (stroke_w, stroke_c),
                    );
                    painter.add(shape);
                    // Arrow direction follows the curve tangent at the end.
                    path = vec![ctrl, end];
                }
                ConnectorRouting::Orthogonal => {
                    path = orthogonal_path(&path);
                    painter.add(egui::Shape::line(path.clone(), (stroke_w, stroke_c)));
                }
                _ => {
                    painter.add(egui::Shape::line(path.clone(), (stroke_w, stroke_c)));
                }
            }

            // Draw arrowhead at end, aligned with the last segment
            if *arrow_end && path.len() >= 2 {
                let tip = path[path.len() - 1];
                let before = path[..path.len() - 1]
                    .iter()
                    .rev()
                    .find(|p| (tip - **p).length_sq() > 1e-3)
                    .copied()
                    .unwrap_or(start);
                let dir = (tip - before).normalized();
                let normal = Vec2::new(-dir.y, dir.x);
                let head_len = 10.0 * viewport.zoom;
                let head_width = 5.0 * viewport.zoom;
                let p1 = tip - dir * head_len + normal * head_width;
                let p2 = tip - dir * head_len - normal * head_width;
                painter.add(egui::Shape::convex_polygon(
                    vec![tip, p1, p2],
                    stroke_c,
                    (0.0, Color32::TRANSPARENT),
                ));
            }

            // Optional label at the middle of the route
            if !label.is_empty() {
                let label_path = if waypoints.is_empty() && *routing != ConnectorRouting::Orthogonal {
                    vec![start, end]
                } else {
                    path
                };
                let font_size = 12.0 * viewport.zoom;
                if font_size >= MIN_READABLE_FONT {
                    let galley = painter.layout(
                        label.clone(),
                        FontId::proportional(font_size),
                        stroke_c,
                        f32::INFINITY,
                    );
                    let rect = Align2::CENTER_CENTER
                        .anchor_size(polyline_midpoint(&label_path), galley.size());
                    let bg = if is_dark {
                        Color32::from_rgba_unmultiplied(20, 22, 28, 220)
                    } else {
                        Color32::from_rgba_unmultiplied(255, 255, 255, 220)
                    };
                    painter.rect_filled(rect.expand(2.0), CornerRadius::same(2), bg);
                    painter.galley(rect.min, galley, stroke_c);
                }
            }
        }

        CanvasElement::FreehandStroke {
            points,
            color,
            width,
            ..
        } => {
            if points.len() >= 2 {
                let screen_points: Vec<Pos2> = points
                    .iter()
                    .map(|p| viewport.world_to_screen(Pos2::new(p[0], p[1]), origin))
                    .collect();
                let stroke_c = color_from_rgb(*color, 255);
                let stroke_w = (*width * viewport.zoom).max(1.0);
                for w in screen_points.windows(2) {
                    painter.line_segment([w[0], w[1]], (stroke_w, stroke_c));
                }
            }
        }
    }

    // Draw selection bounding outline & resize handles if selected
    if is_selected {
        let bounds = viewport.world_rect_to_screen(element.bounding_rect(), origin);
        let sel_color = Color32::from_rgb(80, 150, 255);
        painter.rect_stroke(
            bounds.expand(4.0),
            CornerRadius::same(theme::RADIUS_SM),
            (1.8, sel_color),
            StrokeKind::Outside,
        );

        // Corner handles
        let handle_radius = 4.0;
        let corners = [
            bounds.min,
            Pos2::new(bounds.max.x, bounds.min.y),
            bounds.max,
            Pos2::new(bounds.min.x, bounds.max.y),
        ];
        for corner in corners {
            painter.circle_filled(corner, handle_radius, Color32::WHITE);
            painter.circle_stroke(corner, handle_radius, (1.5, sel_color));
        }
    }
}
