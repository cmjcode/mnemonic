//! Editor session state for a single open note (§3.2): mode switching
//! (Live / Source / Edgeless canvas / Split), debounced autosave, coarse
//! undo/redo, and word count / reading time. Slash-command and
//! wikilink-autocomplete trigger detection are pure string functions here
//! too, so the popup logic in `app` stays thin. Also owns a
//! `renderer::RenderCache` (§Fase 10) — invalidated in `set_body`/`undo`/
//! `redo` and read via `outline()`/`render()`, so the Live view's
//! memoization lives right next to the only code that mutates the body
//! it's keyed on. Callers: `app`.

use std::time::{Duration, Instant};

use anyhow::Result;
use egui_commonmark::CommonMarkCache;

use std::path::{Path, PathBuf};

use std::collections::HashSet;

use crate::canvas::{BindingScope, BlockBinding, CanvasDocument, CanvasElementId, InteractionState};
use crate::notes::Note;

use super::{blocks, sections};

mod canvas_sync;

use super::live_blocks::LiveBlock;
use super::renderer::{self, Heading, LiveParams, RenderCache, RenderOutcome};

/// Idle window before an edit is flushed to disk (§3.2.4: "debounce
/// 500ms-1s").
pub const AUTOSAVE_DEBOUNCE: Duration = Duration::from_millis(800);
const WORDS_PER_MINUTE: usize = 200;
/// Caps memory use of the undo stack; old snapshots are dropped, not the
/// ability to undo recent edits.
const MAX_UNDO_HISTORY: usize = 100;

/// How the note body is currently presented.
/// - `Live`: the default (§3.2.1): rendered, and the clicked line turns
///   into raw Markdown in place (`markdown::renderer`, `app::editor::live`).
/// - `Source`: the whole body as raw Markdown (power users, palette only).
/// - `Edgeless`: infinite 2D spatial canvas / whiteboard (AFFiNE-style).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    Live,
    Source,
    Edgeless,
    /// Markdown on the left, the bound diagram on the right (§Fase 3).
    Split,
}

impl EditorMode {
    /// Modes that draw the canvas.
    pub fn shows_canvas(self) -> bool {
        matches!(self, EditorMode::Edgeless | EditorMode::Split)
    }
}

/// How a canvas note is serialized into `note.body` on save.
///
/// Decided once when the note is opened (from the on-disk body, file
/// extension and tags) — never re-sniffed from `note.body`, which may hold a
/// readable Markdown projection of the canvas while the note is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasStorage {
    /// No diagram layer yet: a plain note. Opening it as a canvas binds
    /// its blocks and switches to `Sidecar`. (Legacy canvas notes whose
    /// body was a lossy Markdown projection also start here.)
    Markdown,
    /// Obsidian JSON Canvas in `<note>.canvas` next to the note; text of
    /// bound nodes lives in the Markdown blocks they point at (§Fase 3).
    Sidecar,
    /// Draw.io XML inside a ```` ```drawio ```` fence of a `.md` note.
    DrawioFence,
    /// Bare Draw.io XML (`.drawio` file, or a body that starts with `<?xml`/`<mxfile`).
    DrawioRaw,
}

impl CanvasStorage {
    pub fn is_drawio(self) -> bool {
        matches!(self, CanvasStorage::DrawioFence | CanvasStorage::DrawioRaw)
    }

    /// Storage format implied by a note as it sits on disk.
    fn detect(note: &Note) -> Self {
        let body = note.body.trim_start();
        let is_drawio_file = note.path.extension().is_some_and(|ext| ext == "drawio");
        let has_drawio_tag = note
            .frontmatter
            .tags
            .iter()
            .any(|t| t.eq_ignore_ascii_case("drawio"));
        if is_drawio_file || body.starts_with("<?xml") || body.starts_with("<mxfile") {
            CanvasStorage::DrawioRaw
        } else if body.contains("```drawio") || has_drawio_tag {
            CanvasStorage::DrawioFence
        } else if note.has_sidecar {
            CanvasStorage::Sidecar
        } else {
            CanvasStorage::Markdown
        }
    }

    pub fn is_sidecar(self) -> bool {
        self == CanvasStorage::Sidecar
    }
}

/// Why a save didn't happen.
#[derive(Debug)]
pub enum SaveError {
    /// The file changed on disk since it was opened/last saved (an
    /// external editor, a sync client, an AI agent) — saving would
    /// silently overwrite that edit. The caller decides (§6 "Watcher
    /// Conflict": reload / overwrite / save a copy).
    Conflict,
    Io(anyhow::Error),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Conflict => write!(f, "file changed on disk since it was opened"),
            SaveError::Io(e) => write!(f, "{e:#}"),
        }
    }
}

/// An open editing session for one note.
pub struct MarkdownEditor {
    pub note: Note,
    pub mode: EditorMode,
    /// Serialization format used by `mark_dirty_canvas`.
    canvas_storage: CanvasStorage,
    /// Vault root, for vault-relative paths in the sidecar and for
    /// resolving bindings into other notes.
    vault_root: Option<PathBuf>,
    /// The sidecar must be (re)written on the next save.
    sidecar_dirty: bool,
    /// Hash of the file's bytes when it was opened / last written by us.
    /// `None` for a note that has no file yet or couldn't be read.
    disk_fingerprint: Option<u64>,
    dirty: bool,
    pending_since: Option<Instant>,
    undo_stack: Vec<String>,
    redo_stack: Vec<String>,
    /// Memoized Live-view parse of `note.body` (§Fase 10),
    /// invalidated on every body mutation below so it never goes stale.
    render_cache: RenderCache,
    /// Edgeless infinite canvas state when in `EditorMode::Edgeless`.
    pub canvas: Option<CanvasDocument>,
    /// Interaction state for canvas manipulation.
    pub canvas_interaction: InteractionState,
    /// Whether the user is currently editing the document's title (double-click rename).
    pub is_editing_title: bool,
    /// Working buffer during inline title editing.
    pub title_edit_buffer: String,
    /// Section boxes whose segment no longer exists (§3.9.2).
    orphans: HashSet<CanvasElementId>,
}

impl MarkdownEditor {
    pub fn open(note: Note) -> MarkdownEditor {
        Self::open_in(note, None)
    }

    /// Opens `note` from the vault at `vault_root` (needed to write
    /// vault-relative paths into a `.canvas` sidecar and to resolve
    /// bindings into other notes).
    pub fn open_in(mut note: Note, vault_root: Option<&Path>) -> MarkdownEditor {
        let vault_root = vault_root.map(Path::to_path_buf);
        let is_canvas = note.is_canvas();
        let canvas_storage = CanvasStorage::detect(&note);
        let initial_mode = if note.has_note_content() {
            EditorMode::Live
        } else if is_canvas {
            EditorMode::Edgeless
        } else {
            EditorMode::Live
        };

        let canvas = match canvas_storage {
            CanvasStorage::Sidecar => load_sidecar(&note, vault_root.as_deref()),
            _ if is_canvas
                || note.body.contains("```canvas")
                || note.body.contains("```drawio")
                || note.body.starts_with("<?xml") =>
            {
                let doc = CanvasDocument::from_markdown_body(&note.frontmatter.title, &note.body);
                // Show a readable projection in the Live/Source view instead of raw
                // JSON/XML. The diagram itself lives in `canvas`; `autosave` always
                // re-serializes from it for Draw.io storage, so this never reaches disk.
                if note.body.contains("```canvas")
                    || note.body.contains("```drawio")
                    || note.body.starts_with("<?xml")
                {
                    note.body = doc.to_markdown_body();
                }
                Some(doc)
            }
            _ => None,
        };

        let title = note.frontmatter.title.clone();

        let mut canvas_interaction = InteractionState::new();
        // Diagram coordinates can sit anywhere; frame the diagram on first show.
        canvas_interaction.pending_fit = canvas_storage.is_drawio() || canvas_storage.is_sidecar();

        let disk_fingerprint = Note::disk_fingerprint(&note.path);
        MarkdownEditor {
            note,
            mode: initial_mode,
            canvas_storage,
            vault_root,
            sidecar_dirty: false,
            disk_fingerprint,
            dirty: false,
            pending_since: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            render_cache: RenderCache::default(),
            canvas,
            canvas_interaction,
            is_editing_title: false,
            title_edit_buffer: title,
            orphans: HashSet::new(),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Marks frontmatter-only edits (tags, aliases, properties) for the
    /// next autosave.
    pub fn mark_metadata_dirty(&mut self) {
        self.dirty = true;
        self.pending_since = Some(Instant::now());
    }

    /// Restarts the debounce window without saving — used after a failed
    /// save so the per-frame poll retries later instead of every frame.
    pub fn postpone_autosave(&mut self) {
        if self.dirty {
            self.pending_since = Some(Instant::now());
        }
    }

    /// Update the note's title and mark as dirty for autosave.
    pub fn set_title(&mut self, new_title: String) {
        if new_title == self.note.frontmatter.title {
            return;
        }
        self.note.frontmatter.title = new_title.clone();
        self.title_edit_buffer = new_title.clone();
        self.dirty = true;
        self.pending_since = Some(Instant::now());
        if let Some(canvas) = &mut self.canvas {
            canvas.title = new_title;
        }
    }

    /// Ensure the canvas document exists. A plain note (no diagram layer
    /// yet) becomes a section canvas: every section / table / fence gets an
    /// anchor and one box, laid out as a mind map, stored as a `.canvas`
    /// sidecar from the next save (§3.9.2).
    pub fn ensure_canvas(&mut self) -> &mut CanvasDocument {
        if self.canvas.is_none() {
            let canvas = match self.canvas_storage {
                CanvasStorage::Sidecar => load_sidecar(&self.note, self.vault_root.as_deref())
                    .unwrap_or_else(|| CanvasDocument::new(&self.note.frontmatter.title)),
                CanvasStorage::Markdown => {
                    let doc = self.build_section_canvas();
                    self.canvas_storage = CanvasStorage::Sidecar;
                    self.sidecar_dirty = true;
                    self.dirty = true;
                    self.pending_since = Some(Instant::now());
                    self.canvas_interaction.pending_fit = true;
                    doc
                }
                _ => CanvasDocument::from_markdown_body(
                    &self.note.frontmatter.title,
                    &self.note.body,
                ),
            };
            self.canvas = Some(canvas);
            self.upgrade_block_canvas();
        }
        self.canvas.as_mut().unwrap()
    }

    /// Vault-relative path of the note (the `file` of bound nodes).
    fn owner_rel_path(&self) -> Option<String> {
        let rel = match &self.vault_root {
            Some(root) => self.note.path.strip_prefix(root).ok().map(Path::to_path_buf),
            None => None,
        }
        .or_else(|| self.note.path.file_name().map(PathBuf::from))?;
        Some(rel.to_string_lossy().replace('\\', "/"))
    }

    /// Replaces the diagram with `doc` (a bound Draw.io import), appending
    /// one anchored paragraph per newly bound node so their text lives in
    /// the Markdown, and switches the note to sidecar storage.
    pub fn import_bound_canvas(&mut self, doc: CanvasDocument, new_blocks: Vec<(BlockBinding, String)>) {
        let mut body = self.note.body.clone();
        for (binding, text) in &new_blocks {
            body = blocks::append_block(&body, text, &binding.block_id);
        }
        self.set_body(body);
        let mut doc = doc;
        doc.title = self.note.frontmatter.title.clone();
        self.canvas = Some(doc);
        self.adopt_sidecar_storage();
    }

    /// Makes the sidecar the storage format (from a plain note or a
    /// Draw.io-fenced note) so the next save writes `<note>.canvas`.
    pub fn adopt_sidecar_storage(&mut self) {
        if self.canvas_storage == CanvasStorage::DrawioRaw {
            return; // a bare `.drawio` file stays a `.drawio` file
        }
        if self.canvas_storage == CanvasStorage::DrawioFence {
            // The XML fence leaves the body; the diagram now lives in the sidecar.
            let projected = self
                .canvas
                .as_ref()
                .map(|c| c.to_markdown_body())
                .unwrap_or_default();
            self.set_body(projected);
        }
        self.canvas_storage = CanvasStorage::Sidecar;
        self.sidecar_dirty = true;
        self.dirty = true;
        self.pending_since = Some(Instant::now());
        self.note.frontmatter.note_type = crate::notes::NoteType::Canvas;
    }

    /// Binds canvas element `id` to a new anchored block appended to the
    /// Markdown (so its text becomes part of the note). No-op if already
    /// bound or the element has no text.
    pub fn bind_element_to_note(&mut self, id: crate::canvas::CanvasElementId) -> bool {
        let existing: Vec<String> = blocks::block_anchors(&self.note.body)
            .into_iter()
            .map(|b| b.id)
            .collect();
        let Some(canvas) = self.canvas.as_mut() else {
            return false;
        };
        let Some(elem) = canvas.get_element_mut(id) else {
            return false;
        };
        if elem.is_bound() {
            return false;
        }
        let Some(text) = elem.text().map(str::to_string) else {
            return false;
        };
        let block_id = blocks::generate_id(&existing);
        elem.set_binding(Some(BlockBinding::local(block_id.clone())));
        let body = blocks::append_block(&self.note.body, &text, &block_id);
        self.set_body(body);
        self.mark_dirty_canvas();
        true
    }

    /// Detaches element `id` from its block: the node keeps its text as a
    /// diagram-only node; the Markdown block stays as it is.
    pub fn unbind_element(&mut self, id: crate::canvas::CanvasElementId) -> bool {
        let Some(canvas) = self.canvas.as_mut() else {
            return false;
        };
        let Some(elem) = canvas.get_element_mut(id) else {
            return false;
        };
        if !elem.is_bound() {
            return false;
        }
        elem.set_binding(None);
        self.mark_dirty_canvas();
        true
    }

    /// Text of the block a binding points at (this note, or another note
    /// in the vault).
    fn resolve_binding(&self, binding: &BlockBinding) -> Option<String> {
        resolve_binding_text(&self.note, self.vault_root.as_deref(), binding)
    }

    /// After a Markdown edit: re-derive the text of bound nodes (except the
    /// one being typed in, whose buffer is ahead of the body) and, for a
    /// section canvas, its boxes and outline edges.
    fn refresh_canvas_from_body(&mut self) {
        if !self.canvas_storage.is_sidecar() {
            return;
        }
        let Some(mut canvas) = self.canvas.take() else {
            return;
        };
        let editing = self.canvas_interaction.editing_text_elem;
        let mut changed = canvas.refresh_bound_text_except(&|b| self.resolve_binding(b), editing);
        changed |= self.reconcile_segments(&mut canvas);
        if changed {
            self.sidecar_dirty = true;
        }
        self.canvas = Some(canvas);
    }

    pub fn canvas_storage(&self) -> CanvasStorage {
        self.canvas_storage
    }

    /// Switch a Markdown-stored canvas to Draw.io storage so element
    /// positions survive a save — used when a Draw.io diagram is imported
    /// into a plain canvas note. Adds the `drawio` tag (as
    /// `Note::create_drawio` does) so the format also sticks when the note
    /// is reopened.
    pub fn adopt_drawio_storage(&mut self) {
        if self.canvas_storage.is_drawio() {
            return;
        }
        self.canvas_storage = CanvasStorage::DrawioFence;
        let tags = &mut self.note.frontmatter.tags;
        if !tags.iter().any(|t| t.eq_ignore_ascii_case("drawio")) {
            tags.push("drawio".to_string());
        }
        self.dirty = true;
        self.pending_since = Some(Instant::now());
    }

    /// Mark canvas as dirty and schedule debounced sync/autosave. For a
    /// sidecar note the Markdown is the source of truth for text: an edited
    /// bound node writes back through `write_back_element` (only that
    /// node, so a stale copy can never overwrite newer Markdown); geometry
    /// goes to the sidecar on save.
    pub fn mark_dirty_canvas(&mut self) {
        let Some(canvas) = &self.canvas else {
            return;
        };
        if self.canvas_storage.is_sidecar() {
            self.sidecar_dirty = true;
            self.dirty = true;
            self.pending_since = Some(Instant::now());
            return;
        }
        let new_body = match self.canvas_storage {
            CanvasStorage::Markdown => canvas.to_markdown_body(),
            CanvasStorage::DrawioRaw => canvas.to_drawio_xml(),
            CanvasStorage::DrawioFence => {
                format!("```drawio\n{}\n```\n", canvas.to_drawio_xml().trim())
            }
            CanvasStorage::Sidecar => unreachable!(),
        };

        if new_body != self.note.body {
            self.note.body = new_body;
            self.dirty = true;
            self.pending_since = Some(Instant::now());
            self.render_cache.invalidate();
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
        self.set_body_inner(new_body);
        self.refresh_canvas_from_body();
    }

    fn set_body_inner(&mut self, new_body: String) {
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
        self.refresh_canvas_from_body();
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
        self.refresh_canvas_from_body();
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

    /// `true` when the file on disk no longer matches what this editor
    /// last read or wrote — somebody else edited it meanwhile.
    pub fn has_external_change(&self) -> bool {
        match (self.disk_fingerprint, Note::disk_fingerprint(&self.note.path)) {
            (Some(known), Some(now)) => known != now,
            // Deleted underneath us, or never readable: not a conflict we
            // can resolve by reloading.
            _ => false,
        }
    }

    /// Persist the note to disk now, regardless of the debounce window —
    /// used both by the per-frame autosave poll and when the user
    /// navigates away from the note. Refuses with `SaveError::Conflict`
    /// when the file changed on disk meanwhile; see `force_save`.
    pub fn autosave(&mut self) -> Result<(), SaveError> {
        if self.has_external_change() {
            return Err(SaveError::Conflict);
        }
        self.force_save().map_err(SaveError::Io)
    }

    /// Saves without the external-change check (the user chose to
    /// overwrite, or the note was just reloaded).
    pub fn force_save(&mut self) -> Result<()> {
        // A Draw.io note's body is a readable projection while open (see
        // `open`); the diagram in `canvas` is the source of truth, so it must
        // be re-serialized whatever mode the editor is in (e.g. a title rename
        // from the Live view), or the XML on disk would be replaced by text.
        if self.mode.shows_canvas() || (self.canvas_storage.is_drawio() && self.canvas.is_some()) {
            self.sync_canvas_to_body();
        }
        // New sections typed in the Markdown get their anchor (and box)
        // now, at idle time, never mid-keystroke (§3.9.2).
        self.anchor_new_segments();
        // `Note::save` stamps `modified` on the copy it writes; mirror that
        // here so sorting by "last modified" is right without a rescan.
        self.note.frontmatter.modified = chrono::Utc::now();
        self.note.save()?;
        // Obsidian convention: the file is named after the note. Done after
        // the write so a rename failure never loses content.
        self.note.sync_file_name_with_title()?;
        if self.canvas_storage.is_sidecar()
            && (self.sidecar_dirty || !self.note.has_sidecar)
            && let Some(canvas) = self.canvas.as_ref()
        {
            let owner = self.owner_rel_path();
            let canvas = canvas.clone();
            self.note.save_sidecar(&canvas, owner.as_deref())?;
            self.sidecar_dirty = false;
        }
        self.disk_fingerprint = Note::disk_fingerprint(&self.note.path);
        self.dirty = false;
        self.pending_since = None;
        Ok(())
    }

    /// Discards unsaved edits and re-reads the note from disk, keeping the
    /// editor mode and undo history (the previous body is pushed onto it
    /// so a reload is itself undoable).
    pub fn reload_from_disk(&mut self) -> Result<()> {
        let fresh = Note::load(&self.note.path)?;
        let previous = std::mem::take(&mut self.note.body);
        if previous != fresh.body {
            self.undo_stack.push(previous);
            if self.undo_stack.len() > MAX_UNDO_HISTORY {
                self.undo_stack.remove(0);
            }
            self.redo_stack.clear();
        }
        self.note = fresh;
        self.canvas_storage = CanvasStorage::detect(&self.note);
        self.title_edit_buffer = self.note.frontmatter.title.clone();
        self.canvas = None;
        self.sidecar_dirty = false;
        if self.note.is_canvas() || self.mode.shows_canvas() {
            self.ensure_canvas();
            if self.canvas_storage.is_drawio() {
                self.note.body = self.canvas.as_ref().map(|c| c.to_markdown_body()).unwrap_or_default();
            }
        }
        self.render_cache.invalidate();
        self.disk_fingerprint = Note::disk_fingerprint(&self.note.path);
        self.dirty = false;
        self.pending_since = None;
        Ok(())
    }

    /// Writes the current (conflicting) content as a new sibling note
    /// `<title> (conflict).md`, leaving the on-disk original alone, and
    /// returns the copy. This editor is then reloaded from disk.
    pub fn save_conflict_copy(&mut self) -> Result<Note> {
        if self.mode.shows_canvas() || (self.canvas_storage.is_drawio() && self.canvas.is_some()) {
            self.sync_canvas_to_body();
        }
        let dir = self
            .note
            .path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("note has no parent folder"))?;
        let title = format!("{} (conflict)", self.note.frontmatter.title);
        let mut copy = Note::create(dir, &title, &self.note.body)?;
        copy.frontmatter.tags = self.note.frontmatter.tags.clone();
        copy.frontmatter.note_type = self.note.frontmatter.note_type;
        copy.save()?;
        self.reload_from_disk()?;
        Ok(copy)
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

    /// Scrolls the Live view to the block anchored `^id` the next time it
    /// is drawn.
    pub fn scroll_to_block(&mut self, id: &str) {
        self.render_cache.scroll_to_block(id);
    }

    /// Scrolls the Live view to the heading with slug `slug` the next time
    /// it is drawn.
    pub fn scroll_to_heading(&mut self, slug: &str) {
        self.render_cache.scroll_to_heading(slug);
    }

    /// The Live blocks of the current body (memoized).
    pub fn blocks(&mut self) -> Vec<LiveBlock> {
        self.render_cache.blocks(&self.note.body).to_vec()
    }

    /// Renders the body in Live mode into `ui`, memoized and virtualized
    /// via this editor's own `RenderCache`; `draw_editor` draws the raw
    /// editor for `params.active` — see `renderer::render_cached`
    /// (§3.2.1, §Fase 10).
    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        cache: &mut CommonMarkCache,
        params: &LiveParams<'_>,
        draw_editor: &mut dyn FnMut(&mut egui::Ui),
    ) -> RenderOutcome {
        renderer::render_cached(ui, cache, &mut self.render_cache, &self.note.body, params, draw_editor)
    }
}

/// A `/` slash-command template offered by the insertion popup (§3.2.4).
pub struct SlashTemplate {
    /// Locale key for the menu label (see `locales/*/main.ftl`).
    pub key: &'static str,
    pub insert: &'static str,
}

pub fn slash_templates() -> &'static [SlashTemplate] {
    &[
        SlashTemplate { key: "slash-heading-1", insert: "# " },
        SlashTemplate { key: "slash-heading-2", insert: "## " },
        SlashTemplate { key: "slash-checklist", insert: "- [ ] " },
        SlashTemplate { key: "slash-bullet-list", insert: "- " },
        SlashTemplate { key: "slash-quote", insert: "> " },
        SlashTemplate { key: "slash-code-block", insert: "```\n\n```" },
        SlashTemplate { key: "slash-callout-note", insert: "> [!note]\n> " },
        SlashTemplate { key: "slash-callout-warning", insert: "> [!warning]\n> " },
        SlashTemplate { key: "slash-table", insert: "| A | B |\n| --- | --- |\n|  |  |" },
        SlashTemplate { key: "slash-divider", insert: "---\n" },
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

/// Reads the sidecar next to `note`, resolving bound text from the
/// Markdown. `None` when the file is missing or unreadable.
fn load_sidecar(note: &Note, vault_root: Option<&Path>) -> Option<CanvasDocument> {
    let json = std::fs::read_to_string(note.sidecar_path()).ok()?;
    let owner = vault_root
        .and_then(|r| note.path.strip_prefix(r).ok())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .or_else(|| note.path.file_name().map(|n| n.to_string_lossy().to_string()));
    match CanvasDocument::from_json_canvas_str(&json, &note.frontmatter.title, owner.as_deref(), &|b| {
        resolve_binding_text(note, vault_root, b)
    }) {
        Ok(doc) => Some(doc),
        Err(e) => {
            log::warn!("editor: unreadable sidecar {}: {e:#}", note.sidecar_path().display());
            None
        }
    }
}

/// Text of the block `binding` points at: in `note` itself, or in the
/// vault note named by `binding.file`.
fn resolve_binding_text(note: &Note, vault_root: Option<&Path>, binding: &BlockBinding) -> Option<String> {
    let text_in = |body: &str| match binding.scope {
        BindingScope::Block => blocks::block_text(body, &binding.block_id),
        BindingScope::Segment => sections::segment_text(body, &binding.block_id),
    };
    match &binding.file {
        None => text_in(&note.body),
        Some(file) => {
            let path = vault_root.map(|r| r.join(file))?;
            if path == note.path {
                return text_in(&note.body);
            }
            let other = Note::load(&path).ok()?;
            text_in(&other.body)
        }
    }
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
        // One section box: the heading owns the task under it.
        let (_dir, mut editor) = editor_with_body("# Title\n\n- [ ] Task 1");
        editor.mode = EditorMode::Edgeless;
        let canvas = editor.ensure_canvas();
        assert_eq!(canvas.elements.len(), 1);
    }

    fn rect_of(elem: &crate::canvas::CanvasElement) -> egui::Rect {
        elem.bounding_rect()
    }

    #[test]
    fn bound_drawio_import_survives_save_and_reopen_with_text_in_markdown() {
        // A diagram-bound note (sidecar storage) receives a Draw.io import:
        // its `text` shapes become Markdown blocks, other shapes stay
        // diagram-only, and every position survives save + reopen.
        let dir = tempdir().unwrap();
        let note = Note::create_canvas(dir.path(), "Papan").unwrap();
        let mut editor = MarkdownEditor::open_in(note, Some(dir.path()));
        assert_eq!(editor.canvas_storage(), CanvasStorage::Sidecar);
        assert_eq!(editor.mode, EditorMode::Edgeless);

        let xml = r#"<mxfile><diagram name="P"><mxGraphModel><root><mxCell id="0"/><mxCell id="1" parent="0"/>
            <mxCell id="a" value="Alpha" style="rounded=1;html=1;" vertex="1" parent="1"><mxGeometry x="900" y="40" width="120" height="60" as="geometry"/></mxCell>
            <mxCell id="t" value="Penjelasan panjang" style="text;html=1;" vertex="1" parent="1"><mxGeometry x="900" y="200" width="200" height="40" as="geometry"/></mxCell>
            <mxCell id="b" value="Beta" style="ellipse;html=1;" vertex="1" parent="1"><mxGeometry x="1300" y="40" width="80" height="80" as="geometry"/></mxCell>
            <mxCell id="e" style="edgeStyle=orthogonalEdgeStyle;html=1;" edge="1" parent="1" source="a" target="b"><mxGeometry relative="1" as="geometry"/></mxCell>
            </root></mxGraphModel></diagram></mxfile>"#;
        let (imported, new_blocks) = crate::canvas::DrawioImporter::from_xml_bound("Papan", xml).unwrap();
        assert_eq!(new_blocks.len(), 1, "only the text shape is bound");
        let expected: Vec<egui::Rect> = imported.elements.iter().map(rect_of).collect();

        // What `show_editor` does after an import.
        editor.import_bound_canvas(imported, new_blocks);
        assert!(editor.note.body.contains("Penjelasan panjang ^"), "{}", editor.note.body);
        assert!(!editor.note.body.contains("Alpha"), "diagram-only shapes stay out of the Markdown");
        editor.autosave().unwrap();
        assert!(editor.note.sidecar_path().exists());

        let reopened = MarkdownEditor::open_in(Note::load(&editor.note.path).unwrap(), Some(dir.path()));
        assert_eq!(reopened.canvas_storage(), CanvasStorage::Sidecar);
        let canvas = reopened.canvas.as_ref().unwrap();
        let got: Vec<egui::Rect> = canvas.elements.iter().map(rect_of).collect();
        assert_eq!(got.len(), expected.len());
        for (g, e) in got.iter().zip(&expected) {
            assert!((g.min - e.min).length() < 0.6 && (g.max - e.max).length() < 0.6, "{g:?} != {e:?}");
        }
        let bound = canvas.elements.iter().find(|e| e.is_bound()).unwrap();
        assert_eq!(bound.text(), Some("Penjelasan panjang"));
    }

    #[test]
    fn editing_a_bound_node_writes_back_into_the_markdown_block() {
        let dir = tempdir().unwrap();
        let note = Note::create_canvas(dir.path(), "Sinkron").unwrap();
        let mut editor = MarkdownEditor::open_in(note, Some(dir.path()));
        let id = editor.canvas.as_ref().unwrap().elements[0].id();
        let block_id = editor.canvas.as_ref().unwrap().elements[0]
            .binding()
            .unwrap()
            .block_id
            .clone();

        // Canvas → Markdown.
        editor.canvas.as_mut().unwrap().get_element_mut(id).unwrap().set_text("Teks dari kanvas".into());
        editor.write_back_element(id, true);
        assert_eq!(
            crate::markdown::blocks::block_text(&editor.note.body, &block_id).as_deref(),
            Some("Teks dari kanvas")
        );

        // Markdown → canvas.
        let body = editor.note.body.replace("Teks dari kanvas", "Teks dari markdown");
        editor.set_body(body);
        assert_eq!(
            editor.canvas.as_ref().unwrap().elements[0].text(),
            Some("Teks dari markdown")
        );

        // A plain note opened as a diagram gets its sections bound.
        let plain = Note::create(dir.path(), "Polos", "Pembuka.\n\n# Judul\n\nSatu paragraf.\n\n- item\n").unwrap();
        let mut e2 = MarkdownEditor::open_in(plain, Some(dir.path()));
        assert_eq!(e2.canvas_storage(), CanvasStorage::Markdown);
        e2.ensure_canvas();
        assert_eq!(e2.canvas_storage(), CanvasStorage::Sidecar);
        assert_eq!(e2.canvas.as_ref().unwrap().elements.len(), 2);
        assert!(e2.canvas.as_ref().unwrap().elements.iter().all(|e| e.is_bound()));
        e2.autosave().unwrap();
        assert!(e2.note.sidecar_path().exists());
    }

    #[test]
    fn drawio_note_keeps_xml_when_saved_from_live_view() {
        let dir = tempdir().unwrap();
        let note = Note::create_drawio(dir.path(), "Diagram").unwrap();
        let before = CanvasDocument::from_markdown_body("Diagram", &note.body);
        let mut editor = MarkdownEditor::open(note);
        assert_eq!(editor.canvas_storage(), CanvasStorage::DrawioFence);

        editor.mode = EditorMode::Live;
        editor.set_title("Diagram Baru".to_string());
        editor.autosave().unwrap();

        let saved = Note::load(&editor.note.path).unwrap();
        assert!(saved.body.contains("```drawio"), "body on disk: {}", saved.body);
        let after = CanvasDocument::from_markdown_body("Diagram Baru", &saved.body);
        let rects = |d: &CanvasDocument| d.elements.iter().map(rect_of).collect::<Vec<_>>();
        assert_eq!(rects(&after), rects(&before));
    }

    #[test]
    fn set_title_updates_frontmatter_and_marks_dirty() {
        let (_dir, mut editor) = editor_with_body("content");
        assert_eq!(editor.note.frontmatter.title, "Judul");
        editor.set_title("New Doc".to_string());
        assert_eq!(editor.note.frontmatter.title, "New Doc");
        assert_eq!(editor.title_edit_buffer, "New Doc");
        assert!(editor.is_dirty());
    }
}
