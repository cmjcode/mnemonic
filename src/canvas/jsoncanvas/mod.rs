//! Obsidian **JSON Canvas 1.0** sidecar (`Judul.canvas`) support.
//!
//! A note is `Judul.md` (text, source of truth) plus an optional `Judul.canvas`. Text of
//! a *bound* element (see [`BlockBinding`]) is never stored in the canvas: it is derived
//! from the markdown block whose anchor is `^<block_id>`, so the editor re-derives it
//! with [`CanvasDocument::refresh_bound_text`] after markdown edits. Diagram-only
//! elements (arrows, decorative shapes, frames) never appear in the markdown.

pub mod convert;
pub mod model;

pub use convert::{from_json_canvas, parse_canvas_color, to_json_canvas, BlockResolver};
pub use model::{
    JcEdge, JcEdgeExt, JcExtension, JcFreeConnector, JcNode, JcNodeExt, JcNodeKind, JcStroke, JcViewport,
    JsonCanvas,
};

use anyhow::Context;

use super::element::BlockBinding;
use super::CanvasDocument;

impl CanvasDocument {
    /// Serialize as a JSON Canvas file (pretty, 2-space indent, trailing newline).
    /// `owner_note` is the vault-relative path of the note that owns the canvas.
    pub fn to_json_canvas_string(&self, owner_note: Option<&str>) -> String {
        let jc = to_json_canvas(self, owner_note);
        let mut s = serde_json::to_string_pretty(&jc).unwrap_or_else(|_| "{\"nodes\":[],\"edges\":[]}".to_string());
        s.push('\n');
        s
    }

    /// Parse a JSON Canvas file. Bound elements get their text from `resolve`.
    pub fn from_json_canvas_str(
        json: &str,
        title: &str,
        owner_note: Option<&str>,
        resolve: &BlockResolver<'_>,
    ) -> anyhow::Result<CanvasDocument> {
        let jc: JsonCanvas = serde_json::from_str(json).context("invalid JSON Canvas file")?;
        Ok(from_json_canvas(&jc, title, owner_note, resolve))
    }

    /// Every distinct block binding used by the canvas, in element order.
    pub fn bound_block_ids(&self) -> Vec<BlockBinding> {
        let mut out: Vec<BlockBinding> = Vec::new();
        for b in self.elements.iter().filter_map(|e| e.binding()) {
            if !out.contains(b) {
                out.push(b.clone());
            }
        }
        out
    }

    /// Re-derive the text of every bound element from the markdown. Elements whose
    /// block no longer resolves keep their current text. Returns whether anything changed.
    pub fn refresh_bound_text(&mut self, resolve: &BlockResolver<'_>) -> bool {
        self.refresh_bound_text_except(resolve, None)
    }

    /// [`Self::refresh_bound_text`], leaving `skip` (the element being
    /// typed in, whose buffer is ahead of the Markdown) alone.
    pub fn refresh_bound_text_except(
        &mut self,
        resolve: &BlockResolver<'_>,
        skip: Option<super::CanvasElementId>,
    ) -> bool {
        let mut changed = false;
        for elem in &mut self.elements {
            if Some(elem.id()) == skip {
                continue;
            }
            let Some(binding) = elem.binding() else { continue };
            let Some(new_text) = resolve(binding) else { continue };
            if elem.text() != Some(new_text.as_str()) {
                elem.set_text(new_text);
                changed = true;
            }
        }
        changed
    }
}
