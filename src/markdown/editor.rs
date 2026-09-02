//! Editor session state for a single open note (§3.2): mode switching
//! (Source / Live Preview / Reading / Edgeless Canvas), debounced autosave, coarse undo/redo,
//! and word count / reading time. Slash-command and wikilink-autocomplete
//! trigger detection are pure string functions here too, so the popup
//! logic in `app.rs` stays thin. Also owns a `renderer::RenderCache`
//! (§Fase 10) — invalidated in `set_body`/`undo`/`redo` and read via
//! `outline()`/`render()`, so the Live Preview/Reading long-document
//! memoization lives right next to the only code that mutates the body it's
//! keyed on. Callers: `app.rs`.

use std::time::{Duration, Instant};

use anyhow::Result;
use egui_commonmark::CommonMarkCache;

use crate::block::BlockTree;
use crate::canvas::{CanvasDocument, InteractionState};
use crate::notes::Note;

use super::renderer::{self, Heading, RenderCache, RenderOutcome};

/// Idle window before an edit is flushed to disk (§3.2.4: "debounce
/// 500ms-1s").
pub const AUTOSAVE_DEBOUNCE: Duration = Duration::from_millis(800);
const WORDS_PER_MINUTE: usize = 200;
/// Caps memory use of the undo stack; old snapshots are dropped, not the
/// ability to undo recent edits.
const MAX_UNDO_HISTORY: usize = 100;

/// How the note body is currently presented.
/// - `Source`: raw Markdown text editor.
/// - `LivePreview`: interactive rendered CommonMark view with inline checklists & wikilinks.
/// - `Reading`: distraction-free reading mode.
/// - `Edgeless`: infinite 2D spatial canvas / whiteboard representation (AFFiNE-style dual mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    Source,
    LivePreview,
    Reading,
    Edgeless,
}

/// An open editing session for one note.
pub struct MarkdownEditor {
    pub note: Note,
    pub mode: EditorMode,
    dirty: bool,
    pending_since: Option<Instant>,
    undo_stack: Vec<String>,
    redo_stack: Vec<String>,
    /// Memoized Live Preview/Reading parse of `note.body` (§Fase 10),
    /// invalidated on every body mutation below so it never goes stale.
    render_cache: RenderCache,
    /// Edgeless infinite canvas state when in `EditorMode::Edgeless`.
    pub canvas: Option<CanvasDocument>,
    /// Interaction state for canvas manipulation.
    pub canvas_interaction: InteractionState,
}

impl MarkdownEditor {
    pub fn open(note: Note) -> MarkdownEditor {
        let is_canvas = note.is_canvas();
        let initial_mode = if is_canvas {
            EditorMode::Edgeless
        } else {
            EditorMode::LivePreview
        };

        let canvas = if is_canvas {
            Some(CanvasDocument::from_markdown_body(
                &note.frontmatter.title,
                &note.body,
            ))
        } else {
            None
        };

        MarkdownEditor {
            note,
            mode: initial_mode,
            dirty: false,
            pending_since: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            render_cache: RenderCache::default(),
            canvas,
            canvas_interaction: InteractionState::new(),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Ensure the canvas document exists, generating or parsing it from the document's body if not yet initialized.
    pub fn ensure_canvas(&mut self) -> &mut CanvasDocument {
        if self.canvas.is_none() {
            let canvas = CanvasDocument::from_markdown_body(
                &self.note.frontmatter.title,
                &self.note.body,
            );
            self.canvas = Some(canvas);
        }
        self.canvas.as_mut().unwrap()
    }

    /// Mark canvas as dirty and schedule debounced sync/autosave.
    pub fn mark_dirty_canvas(&mut self) {
        if let Some(canvas) = &self.canvas {
            let new_body = canvas.to_markdown_body();
            if new_body != self.note.body {
                self.note.body = new_body;
                self.dirty = true;
                self.pending_since = Some(Instant::now());
                self.render_cache.invalidate();
            }
        }
    }

    /// Synchronize canvas state back to note body.
    pub fn sync_canvas_to_body(&mut self) {
        self.mark_dirty_canvas();
    }

    /// Replace the note body. Records an undo snapshot of the previous
    /// value and (re)starts the autosave debounce window. A no-op if the
    /// body is unchanged, so re-rendering the same text every frame
    /// doesn't spam the undo stack.
    pub fn set_body(&mut self, new_body: String) {
        if new_body == self.note.body {
            return;
        }
        self.undo_stack.push(self.note.body.clone());
        if self.undo_stack.len() > MAX_UNDO_HISTORY {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
        self.note.body = new_body;
        self.dirty = true;
        self.pending_since = Some(Instant::now());
        self.render_cache.invalidate();
    }

    /// Step back to the previous snapshot. Returns `false` if there's
    /// nothing to undo.
    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo_stack.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.note.body, prev);
        self.redo_stack.push(current);
        self.dirty = true;
        self.pending_since = Some(Instant::now());
        self.render_cache.invalidate();
        true
    }

    /// Re-apply a snapshot previously undone. Returns `false` if there's
    /// nothing to redo.
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.note.body, next);
        self.undo_stack.push(current);
        self.dirty = true;
        self.pending_since = Some(Instant::now());
        self.render_cache.invalidate();
        true
    }

    /// True once the debounce window has elapsed since the last edit and
    /// there are unsaved changes.
    pub fn should_autosave(&self) -> bool {
        self.dirty
            && self
                .pending_since
                .is_some_and(|t| t.elapsed() >= AUTOSAVE_DEBOUNCE)
    }

    /// Persist the note to disk now, regardless of the debounce window —
    /// used both by the per-frame autosave poll and when the user
    /// navigates away from the note.
    pub fn autosave(&mut self) -> Result<()> {
        if self.mode == EditorMode::Edgeless {
            if let Some(canvas) = &self.canvas {
                self.note.body = canvas.to_markdown_body();
            }
        }
        self.note.save()?;
        self.dirty = false;
        self.pending_since = None;
        Ok(())
    }

    pub fn word_count(&self) -> usize {
        self.note.body.split_whitespace().count()
    }

    /// Estimated reading time in minutes at `WORDS_PER_MINUTE`, rounded up
    /// to at least 1 minute for any non-empty note.
    pub fn reading_time_minutes(&self) -> usize {
        let words = self.word_count();
        if words == 0 {
            0
        } else {
            words.div_ceil(WORDS_PER_MINUTE).max(1)
        }
    }

    /// Heading outline for the current body (Outline side panel),
    /// recomputed only when the body changed since the last call — used by
    /// `app.rs` every frame without re-walking/re-slugging an unchanged
    /// document on frames where nothing edited it (§Fase 10).
    pub fn outline(&mut self) -> Vec<Heading> {
        self.render_cache.outline(&self.note.body).to_vec()
    }

    /// Renders the body into `ui` (Live Preview/Reading modes), memoized
    /// and virtualized against `viewport` (content-space, from
    /// `egui::ScrollArea::show_viewport`) via this editor's own
    /// `RenderCache` — see `renderer::render_cached` and the module doc
    /// comment on `markdown::renderer` (§Fase 10, §6 risk 5).
    pub fn render(&mut self, ui: &mut egui::Ui, cache: &mut CommonMarkCache, viewport: egui::Rect) -> RenderOutcome {
        renderer::render_cached(ui, cache, &mut self.render_cache, &self.note.body, viewport)
    }
}

/// A `/` slash-command template offered by the insertion popup (§3.2.4).
pub struct SlashTemplate {
    pub label: &'static str,
    pub insert: &'static str,
}

pub fn slash_templates() -> &'static [SlashTemplate] {
    &[
        SlashTemplate { label: "Heading 1", insert: "# " },
        SlashTemplate { label: "Heading 2", insert: "## " },
        SlashTemplate { label: "Checklist", insert: "- [ ] " },
        SlashTemplate { label: "Code block", insert: "```rust\n\n```" },
        SlashTemplate { label: "Callout Note", insert: "> [!note]\n> " },
        SlashTemplate { label: "Callout Warning", insert: "> [!warning]\n> " },
        SlashTemplate { label: "Table", insert: "| Kolom 1 | Kolom 2 |\n| --- | --- |\n|  |  |" },
        SlashTemplate { label: "Divider", insert: "---\n" },
        SlashTemplate { label: "Canvas Embed", insert: "![[canvas:whiteboard]]\n" },
    ]
}

/// True when the cursor sits right after a lone `/` at the start of its
/// line — the slash-command trigger (§3.2.4).
pub fn slash_menu_triggered(text_before_cursor: &str) -> bool {
    current_line(text_before_cursor) == "/"
}

/// Detects an in-progress `[[partial title` wikilink under the cursor
/// (§3.2.2 autocomplete). Returns the partial query typed so far, or
/// `None` if the cursor isn't inside an unfinished wikilink.
pub fn wikilink_autocomplete_query(text_before_cursor: &str) -> Option<String> {
    let line = current_line(text_before_cursor);
    let start = line.rfind("[[")?;
    let after = &line[start + 2..];
    if after.contains("]]") || after.contains("[[") {
        return None;
    }
    Some(after.to_string())
}

fn current_line(text_before_cursor: &str) -> &str {
    text_before_cursor.rsplit('\n').next().unwrap_or("")
}

/// Converts an `egui` char-based cursor index into a byte offset into
/// `text` — `egui`'s cursor counts Unicode scalar values, but Rust string
/// slicing needs byte offsets.
pub fn char_index_to_byte_offset(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn editor_with_body(body: &str) -> (tempfile::TempDir, MarkdownEditor) {
        let dir = tempdir().unwrap();
        let note = Note::create(dir.path(), "Judul", body).unwrap();
        (dir, MarkdownEditor::open(note))
    }

    #[test]
    fn set_body_is_noop_when_unchanged() {
        let (_dir, mut editor) = editor_with_body("isi");
        editor.set_body("isi".to_string());
        assert!(!editor.is_dirty());
    }

    #[test]
    fn set_body_marks_dirty_and_enables_autosave_after_debounce() {
        let (_dir, mut editor) = editor_with_body("awal");
        editor.set_body("ubah".to_string());
        assert!(editor.is_dirty());
        assert!(!editor.should_autosave());
    }

    #[test]
    fn edgeless_mode_and_canvas_initialization() {
        let (_dir, mut editor) = editor_with_body("# Title\n\n- [ ] Task 1");
        editor.mode = EditorMode::Edgeless;
        let canvas = editor.ensure_canvas();
        assert_eq!(canvas.elements.len(), 2);
    }
}
