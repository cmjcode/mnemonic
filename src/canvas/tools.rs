//! Interaction tools and state machines for the infinite whiteboard canvas.

use super::element::ShapeKind;

/// Active interaction mode on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasTool {
    Select,
    Pan,
    StickyNote,
    Shape(ShapeKind),
    Connector,
    Pen,
    Eraser,
}

impl Default for CanvasTool {
    fn default() -> Self {
        CanvasTool::Select
    }
}

/// Curated palette of soft, modern colors for sticky notes and shapes.
pub const PALETTE_STICKY_YELLOW: [f32; 3] = [1.0, 0.94, 0.55];
pub const PALETTE_STICKY_BLUE: [f32; 3] = [0.65, 0.85, 1.0];
pub const PALETTE_STICKY_GREEN: [f32; 3] = [0.68, 0.94, 0.72];
pub const PALETTE_STICKY_PINK: [f32; 3] = [1.0, 0.75, 0.85];
pub const PALETTE_STICKY_PURPLE: [f32; 3] = [0.85, 0.75, 1.0];
pub const PALETTE_STICKY_ORANGE: [f32; 3] = [1.0, 0.82, 0.60];
pub const PALETTE_PRIMARY_ACCENT: [f32; 3] = [0.40, 0.65, 1.0];
pub const PALETTE_STROKE_DARK: [f32; 3] = [0.25, 0.28, 0.35];
pub const PALETTE_STROKE_LIGHT: [f32; 3] = [0.85, 0.88, 0.95];

/// Drag/interaction state while manipulating the canvas.
#[derive(Debug, Clone, Default)]
pub struct InteractionState {
    pub active_tool: CanvasTool,
    pub primary_color: [f32; 3],
    pub stroke_width: f32,
    pub is_dragging: bool,
    pub drag_start_world: Option<[f32; 2]>,
    pub drag_current_world: Option<[f32; 2]>,
    pub current_freehand_points: Vec<[f32; 2]>,
    pub editing_text_elem: Option<super::element::CanvasElementId>,
    /// Element grabbed by the current Select-tool drag, if any. Resolved once
    /// at drag start so the hit test doesn't run every frame.
    pub dragged_elem: Option<super::element::CanvasElementId>,
    /// Fit the viewport to the document bounds on the next frame the canvas
    /// is shown (needs the screen size, which only the surface knows).
    pub pending_fit: bool,
}

impl InteractionState {
    pub fn new() -> Self {
        InteractionState {
            active_tool: CanvasTool::Select,
            primary_color: PALETTE_STICKY_YELLOW,
            stroke_width: 2.0,
            is_dragging: false,
            drag_start_world: None,
            drag_current_world: None,
            current_freehand_points: Vec::new(),
            editing_text_elem: None,
            dragged_elem: None,
            pending_fit: false,
        }
    }
}
