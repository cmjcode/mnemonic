//! Card thumbnails for diagram notes (§Fase 3.8): a tiny vector
//! rendering of a `CanvasDocument` — rectangles, ellipses, connector
//! lines, strokes — scaled into the card, built once per vault refresh
//! (`app::Derived`) and painted per frame without textures. Callers:
//! `app` (build), `app::grid` (paint).

use egui::{Color32, Painter, Pos2, Rect, Stroke, StrokeKind};

use super::element::{CanvasElement, ShapeKind};
use super::CanvasDocument;

#[derive(Debug, Clone, PartialEq)]
enum Item {
    Box { rect: [f32; 4], fill: [f32; 3], ellipse: bool, diamond: bool },
    Line { from: [f32; 2], to: [f32; 2], color: [f32; 3] },
    Stroke { points: Vec<[f32; 2]>, color: [f32; 3] },
}

/// A resolution-independent sketch of a diagram.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CanvasThumb {
    /// World-space bounds of everything drawn.
    bounds: Option<[f32; 4]>,
    items: Vec<Item>,
}

/// Elements beyond this are dropped (cards are small; enough to convey
/// the shape of the diagram).
const MAX_ITEMS: usize = 120;

impl CanvasThumb {
    pub fn from_doc(doc: &CanvasDocument) -> CanvasThumb {
        let mut items = Vec::new();
        let mut bounds: Option<Rect> = None;
        let mut grow = |r: Rect| {
            bounds = Some(bounds.map_or(r, |b| b.union(r)));
        };
        for elem in doc.elements.iter().take(MAX_ITEMS) {
            let r = elem.bounding_rect();
            match elem {
                CanvasElement::StickyNote { color, .. } => {
                    grow(r);
                    items.push(Item::Box { rect: rect4(r), fill: *color, ellipse: false, diamond: false });
                }
                CanvasElement::Shape { kind, fill_color, stroke_color, .. } => {
                    grow(r);
                    items.push(Item::Box {
                        rect: rect4(r),
                        fill: fill_color.unwrap_or(*stroke_color),
                        ellipse: matches!(kind, ShapeKind::Ellipse),
                        diamond: matches!(kind, ShapeKind::Diamond),
                    });
                }
                CanvasElement::Frame { .. } | CanvasElement::DocCard { .. } => {
                    grow(r);
                    let fill = color_of(elem).unwrap_or([0.6, 0.6, 0.65]);
                    items.push(Item::Box { rect: rect4(r), fill, ellipse: false, diamond: false });
                }
                CanvasElement::Connector { from_pos, to_pos, stroke_color, waypoints, .. } => {
                    grow(r);
                    let mut points = vec![*from_pos];
                    points.extend(waypoints.iter().copied());
                    points.push(*to_pos);
                    for pair in points.windows(2) {
                        items.push(Item::Line { from: pair[0], to: pair[1], color: *stroke_color });
                    }
                }
                CanvasElement::FreehandStroke { points, color, .. } => {
                    if points.len() >= 2 {
                        grow(r);
                        items.push(Item::Stroke { points: points.clone(), color: *color });
                    }
                }
            }
        }
        CanvasThumb {
            bounds: bounds.map(rect4),
            items,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Draws the sketch fitted (letterboxed, 8 px padding) into `area`.
    pub fn paint(&self, painter: &Painter, area: Rect, is_dark: bool) {
        let Some(b) = self.bounds else {
            return;
        };
        let world = Rect::from_min_max(Pos2::new(b[0], b[1]), Pos2::new(b[2], b[3]));
        let inner = area.shrink(8.0);
        if inner.width() < 4.0 || inner.height() < 4.0 {
            return;
        }
        let scale = (inner.width() / world.width().max(1.0))
            .min(inner.height() / world.height().max(1.0))
            .min(1.0);
        let drawn = world.size() * scale;
        let origin = inner.center() - drawn * 0.5;
        let map = |p: [f32; 2]| origin + (Pos2::new(p[0], p[1]) - world.min) * scale;
        let line_w = (1.2 * scale.max(0.35)).clamp(0.8, 2.0);
        let outline = if is_dark { Color32::from_black_alpha(90) } else { Color32::from_black_alpha(60) };

        for item in &self.items {
            match item {
                Item::Box { rect, fill, ellipse, diamond } => {
                    let r = Rect::from_min_max(map([rect[0], rect[1]]), map([rect[2], rect[3]]));
                    let color = rgb(*fill, if is_dark { 220 } else { 235 });
                    if *ellipse {
                        painter.add(egui::Shape::ellipse_filled(r.center(), r.size() * 0.5, color));
                    } else if *diamond {
                        let pts = vec![
                            Pos2::new(r.center().x, r.top()),
                            Pos2::new(r.right(), r.center().y),
                            Pos2::new(r.center().x, r.bottom()),
                            Pos2::new(r.left(), r.center().y),
                        ];
                        painter.add(egui::Shape::convex_polygon(pts, color, Stroke::new(0.5, outline)));
                    } else {
                        painter.rect(r, 2.0, color, Stroke::new(0.5, outline), StrokeKind::Middle);
                    }
                }
                Item::Line { from, to, color } => {
                    painter.line_segment([map(*from), map(*to)], Stroke::new(line_w, rgb(*color, 200)));
                }
                Item::Stroke { points, color } => {
                    let pts: Vec<Pos2> = points.iter().map(|p| map(*p)).collect();
                    painter.add(egui::Shape::line(pts, Stroke::new(line_w, rgb(*color, 200))));
                }
            }
        }
    }
}

fn rect4(r: Rect) -> [f32; 4] {
    [r.min.x, r.min.y, r.max.x, r.max.y]
}

fn rgb(c: [f32; 3], alpha: u8) -> Color32 {
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u8;
    Color32::from_rgba_unmultiplied(ch(c[0]), ch(c[1]), ch(c[2]), alpha)
}

fn color_of(elem: &CanvasElement) -> Option<[f32; 3]> {
    match elem {
        CanvasElement::Frame { color, .. } => Some(*color),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::element::CanvasElementId;

    #[test]
    fn thumb_collects_boxes_and_lines_with_bounds() {
        let mut doc = CanvasDocument::new("t");
        doc.add_element(CanvasElement::StickyNote {
            id: CanvasElementId::new(),
            pos: [10.0, 20.0],
            size: [100.0, 50.0],
            text: String::new(),
            color: [1.0, 0.9, 0.5],
            binding: None,
        });
        doc.add_element(CanvasElement::Connector {
            id: CanvasElementId::new(),
            from_elem: None,
            to_elem: None,
            from_pos: [0.0, 0.0],
            to_pos: [200.0, 100.0],
            routing: crate::canvas::element::ConnectorRouting::Straight,
            stroke_color: [0.0, 0.0, 0.0],
            stroke_width: 2.0,
            label: String::new(),
            arrow_end: true,
            waypoints: vec![[100.0, 0.0]],
        });
        let thumb = CanvasThumb::from_doc(&doc);
        assert_eq!(thumb.items.len(), 3);
        assert!(!thumb.is_empty());
        let b = thumb.bounds.unwrap();
        assert!(b[0] <= 0.0 && b[1] <= 0.0 && b[2] >= 200.0 && b[3] >= 100.0);
        assert!(CanvasThumb::from_doc(&CanvasDocument::new("empty")).is_empty());
    }
}
