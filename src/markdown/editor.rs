//! Editor session state for a single open note (§3.2): mode switching
//! (Source / Live Preview / Reading), debounced autosave, coarse undo/redo,
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

use crate::notes::Note;

use super::renderer::{self, Heading, RenderCache, RenderOutcome};

/// Idle window before an edit is flushed to disk (§3.2.4: "debounce
/// 500ms-1s").
pub const AUTOSAVE_DEBOUNCE: Duration = Duration::from_millis(800);
const WORDS_PER_MINUTE: usize = 200;
/// Caps memory use of the undo stack; old snapshots are dropped, not the
/// ability to undo recent edits.
const MAX_UNDO_HISTORY: usize = 100;

/// How the note body is currently presented. `LivePreview` and `Reading`
/// share the same interactive renderer (`markdown::renderer::render`) —
/// true inline WYSIWYG editing isn't practical in immediate-mode `egui`,
/// so raw text editing always happens in `Source` mode; `LivePreview` is
/// the interactive rendered view used as the default, `Reading` is the
/// same view intended for distraction-free reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    Source,
    LivePreview,
    Reading,
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
}

impl MarkdownEditor {
    pub fn open(note: Note) -> MarkdownEditor {
        MarkdownEditor {
            note,
            mode: EditorMode::LivePreview,
            dirty: false,
            pending_since: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            render_cache: RenderCache::default(),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
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
        SlashTemplate { label: "Heading", insert: "# " },
        SlashTemplate { label: "Checklist", insert: "- [ ] " },
        SlashTemplate { label: "Code block", insert: "```\n\n```" },
        SlashTemplate { label: "Callout", insert: "> [!note]\n> " },
        SlashTemplate { label: "Table", insert: "| Kolom 1 | Kolom 2 |\n| --- | --- |\n|  |  |" },
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

    // Keeps the backing `TempDir` alive for as long as the editor: dropping
    // it early would delete the note file out from under `autosave()`.
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
        let (_dir, mut editor) = editor_with_body("isi lama");
        editor.set_body("isi baru".to_string());
        assert!(editor.is_dirty());
        assert!(!editor.should_autosave()); // debounce window not elapsed yet
    }

    #[test]
    fn autosave_persists_and_clears_dirty_flag() {
        let (_dir, mut editor) = editor_with_body("isi lama");
        editor.set_body("isi baru".to_string());
        editor.autosave().unwrap();

        assert!(!editor.is_dirty());
        let reloaded = Note::load(&editor.note.path).unwrap();
        assert_eq!(reloaded.body, "isi baru");
    }

    #[test]
    fn undo_then_redo_round_trips() {
        let (_dir, mut editor) = editor_with_body("versi 1");
        editor.set_body("versi 2".to_string());
        editor.set_body("versi 3".to_string());

        assert!(editor.undo());
        assert_eq!(editor.note.body, "versi 2");
        assert!(editor.undo());
        assert_eq!(editor.note.body, "versi 1");
        assert!(!editor.undo());

        assert!(editor.redo());
        assert_eq!(editor.note.body, "versi 2");
    }

    #[test]
    fn editing_after_undo_clears_redo_stack() {
        let (_dir, mut editor) = editor_with_body("versi 1");
        editor.set_body("versi 2".to_string());
        editor.undo();
        editor.set_body("cabang baru".to_string());

        assert!(!editor.redo());
    }

    #[test]
    fn word_count_and_reading_time() {
        let (_dir, editor) = editor_with_body("satu dua tiga");
        assert_eq!(editor.word_count(), 3);
        assert_eq!(editor.reading_time_minutes(), 1);

        let (_dir2, empty) = editor_with_body("");
        assert_eq!(empty.reading_time_minutes(), 0);
    }

    #[test]
    fn slash_menu_triggered_only_on_lone_slash_at_line_start() {
        assert!(slash_menu_triggered("/"));
        assert!(slash_menu_triggered("paragraf pertama\n/"));
        assert!(!slash_menu_triggered("teks/lain"));
        assert!(!slash_menu_triggered(""));
    }

    #[test]
    fn wikilink_autocomplete_query_extracts_partial_title() {
        assert_eq!(
            wikilink_autocomplete_query("Lihat [[Bel"),
            Some("Bel".to_string())
        );
        assert_eq!(wikilink_autocomplete_query("[["), Some(String::new()));
    }

    #[test]
    fn wikilink_autocomplete_query_none_once_link_closed() {
        assert_eq!(wikilink_autocomplete_query("[[Selesai]] lanjut"), None);
    }

    #[test]
    fn wikilink_autocomplete_query_uses_most_recent_bracket_pair() {
        assert_eq!(
            wikilink_autocomplete_query("[[Lama]] dan [[Baru"),
            Some("Baru".to_string())
        );
    }

    #[test]
    fn char_index_to_byte_offset_handles_multibyte_chars() {
        let text = "héllo"; // 'é' is 2 bytes
        assert_eq!(char_index_to_byte_offset(text, 0), 0);
        assert_eq!(char_index_to_byte_offset(text, 1), 1);
        assert_eq!(char_index_to_byte_offset(text, 2), 3); // after 'é'
        assert_eq!(char_index_to_byte_offset(text, 5), text.len());
    }
}
