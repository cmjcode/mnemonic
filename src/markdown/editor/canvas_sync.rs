//! Notes ⇄ canvas sync for section canvases (§3.9.2): a plain note opened
//! as a canvas becomes one box per section segment (`markdown::sections`)
//! laid out as a mind map (`canvas::outline`). The Markdown stays the only
//! source of text:
//! - canvas → Markdown: only the box being edited writes back, and only
//!   while its text is still one segment of the same kind; a box whose
//!   edit splits it (typing a new `## heading`, a table…) writes back when
//!   the edit closes, then the new segments get anchors and boxes.
//! - Markdown → canvas: every body change re-derives bound text (except the
//!   box being typed in), adds boxes for new anchored segments, re-derives
//!   the outline edges and marks boxes whose segment vanished as orphans.
//!
//! New segments are anchored on save / edit close, never mid-keystroke.
//! Callers: `markdown::editor`, `app::editor::canvas_surface`.

use std::collections::{HashMap, HashSet};

use crate::canvas::{BindingScope, BlockBinding, CanvasDocument, CanvasElement, CanvasElementId, outline};
use crate::markdown::sections::{self, SegmentKind};

use super::{CanvasStorage, MarkdownEditor, blocks};

/// Whether `doc` is an older per-block canvas of this note: local boxes
/// bound to single `^block` lines and no section box. Such a canvas never
/// shows tables, code or Mermaid fences (they had no block anchor).
fn is_block_canvas(doc: &CanvasDocument) -> bool {
    !outline::is_section_canvas(doc)
        && doc.elements.iter().any(|e| e.binding().is_some_and(|b| b.file.is_none() && !b.is_segment()))
}

/// Replaces the local per-block boxes of `canvas` with the section boxes of
/// `fresh`. Diagram-only shapes and boxes bound to other notes stay; a
/// connector that touched a block box moves to the box of the section
/// holding that block (dropped when both ends land in one section, or when
/// the outline already links the two).
fn adopt_section_boxes(canvas: &mut CanvasDocument, fresh: CanvasDocument, body: &str) {
    let fresh_boxes = outline::segment_elements(&fresh);
    let mut moved: HashMap<CanvasElementId, CanvasElementId> = HashMap::new();
    for e in &canvas.elements {
        let Some(b) = e.binding().filter(|b| b.file.is_none()) else { continue };
        let target = body
            .lines()
            .position(|l| blocks::line_has_anchor(l, &b.block_id))
            .and_then(|line| sections::segment_at_line(body, line))
            .and_then(|seg| seg.id)
            .and_then(|id| fresh_boxes.get(&id).copied());
        if let Some(target) = target {
            moved.insert(e.id(), target);
        }
    }
    let mut linked: HashSet<(CanvasElementId, CanvasElementId)> = fresh
        .elements
        .iter()
        .filter_map(|e| match e {
            CanvasElement::Connector { from_elem: Some(f), to_elem: Some(t), .. } => Some((*f, *t)),
            _ => None,
        })
        .collect();
    canvas.elements.retain(|e| e.binding().is_none_or(|b| b.file.is_some()));
    let kept: HashSet<CanvasElementId> = canvas.elements.iter().map(|e| e.id()).collect();
    canvas.elements.retain_mut(|e| {
        let CanvasElement::Connector { from_elem, to_elem, .. } = e else { return true };
        for end in [&mut *from_elem, &mut *to_elem] {
            if let Some(id) = *end
                && !kept.contains(&id)
            {
                *end = moved.get(&id).copied();
            }
        }
        match (*from_elem, *to_elem) {
            (Some(f), Some(t)) => f != t && !linked.contains(&(t, f)) && linked.insert((f, t)),
            _ => true,
        }
    });
    canvas.elements.extend(fresh.elements);
}

/// Same kind of segment, ignoring heading level / fence language.
fn same_kind(a: &SegmentKind, b: &SegmentKind) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
}

impl MarkdownEditor {
    /// A section canvas for the current body: anchors every segment (one
    /// undoable body change) and lays the boxes out as a mind map.
    pub(super) fn build_section_canvas(&mut self) -> CanvasDocument {
        let (body, segs) = sections::anchor_all_segments(&self.note.body);
        if body != self.note.body {
            self.set_body_inner(body);
        }
        outline::build_outline_canvas(&self.note.frontmatter.title, &segs)
    }

    /// Whether the open canvas mirrors the note's sections.
    pub fn is_section_canvas(&self) -> bool {
        self.canvas.as_ref().is_some_and(outline::is_section_canvas)
    }

    /// Boxes whose section no longer exists in the Markdown.
    pub fn orphans(&self) -> &HashSet<CanvasElementId> {
        &self.orphans
    }

    /// Block id of a locally bound element.
    pub fn element_block_id(&self, id: CanvasElementId) -> Option<String> {
        let b = self.canvas.as_ref()?.get_element(id)?.binding()?;
        b.file.is_none().then(|| b.block_id.clone())
    }

    /// Canvas → Markdown for the element `id` after its text was edited.
    /// `closing` = the inline editor closed (split edits apply now).
    /// Returns whether the body changed.
    pub fn write_back_element(&mut self, id: CanvasElementId, closing: bool) -> bool {
        let Some(elem) = self.canvas.as_ref().and_then(|c| c.get_element(id)) else {
            return false;
        };
        let (Some(binding), Some(text)) = (elem.binding().cloned(), elem.text().map(str::to_string)) else {
            return false;
        };
        if binding.file.is_some() || !self.canvas_storage.is_sidecar() {
            return false; // another note's block: edit it there
        }
        let updated = match binding.scope {
            BindingScope::Block => blocks::replace_block_text(&self.note.body, &binding.block_id, &text),
            BindingScope::Segment => {
                let Some(seg) = sections::find_segment(&self.note.body, &binding.block_id) else {
                    return false;
                };
                let parsed = sections::segments(&text);
                let single = parsed.len() == 1 && same_kind(&parsed[0].kind, &seg.kind);
                if !single && !closing {
                    return false; // wait until the edit closes
                }
                sections::replace_segment_text(&self.note.body, &binding.block_id, &text)
            }
        };
        let changed = updated.is_some();
        if let Some(body) = updated {
            self.set_body(body);
        }
        if closing {
            self.anchor_new_segments();
            self.refresh_canvas_from_body();
        }
        changed
    }

    /// Gives new Markdown segments an anchor so they get a box (section
    /// canvases only). Returns whether the body changed.
    pub fn anchor_new_segments(&mut self) -> bool {
        if !self.canvas_storage.is_sidecar() || !self.is_section_canvas() {
            return false;
        }
        let (body, _) = sections::anchor_all_segments(&self.note.body);
        if body == self.note.body {
            return false;
        }
        self.set_body(body);
        true
    }

    /// Markdown → canvas structure: boxes for new anchored segments, outline
    /// edges, box growth for longer text, orphans. Returns whether the
    /// canvas changed.
    pub(super) fn reconcile_segments(&mut self, canvas: &mut CanvasDocument) -> bool {
        if !outline::is_section_canvas(canvas) {
            self.orphans.clear();
            return false;
        }
        let segs = sections::segments(&self.note.body);
        let mut changed = outline::add_missing_boxes(canvas, &segs) > 0;
        changed |= outline::sync_outline_edges(canvas, &segs);
        let live: HashSet<&str> = segs.iter().filter_map(|s| s.id.as_deref()).collect();
        let boxes = outline::segment_elements(canvas);
        self.orphans = boxes
            .iter()
            .filter(|(id, _)| !live.contains(id.as_str()))
            .map(|(_, e)| *e)
            .collect();
        // Grow (never shrink) boxes whose text got longer.
        for seg in &segs {
            let Some(eid) = seg.id.as_deref().and_then(|id| boxes.get(id)) else { continue };
            let want = outline::box_size(seg)[1];
            if let Some(crate::canvas::CanvasElement::Shape { rect, .. }) = canvas.get_element_mut(*eid)
                && rect[3] - rect[1] < want - 1.0
            {
                rect[3] = rect[1] + want;
                changed = true;
            }
        }
        if changed {
            outline::reattach_connectors(canvas, None);
        }
        changed
    }

    /// An older per-block sidecar becomes a section canvas as soon as it is
    /// opened, so its tables, code and Mermaid fences get boxes too
    /// (§3.9.2). Returns whether it was converted.
    pub(super) fn upgrade_block_canvas(&mut self) -> bool {
        if !self.canvas_storage.is_sidecar() || !self.canvas.as_ref().is_some_and(is_block_canvas) {
            return false;
        }
        log::info!("editor: converting per-block canvas of {} to sections", self.note.path.display());
        self.tidy_canvas();
        true
    }

    /// Re-lays out the section boxes as a mind map. A canvas made of the
    /// older per-block boxes is converted first (its local block boxes are
    /// replaced by section boxes; diagram-only shapes stay).
    pub fn tidy_canvas(&mut self) {
        if !self.canvas_storage.is_sidecar() && self.canvas_storage != CanvasStorage::Markdown {
            return;
        }
        self.ensure_canvas();
        if !self.is_section_canvas() {
            let fresh = self.build_section_canvas();
            if let Some(canvas) = self.canvas.as_mut() {
                adopt_section_boxes(canvas, fresh, &self.note.body);
            }
            self.canvas_storage = CanvasStorage::Sidecar;
        } else {
            self.anchor_new_segments();
        }
        let segs = sections::segments(&self.note.body);
        if let Some(mut canvas) = self.canvas.take() {
            canvas.hidden_segments.clear();
            self.reconcile_segments(&mut canvas);
            outline::relayout(&mut canvas, &segs);
            self.canvas = Some(canvas);
        }
        self.canvas_interaction.pending_fit = true;
        self.mark_sidecar_dirty();
    }

    /// The user removed segment-bound boxes from the canvas: keep them out
    /// of the outline sync. Their Markdown is untouched.
    pub fn hide_segments(&mut self, bindings: &[BlockBinding]) {
        let Some(canvas) = self.canvas.as_mut() else { return };
        for b in bindings.iter().filter(|b| b.is_segment() && b.file.is_none()) {
            if !canvas.hidden_segments.contains(&b.block_id) {
                canvas.hidden_segments.push(b.block_id.clone());
            }
        }
        self.mark_sidecar_dirty();
    }

    /// Deletes the element and, for a segment box, its section from the
    /// Markdown (an explicit, undoable action).
    pub fn delete_element_from_note(&mut self, id: CanvasElementId) -> bool {
        let Some(binding) = self.canvas.as_ref().and_then(|c| c.get_element(id)).and_then(|e| e.binding().cloned())
        else {
            return false;
        };
        if binding.file.is_some() || !binding.is_segment() {
            return false;
        }
        if let Some(canvas) = self.canvas.as_mut() {
            canvas.remove_element(id);
        }
        if let Some(body) = sections::remove_segment(&self.note.body, &binding.block_id) {
            self.set_body(body);
        }
        self.mark_sidecar_dirty();
        true
    }

    /// Appends `text` to the note as a new section/segment and puts its box
    /// at world position `pos`. Returns the new element.
    pub fn add_section_box(&mut self, text: &str, pos: [f32; 2]) -> Option<CanvasElementId> {
        self.ensure_canvas();
        let (body, id) = sections::append_segment(&self.note.body, text);
        let seg = sections::find_segment(&body, &id)?;
        let size = outline::box_size(&seg);
        let elem = outline::segment_box(&seg, pos, size);
        let eid = elem.id();
        self.canvas.as_mut()?.add_element(elem);
        self.set_body(body);
        self.mark_sidecar_dirty();
        Some(eid)
    }

    /// Brings back every box the user hid.
    pub fn show_hidden_segments(&mut self) {
        if let Some(mut canvas) = self.canvas.take() {
            canvas.hidden_segments.clear();
            self.reconcile_segments(&mut canvas);
            self.canvas = Some(canvas);
        }
        self.mark_sidecar_dirty();
    }

    fn mark_sidecar_dirty(&mut self) {
        self.sidecar_dirty = true;
        self.dirty = true;
        self.pending_since = Some(std::time::Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::canvas::{CanvasElement, CanvasElementId};
    use crate::markdown::sections;
    use tempfile::tempdir;

    const BODY: &str = "# Produk\nVisi singkat.\n\n## Data\nPenjelasan.\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n## Tim\nOrang.\n";

    fn open_section_canvas() -> (tempfile::TempDir, MarkdownEditor) {
        let dir = tempdir().unwrap();
        let note = Note::create(dir.path(), "Proyek", BODY).unwrap();
        let mut e = MarkdownEditor::open_in(note, Some(dir.path()));
        e.mode = EditorMode::Split;
        e.ensure_canvas();
        (dir, e)
    }

    fn box_of(e: &MarkdownEditor, needle: &str) -> CanvasElementId {
        e.canvas
            .as_ref()
            .unwrap()
            .elements
            .iter()
            .find(|el| el.is_bound() && el.text().is_some_and(|t| t.contains(needle)))
            .map(|el| el.id())
            .unwrap_or_else(|| panic!("no box with {needle:?}"))
    }

    fn text_of(e: &MarkdownEditor, id: CanvasElementId) -> String {
        e.canvas.as_ref().unwrap().get_element(id).unwrap().text().unwrap().to_string()
    }

    #[test]
    fn plain_note_becomes_one_box_per_section_with_outline_edges() {
        let (_d, e) = open_section_canvas();
        assert!(e.is_section_canvas());
        let canvas = e.canvas.as_ref().unwrap();
        let boxes = canvas.elements.iter().filter(|el| el.is_bound()).count();
        assert_eq!(boxes, 4, "Produk, Data, table, Tim");
        let edges = canvas
            .elements
            .iter()
            .filter(|el| matches!(el, CanvasElement::Connector { meta, .. } if meta.outline))
            .count();
        assert_eq!(edges, 3);
        assert!(e.note.body.contains("# Produk ^"), "{}", e.note.body);
    }

    #[test]
    fn editing_a_box_rewrites_only_its_section_and_markdown_edits_flow_back() {
        let (_d, mut e) = open_section_canvas();
        let data = box_of(&e, "## Data");
        e.canvas_interaction.editing_text_elem = Some(data);
        e.canvas.as_mut().unwrap().get_element_mut(data).unwrap().set_text("## Data\nPenjelasan baru.".into());
        assert!(e.write_back_element(data, false));
        assert!(e.note.body.contains("Penjelasan baru."));
        assert!(e.note.body.contains("## Tim ^") && e.note.body.contains("Orang."));
        assert!(e.note.body.contains("| 1 | 2 |"), "table untouched");

        // Markdown → canvas.
        e.canvas_interaction.editing_text_elem = None;
        let body = e.note.body.replace("Orang.", "Orang-orang hebat.");
        e.set_body(body);
        let tim = box_of(&e, "## Tim");
        assert!(text_of(&e, tim).contains("hebat"));
    }

    #[test]
    fn splitting_a_box_waits_for_close_then_adds_a_box() {
        let (_d, mut e) = open_section_canvas();
        let tim = box_of(&e, "## Tim");
        e.canvas_interaction.editing_text_elem = Some(tim);
        e.canvas.as_mut().unwrap().get_element_mut(tim).unwrap().set_text("## Tim\nOrang.\n### Peran\nCTO".into());
        assert!(!e.write_back_element(tim, false), "split edit deferred");
        e.canvas_interaction.editing_text_elem = None;
        assert!(e.write_back_element(tim, true));
        assert_eq!(e.note.body.matches("### Peran").count(), 1, "{}", e.note.body);
        box_of(&e, "### Peran");
        // The Tim box now holds only its own run.
        assert_eq!(text_of(&e, tim), "## Tim\nOrang.");
    }

    #[test]
    fn new_markdown_section_gets_a_box_on_save_and_undo_refreshes_canvas() {
        let (_d, mut e) = open_section_canvas();
        let body = format!("{}\n## Risiko\nPasar.\n", e.note.body);
        e.set_body(body);
        e.autosave().unwrap();
        box_of(&e, "## Risiko");
        assert!(e.note.sidecar_path().exists());
        // Undo must also refresh bound text, or a save would write stale text back.
        let data = box_of(&e, "## Data");
        e.canvas.as_mut().unwrap().get_element_mut(data).unwrap().set_text("## Data\nX".into());
        e.write_back_element(data, true);
        e.undo();
        assert!(text_of(&e, data).contains("Penjelasan."), "{}", text_of(&e, data));
    }

    #[test]
    fn removed_section_turns_its_box_into_an_orphan_and_hidden_boxes_stay_hidden() {
        let (_d, mut e) = open_section_canvas();
        let tim = box_of(&e, "## Tim");
        let id = e.element_block_id(tim).unwrap();
        let body = sections::remove_segment(&e.note.body, &id).unwrap();
        e.set_body(body);
        assert!(e.orphans().contains(&tim));

        let data = box_of(&e, "## Data");
        let binding = e.canvas.as_ref().unwrap().get_element(data).unwrap().binding().cloned().unwrap();
        e.canvas.as_mut().unwrap().remove_element(data);
        e.hide_segments(&[binding]);
        e.set_body(format!("{}\nlagi\n", e.note.body));
        assert!(
            e.canvas
                .as_ref()
                .unwrap()
                .elements
                .iter()
                .all(|el| el.text().is_none_or(|t| !t.starts_with("## Data")))
        );
    }

    #[test]
    fn delete_from_note_and_add_box() {
        let (_d, mut e) = open_section_canvas();
        let tim = box_of(&e, "## Tim");
        assert!(e.delete_element_from_note(tim));
        assert!(!e.note.body.contains("## Tim"));
        let new = e.add_section_box("## Ide\nCatatan baru", [900.0, 40.0]).unwrap();
        assert!(e.note.body.contains("## Ide ^"));
        let r = e.canvas.as_ref().unwrap().get_element(new).unwrap().bounding_rect();
        assert_eq!(r.min.x, 900.0);
    }

    #[test]
    fn block_sidecar_is_upgraded_on_open_and_shows_mermaid() {
        let dir = tempdir().unwrap();
        let body = "# A ^aaaaaa\nisi ^bbbbbb\n\n## ER ^cccccc\n\n```mermaid\nerDiagram\n  X ||--o{ Y : has\n```\n";
        let note = Note::create(dir.path(), "Skema", body).unwrap();
        let file = |id: &str, block: &str, y: i32| {
            format!(
                r##"{{"id":"{id}","type":"file","x":0,"y":{y},"width":240,"height":56,"file":"Skema.md","subpath":"#^{block}","mnemonic":{{"kind":"shape","shape":"RoundedRect"}}}}"##
            )
        };
        let json = format!(
            r#"{{"nodes":[{},{},{}],"edges":[{{"id":"e1","fromNode":"n1","toNode":"n3"}},{{"id":"e2","fromNode":"n1","toNode":"n2"}}]}}"#,
            file("n1", "aaaaaa", 0),
            file("n2", "bbbbbb", 80),
            file("n3", "cccccc", 160),
        );
        std::fs::write(note.sidecar_path(), json).unwrap();
        let mut e = MarkdownEditor::open_in(note, Some(dir.path()));
        e.ensure_canvas();
        assert!(e.is_section_canvas());
        let diagram = box_of(&e, "erDiagram");
        assert!(text_of(&e, diagram).starts_with("```mermaid"));
        // The fence got its anchor on the line below.
        let segs = sections::segments(&e.note.body);
        assert!(segs.iter().any(|s| s.kind == sections::SegmentKind::Mermaid && s.id.is_some()));
        // No per-block box is left; e1 duplicates the outline edge and e2
        // collapsed into one section, so only outline edges remain.
        let canvas = e.canvas.as_ref().unwrap();
        assert!(canvas.elements.iter().all(|el| el.binding().is_none_or(|b| b.is_segment())));
        let connectors = canvas.elements.iter().filter(|el| matches!(el, CanvasElement::Connector { .. })).count();
        assert_eq!(connectors, 2);
    }

    #[test]
    fn tidy_converts_a_block_canvas() {
        let dir = tempdir().unwrap();
        let note = Note::create_canvas(dir.path(), "Lama").unwrap();
        let mut e = MarkdownEditor::open_in(note, Some(dir.path()));
        assert!(!e.is_section_canvas());
        e.tidy_canvas();
        assert!(e.is_section_canvas());
    }
}
