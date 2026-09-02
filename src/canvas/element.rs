//! Graphic elements and whiteboard primitives on the infinite 2D canvas.

use egui::{Pos2, Rect, Vec2};
use uuid::Uuid;

/// Unique identifier for an element on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CanvasElementId(pub Uuid);

impl CanvasElementId {
    pub fn new() -> Self {
        CanvasElementId(Uuid::new_v4())
    }
}

impl Default for CanvasElementId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for CanvasElementId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Geometric shape types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ShapeKind {
    Rectangle,
    RoundedRect,
    Ellipse,
    Diamond,
    CalloutBubble,
}

/// Connection line routing styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ConnectorRouting {
    Straight,
    Curved,
    Orthogonal,
}

/// All interactive elements that can live on the canvas.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum CanvasElement {
    StickyNote {
        id: CanvasElementId,
        pos: [f32; 2],
        size: [f32; 2],
        text: String,
        color: [f32; 3], // Pastel color (e.g. yellow, blue, green, pink)
    },
    Shape {
        id: CanvasElementId,
        kind: ShapeKind,
        rect: [f32; 4], // [min_x, min_y, max_x, max_y]
        stroke_color: [f32; 3],
        stroke_width: f32,
        fill_color: Option<[f32; 3]>,
        text: String,
    },
    Connector {
        id: CanvasElementId,
        from_elem: Option<CanvasElementId>,
        to_elem: Option<CanvasElementId>,
        from_pos: [f32; 2],
        to_pos: [f32; 2],
        routing: ConnectorRouting,
        stroke_color: [f32; 3],
        stroke_width: f32,
        label: String,
        arrow_end: bool,
    },
    DocCard {
        id: CanvasElementId,
        pos: [f32; 2],
        size: [f32; 2],
        note_id: Option<Uuid>,
        title: String,
        snippet: String,
        doc_type: String, // "note" | "pdf"
    },
    Frame {
        id: CanvasElementId,
        rect: [f32; 4],
        title: String,
        color: [f32; 3],
    },
    FreehandStroke {
        id: CanvasElementId,
        points: Vec<[f32; 2]>,
        color: [f32; 3],
        width: f32,
    },
}

impl CanvasElement {
    pub fn id(&self) -> CanvasElementId {
        match self {
            CanvasElement::StickyNote { id, .. }
            | CanvasElement::Shape { id, .. }
            | CanvasElement::Connector { id, .. }
            | CanvasElement::DocCard { id, .. }
            | CanvasElement::Frame { id, .. }
            | CanvasElement::FreehandStroke { id, .. } => *id,
        }
    }

    /// World-space bounding rectangle of the element.
    pub fn bounding_rect(&self) -> Rect {
        match self {
            CanvasElement::StickyNote { pos, size, .. } => {
                Rect::from_min_size(Pos2::new(pos[0], pos[1]), Vec2::new(size[0], size[1]))
            }
            CanvasElement::Shape { rect, .. } | CanvasElement::Frame { rect, .. } => {
                Rect::from_min_max(Pos2::new(rect[0], rect[1]), Pos2::new(rect[2], rect[3]))
            }
            CanvasElement::DocCard { pos, size, .. } => {
                Rect::from_min_size(Pos2::new(pos[0], pos[1]), Vec2::new(size[0], size[1]))
            }
            CanvasElement::Connector { from_pos, to_pos, .. } => {
                let min_x = from_pos[0].min(to_pos[0]) - 5.0;
                let min_y = from_pos[1].min(to_pos[1]) - 5.0;
                let max_x = from_pos[0].max(to_pos[0]) + 5.0;
                let max_y = from_pos[1].max(to_pos[1]) + 5.0;
                Rect::from_min_max(Pos2::new(min_x, min_y), Pos2::new(max_x, max_y))
            }
            CanvasElement::FreehandStroke { points, width, .. } => {
                if points.is_empty() {
                    return Rect::NOTHING;
                }
                let mut min_x = points[0][0];
                let mut min_y = points[0][1];
                let mut max_x = points[0][0];
                let mut max_y = points[0][1];
                for pt in points {
                    min_x = min_x.min(pt[0]);
                    min_y = min_y.min(pt[1]);
                    max_x = max_x.max(pt[0]);
                    max_y = max_y.max(pt[1]);
                }
                let pad = width * 0.5 + 2.0;
                Rect::from_min_max(
                    Pos2::new(min_x - pad, min_y - pad),
                    Pos2::new(max_x + pad, max_y + pad),
                )
            }
        }
    }

    /// Translate the element by a delta vector in world space.
    pub fn translate(&mut self, delta: Vec2) {
        match self {
            CanvasElement::StickyNote { pos, .. } | CanvasElement::DocCard { pos, .. } => {
                pos[0] += delta.x;
                pos[1] += delta.y;
            }
            CanvasElement::Shape { rect, .. } | CanvasElement::Frame { rect, .. } => {
                rect[0] += delta.x;
                rect[1] += delta.y;
                rect[2] += delta.x;
                rect[3] += delta.y;
            }
            CanvasElement::Connector { from_pos, to_pos, .. } => {
                from_pos[0] += delta.x;
                from_pos[1] += delta.y;
                to_pos[0] += delta.x;
                to_pos[1] += delta.y;
            }
            CanvasElement::FreehandStroke { points, .. } => {
                for pt in points {
                    pt[0] += delta.x;
                    pt[1] += delta.y;
                }
            }
        }
    }
}
