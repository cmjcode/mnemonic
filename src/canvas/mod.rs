//! Edgeless Infinite Canvas Engine for AFFiNE-style visual whiteboarding in Rust.
//!
//! Provides the data structures, coordinate transforms, interactive toolbars,
//! and vector painters for a 2D spatial canvas that unifies with Markdown documents.

pub mod drawio;
pub mod element;
pub mod painter;
pub mod tools;
pub mod viewport;

pub use drawio::{DrawioExporter, DrawioImporter};
pub use element::{CanvasElement, CanvasElementId, ConnectorRouting, ShapeKind};
pub use painter::draw_element;
pub use tools::{CanvasTool, InteractionState};
pub use viewport::Viewport;

use std::collections::HashSet;
use uuid::Uuid;

use crate::block::{BlockKind, BlockTree};

/// A full 2D whiteboard canvas document.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CanvasDocument {
    pub id: Uuid,
    pub title: String,
    pub elements: Vec<CanvasElement>,
    pub viewport: Viewport,
}

impl Default for CanvasDocument {
    fn default() -> Self {
        CanvasDocument {
            id: Uuid::new_v4(),
            title: "Untitled Canvas".to_string(),
            elements: Vec::new(),
            viewport: Viewport::default(),
        }
    }
}

impl CanvasDocument {
    pub fn new(title: &str) -> Self {
        CanvasDocument {
            id: Uuid::new_v4(),
            title: title.to_string(),
            elements: Vec::new(),
            viewport: Viewport::default(),
        }
    }

    pub fn add_element(&mut self, elem: CanvasElement) -> CanvasElementId {
        let id = elem.id();
        self.elements.push(elem);
        id
    }

    pub fn remove_element(&mut self, id: CanvasElementId) -> Option<CanvasElement> {
        if let Some(pos) = self.elements.iter().position(|e| e.id() == id) {
            Some(self.elements.remove(pos))
        } else {
            None
        }
    }

    pub fn get_element(&self, id: CanvasElementId) -> Option<&CanvasElement> {
        self.elements.iter().find(|e| e.id() == id)
    }

    pub fn get_element_mut(&mut self, id: CanvasElementId) -> Option<&mut CanvasElement> {
        self.elements.iter_mut().find(|e| e.id() == id)
    }

    /// Find the topmost element hit by a world position.
    pub fn element_at(&self, world_pos: egui::Pos2) -> Option<&CanvasElement> {
        self.elements
            .iter()
            .rev()
            .find(|e| e.bounding_rect().contains(world_pos))
    }

    /// Find all element IDs enclosed or intersecting with a world rectangle.
    pub fn elements_in_rect(&self, world_rect: egui::Rect) -> HashSet<CanvasElementId> {
        self.elements
            .iter()
            .filter(|e| world_rect.intersects(e.bounding_rect()))
            .map(|e| e.id())
            .collect()
    }

    /// Automatically projects a `BlockTree` from Page Mode into a clean 2D layout on the whiteboard!
    pub fn from_block_tree(title: &str, tree: &BlockTree) -> Self {
        let mut canvas = CanvasDocument::new(title);
        let mut cur_y = 60.0;
        let left_x = 80.0;

        for &block_id in &tree.root_blocks {
            let Some(node) = tree.get(block_id) else {
                continue;
            };

            match &node.kind {
                BlockKind::Heading { text, level } => {
                    let width = (text.len() as f32 * 12.0).clamp(240.0, 500.0);
                    let height = 50.0;
                    canvas.add_element(CanvasElement::Shape {
                        id: CanvasElementId::new(),
                        kind: ShapeKind::RoundedRect,
                        rect: [left_x, cur_y, left_x + width, cur_y + height],
                        stroke_color: if *level == 1 {
                            tools::PALETTE_PRIMARY_ACCENT
                        } else {
                            tools::PALETTE_STROKE_LIGHT
                        },
                        stroke_width: 2.0,
                        fill_color: Some([0.18, 0.22, 0.32]),
                        text: text.clone(),
                        text_color: None,
                    });
                    cur_y += height + 35.0;
                }
                BlockKind::Checklist { checked, text } => {
                    let mark = if *checked { "✓ " } else { "☐ " };
                    canvas.add_element(CanvasElement::StickyNote {
                        id: CanvasElementId::new(),
                        pos: [left_x + 30.0, cur_y],
                        size: [220.0, 110.0],
                        text: format!("{}{}", mark, text),
                        color: if *checked {
                            tools::PALETTE_STICKY_GREEN
                        } else {
                            tools::PALETTE_STICKY_YELLOW
                        },
                    });
                    cur_y += 130.0;
                }
                BlockKind::Callout { kind, text } => {
                    canvas.add_element(CanvasElement::StickyNote {
                        id: CanvasElementId::new(),
                        pos: [left_x, cur_y],
                        size: [260.0, 140.0],
                        text: format!("[{}]\n{}", kind.to_uppercase(), text),
                        color: tools::PALETTE_STICKY_ORANGE,
                    });
                    cur_y += 160.0;
                }
                BlockKind::Paragraph(text) => {
                    if !text.trim().is_empty() {
                        canvas.add_element(CanvasElement::StickyNote {
                            id: CanvasElementId::new(),
                            pos: [left_x, cur_y],
                            size: [240.0, 130.0],
                            text: text.clone(),
                            color: tools::PALETTE_STICKY_BLUE,
                        });
                        cur_y += 150.0;
                    }
                }
                _ => {}
            }
        }

        canvas
    }

    /// Converts canvas elements into a clean hierarchical `BlockTree` sorted by reading order (top-to-bottom).
    pub fn to_block_tree(&self) -> BlockTree {
        let mut tree = BlockTree::default();

        // Sort elements by top-to-bottom (min y), then left-to-right (min x)
        let mut sorted_elements = self.elements.clone();
        sorted_elements.sort_by(|a, b| {
            let (ay, ax) = match a {
                CanvasElement::StickyNote { pos, .. } => (pos[1], pos[0]),
                CanvasElement::Shape { rect, .. } => (rect[1], rect[0]),
                CanvasElement::DocCard { pos, .. } => (pos[1], pos[0]),
                CanvasElement::Frame { rect, .. } => (rect[1], rect[0]),
                CanvasElement::Connector { from_pos, .. } => (from_pos[1], from_pos[0]),
                CanvasElement::FreehandStroke { points, .. } => {
                    points.first().map(|p| (p[1], p[0])).unwrap_or((0.0, 0.0))
                }
            };
            let (by, bx) = match b {
                CanvasElement::StickyNote { pos, .. } => (pos[1], pos[0]),
                CanvasElement::Shape { rect, .. } => (rect[1], rect[0]),
                CanvasElement::DocCard { pos, .. } => (pos[1], pos[0]),
                CanvasElement::Frame { rect, .. } => (rect[1], rect[0]),
                CanvasElement::Connector { from_pos, .. } => (from_pos[1], from_pos[0]),
                CanvasElement::FreehandStroke { points, .. } => {
                    points.first().map(|p| (p[1], p[0])).unwrap_or((0.0, 0.0))
                }
            };
            ay.partial_cmp(&by)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| ax.partial_cmp(&bx).unwrap_or(std::cmp::Ordering::Equal))
        });

        for elem in sorted_elements {
            match elem {
                CanvasElement::Shape { text, .. } => {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        tree.add_root_block(crate::block::BlockNode::new(BlockKind::Heading {
                            level: 1,
                            text: trimmed.to_string(),
                        }));
                    }
                }
                CanvasElement::StickyNote { text, .. } => {
                    let trimmed = text.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if trimmed.starts_with("✓ ") || trimmed.starts_with("- [x] ") || trimmed.starts_with("- [X] ") {
                        let content = if trimmed.starts_with("✓ ") {
                            trimmed[3..].trim()
                        } else {
                            trimmed[6..].trim()
                        };
                        tree.add_root_block(crate::block::BlockNode::new(BlockKind::Checklist {
                            checked: true,
                            text: content.to_string(),
                        }));
                    } else if trimmed.starts_with("☐ ") || trimmed.starts_with("- [ ] ") {
                        let content = if trimmed.starts_with("☐ ") {
                            trimmed[3..].trim()
                        } else {
                            trimmed[6..].trim()
                        };
                        tree.add_root_block(crate::block::BlockNode::new(BlockKind::Checklist {
                            checked: false,
                            text: content.to_string(),
                        }));
                    } else if trimmed.starts_with('[') && trimmed.contains(']') {
                        if let Some(close_idx) = trimmed.find(']') {
                            let kind = trimmed[1..close_idx].to_lowercase();
                            let content = trimmed[close_idx + 1..].trim();
                            tree.add_root_block(crate::block::BlockNode::new(BlockKind::Callout {
                                kind,
                                text: content.to_string(),
                            }));
                        } else {
                            tree.add_root_block(crate::block::BlockNode::new(BlockKind::Paragraph(trimmed.to_string())));
                        }
                    } else {
                        tree.add_root_block(crate::block::BlockNode::new(BlockKind::Paragraph(trimmed.to_string())));
                    }
                }
                CanvasElement::DocCard { title, .. } => {
                    tree.add_root_block(crate::block::BlockNode::new(BlockKind::Paragraph(format!("[[{title}]]"))));
                }
                CanvasElement::Frame { title, .. } => {
                    if !title.trim().is_empty() {
                        tree.add_root_block(crate::block::BlockNode::new(BlockKind::Heading {
                            level: 2,
                            text: title.trim().to_string(),
                        }));
                    }
                }
                _ => {}
            }
        }

        tree
    }

    /// Converts canvas elements into clean, human-readable Markdown (no raw JSON blocks).
    pub fn to_markdown_body(&self) -> String {
        let tree = self.to_block_tree();
        let md = tree.to_markdown();
        if md.trim().is_empty() {
            String::new()
        } else {
            format!("{}\n", md.trim())
        }
    }

    /// Try parsing a `CanvasDocument` from a Draw.io XML string.
    pub fn from_drawio_xml(title: &str, xml: &str) -> anyhow::Result<Self> {
        DrawioImporter::from_xml(title, xml)
    }

    /// Export this `CanvasDocument` into standard Draw.io XML format.
    pub fn to_drawio_xml(&self) -> String {
        DrawioExporter::to_xml(self)
    }

    /// Try parsing a `CanvasDocument` from markdown body, or convert from block tree if not a serialized canvas.
    pub fn from_markdown_body(title: &str, body: &str) -> Self {
        let trimmed = body.trim();
        // Check if body is Draw.io XML or ```drawio codeblock
        if trimmed.starts_with("<?xml")
            || trimmed.starts_with("<mxfile")
            || trimmed.starts_with("<mxGraphModel")
            || trimmed.contains("```drawio")
        {
            if let Ok(doc) = Self::from_drawio_xml(title, trimmed) {
                return doc;
            }
        }

        // Check if body contains legacy ```canvas ... ```
        if let Some(start_idx) = trimmed.find("```canvas") {
            let after_start = &trimmed[start_idx + 9..];
            if let Some(end_idx) = after_start.find("```") {
                let json_str = after_start[..end_idx].trim();
                if let Ok(mut doc) = serde_json::from_str::<CanvasDocument>(json_str) {
                    doc.title = title.to_string();
                    return doc;
                }
            }
        } else if trimmed.starts_with('{') && trimmed.ends_with('}') {
            if let Ok(mut doc) = serde_json::from_str::<CanvasDocument>(trimmed) {
                doc.title = title.to_string();
                return doc;
            }
        }

        // Standard: construct canvas from markdown block tree
        let tree = crate::block::BlockTree::from_markdown(body);
        Self::from_block_tree(title, &tree)
    }

    /// Extract all textual content across sticky notes, shapes, and connector labels for search & previews.
    pub fn extract_searchable_text(&self) -> String {
        let mut texts = Vec::new();
        for elem in &self.elements {
            match elem {
                CanvasElement::StickyNote { text, .. } => {
                    if !text.trim().is_empty() {
                        texts.push(text.trim().to_string());
                    }
                }
                CanvasElement::Shape { text, .. } => {
                    if !text.trim().is_empty() {
                        texts.push(text.trim().to_string());
                    }
                }
                CanvasElement::Connector { label, .. } => {
                    if !label.trim().is_empty() {
                        texts.push(label.trim().to_string());
                    }
                }
                _ => {}
            }
        }
        texts.join("\n")
    }

    /// Summary description of elements on the canvas (e.g. "3 Sticky Note · 2 Bentuk")
    pub fn summary_text(&self) -> String {
        let mut notes_count = 0;
        let mut shapes_count = 0;
        let mut connectors_count = 0;
        let mut strokes_count = 0;

        for elem in &self.elements {
            match elem {
                CanvasElement::StickyNote { .. } => notes_count += 1,
                CanvasElement::Shape { .. } => shapes_count += 1,
                CanvasElement::Connector { .. } => connectors_count += 1,
                CanvasElement::FreehandStroke { .. } => strokes_count += 1,
                _ => {}
            }
        }

        let mut parts = Vec::new();
        if notes_count > 0 {
            parts.push(format!("{notes_count} Sticky Note"));
        }
        if shapes_count > 0 {
            parts.push(format!("{shapes_count} Bentuk"));
        }
        if connectors_count > 0 {
            parts.push(format!("{connectors_count} Konektor"));
        }
        if strokes_count > 0 {
            parts.push(format!("{strokes_count} Coretan"));
        }

        if parts.is_empty() {
            "Kanvas Kosong".to_string()
        } else {
            parts.join(" · ")
        }
    }

    /// Converts canvas elements into clean, formatted readable Markdown prose for Page Mode view.
    pub fn to_readable_markdown(&self) -> String {
        let mut sections = Vec::new();
        for elem in &self.elements {
            match elem {
                CanvasElement::StickyNote { text, .. } => {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        sections.push(format!("> [!note]\n> {}", trimmed.replace('\n', "\n> ")));
                    }
                }
                CanvasElement::Shape { text, kind, .. } => {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        sections.push(format!("### {:?}\n\n{}", kind, trimmed));
                    }
                }
                CanvasElement::Connector { label, .. } => {
                    let trimmed = label.trim();
                    if !trimmed.is_empty() {
                        sections.push(format!("*Konektor: {}*", trimmed));
                    }
                }
                _ => {}
            }
        }

        if sections.is_empty() {
            String::new()
        } else {
            sections.join("\n\n")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_add_and_hit_test() {
        let mut canvas = CanvasDocument::new("Test Canvas");
        let elem_id = canvas.add_element(CanvasElement::StickyNote {
            id: CanvasElementId::new(),
            pos: [100.0, 100.0],
            size: [200.0, 150.0],
            text: "Hello Rust".to_string(),
            color: tools::PALETTE_STICKY_YELLOW,
        });

        assert_eq!(canvas.elements.len(), 1);
        let hit = canvas.element_at(egui::Pos2::new(150.0, 150.0));
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().id(), elem_id);

        let miss = canvas.element_at(egui::Pos2::new(50.0, 50.0));
        assert!(miss.is_none());
    }

    #[test]
    fn auto_layout_from_block_tree() {
        let md = "# System Design\n\n- [ ] Database setup\n\n- [x] Rust backend";
        let tree = BlockTree::from_markdown(md);
        let canvas = CanvasDocument::from_block_tree("System Design", &tree);

        assert_eq!(canvas.elements.len(), 3);
    }

    #[test]
    fn canvas_remove_and_rect_query() {
        let mut canvas = CanvasDocument::new("Test Canvas");
        let elem1 = canvas.add_element(CanvasElement::StickyNote {
            id: CanvasElementId::new(),
            pos: [50.0, 50.0],
            size: [100.0, 100.0],
            text: "Note 1".to_string(),
            color: tools::PALETTE_STICKY_BLUE,
        });

        let elem2 = canvas.add_element(CanvasElement::StickyNote {
            id: CanvasElementId::new(),
            pos: [400.0, 400.0],
            size: [100.0, 100.0],
            text: "Note 2".to_string(),
            color: tools::PALETTE_STICKY_GREEN,
        });

        let in_rect = canvas.elements_in_rect(egui::Rect::from_min_max(
            egui::pos2(0.0, 0.0),
            egui::pos2(200.0, 200.0),
        ));
        assert!(in_rect.contains(&elem1));
        assert!(!in_rect.contains(&elem2));

        let removed = canvas.remove_element(elem1);
        assert!(removed.is_some());
        assert_eq!(canvas.elements.len(), 1);
        assert_eq!(canvas.elements[0].id(), elem2);
    }

    #[test]
    fn test_canvas_markdown_roundtrip() {
        let mut canvas = CanvasDocument::new("Architecture Diagram");
        canvas.add_element(CanvasElement::StickyNote {
            id: CanvasElementId::new(),
            pos: [100.0, 150.0],
            size: [200.0, 120.0],
            text: "Backend API".to_string(),
            color: tools::PALETTE_STICKY_YELLOW,
        });
        canvas.add_element(CanvasElement::Shape {
            id: CanvasElementId::new(),
            kind: ShapeKind::RoundedRect,
            rect: [350.0, 150.0, 500.0, 250.0],
            stroke_color: tools::PALETTE_PRIMARY_ACCENT,
            stroke_width: 2.0,
            fill_color: None,
            text: "Database PostgreSQL".to_string(),
            text_color: None,
        });

        let md_body = canvas.to_markdown_body();
        assert!(md_body.contains("Backend API"));
        assert!(md_body.contains("Database PostgreSQL"));
        assert!(!md_body.contains("```canvas"));

        let loaded = CanvasDocument::from_markdown_body("Architecture Diagram", &md_body);
        assert_eq!(loaded.elements.len(), 2);
        assert_eq!(loaded.title, "Architecture Diagram");

        let text = loaded.extract_searchable_text();
        assert!(text.contains("Backend API"));
        assert!(text.contains("Database PostgreSQL"));

        let summary = loaded.summary_text();
        assert!(summary.contains("Sticky Note"));
        assert!(summary.contains("Bentuk"));
    }
}
