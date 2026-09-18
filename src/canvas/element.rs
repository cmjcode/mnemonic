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

/// Link between a canvas element and a markdown block (`^<block_id>` anchor).
///
/// The text of a bound element is *derived* from the markdown block, never stored in
/// the canvas file; the canvas only remembers which block it mirrors.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct BlockBinding {
    /// Vault-relative note path (`None` = the note that owns this canvas).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Block anchor id without the leading `^`.
    pub block_id: String,
}

impl BlockBinding {
    /// Binding to a block of the note that owns the canvas.
    pub fn local(block_id: impl Into<String>) -> Self {
        BlockBinding {
            file: None,
            block_id: block_id.into(),
        }
    }

    /// Binding to a block of another note in the vault.
    pub fn to_file(file: impl Into<String>, block_id: impl Into<String>) -> Self {
        BlockBinding {
            file: Some(file.into()),
            block_id: block_id.into(),
        }
    }

    /// Obsidian-style subpath: `#^<block_id>`.
    pub fn subpath(&self) -> String {
        format!("#^{}", self.block_id)
    }

    /// A fresh 6-character lowercase alphanumeric block id (Obsidian style) that is
    /// not in `taken`.
    pub fn generate_id(taken: &std::collections::HashSet<String>) -> String {
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
        loop {
            let id: String = Uuid::new_v4()
                .as_bytes()
                .iter()
                .take(6)
                .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
                .collect();
            if !taken.contains(&id) {
                return id;
            }
        }
    }
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
        /// Markdown block this note mirrors; `None` = diagram-only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        binding: Option<BlockBinding>,
    },
    Shape {
        id: CanvasElementId,
        kind: ShapeKind,
        rect: [f32; 4], // [min_x, min_y, max_x, max_y]
        stroke_color: [f32; 3],
        stroke_width: f32,
        fill_color: Option<[f32; 3]>,
        text: String,
        /// Explicit label color; `None` picks a color that contrasts with the fill.
        #[serde(default)]
        text_color: Option<[f32; 3]>,
        /// Markdown block this shape's label mirrors; `None` = diagram-only.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        binding: Option<BlockBinding>,
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
        /// Intermediate bend points between `from_pos` and `to_pos`.
        #[serde(default)]
        waypoints: Vec<[f32; 2]>,
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

    /// Markdown block binding of a sticky note or shape (`None` for diagram-only
    /// elements and for kinds that cannot be bound).
    pub fn binding(&self) -> Option<&BlockBinding> {
        match self {
            CanvasElement::StickyNote { binding, .. } | CanvasElement::Shape { binding, .. } => {
                binding.as_ref()
            }
            _ => None,
        }
    }

    /// Set (or clear) the block binding. No-op for kinds that cannot be bound.
    pub fn set_binding(&mut self, new_binding: Option<BlockBinding>) {
        if let CanvasElement::StickyNote { binding, .. } | CanvasElement::Shape { binding, .. } = self {
            *binding = new_binding;
        }
    }

    /// Whether this element mirrors a markdown block.
    pub fn is_bound(&self) -> bool {
        self.binding().is_some()
    }

    /// Textual content: sticky note / shape text, connector label, frame title,
    /// doc card title. `None` for freehand strokes.
    pub fn text(&self) -> Option<&str> {
        match self {
            CanvasElement::StickyNote { text, .. } | CanvasElement::Shape { text, .. } => Some(text),
            CanvasElement::Connector { label, .. } => Some(label),
            CanvasElement::Frame { title, .. } | CanvasElement::DocCard { title, .. } => Some(title),
            CanvasElement::FreehandStroke { .. } => None,
        }
    }

    /// Replace the textual content (see [`Self::text`]). No-op for freehand strokes.
    pub fn set_text(&mut self, new_text: String) {
        match self {
            CanvasElement::StickyNote { text, .. } | CanvasElement::Shape { text, .. } => *text = new_text,
            CanvasElement::Connector { label, .. } => *label = new_text,
            CanvasElement::Frame { title, .. } | CanvasElement::DocCard { title, .. } => *title = new_text,
            CanvasElement::FreehandStroke { .. } => {}
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
            CanvasElement::Connector { from_pos, to_pos, waypoints, .. } => {
                let mut min_x = from_pos[0].min(to_pos[0]);
                let mut min_y = from_pos[1].min(to_pos[1]);
                let mut max_x = from_pos[0].max(to_pos[0]);
                let mut max_y = from_pos[1].max(to_pos[1]);
                for pt in waypoints {
                    min_x = min_x.min(pt[0]);
                    min_y = min_y.min(pt[1]);
                    max_x = max_x.max(pt[0]);
                    max_y = max_y.max(pt[1]);
                }
                Rect::from_min_max(
                    Pos2::new(min_x - 5.0, min_y - 5.0),
                    Pos2::new(max_x + 5.0, max_y + 5.0),
                )
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
            CanvasElement::Connector { from_pos, to_pos, waypoints, .. } => {
                from_pos[0] += delta.x;
                from_pos[1] += delta.y;
                to_pos[0] += delta.x;
                to_pos[1] += delta.y;
                for pt in waypoints {
                    pt[0] += delta.x;
                    pt[1] += delta.y;
                }
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
