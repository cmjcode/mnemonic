//! Top-level `eframe::App`: state, vault/index lifecycle, background
//! workers, file operations, keyboard shortcuts, and screen layout.
//!
//! Layout (every screen): top bar → left sidebar → optional AI panel →
//! central content (welcome, grid, editor, or PDF viewer) → modals,
//! command palette, toasts. Screens live in submodules:
//! `welcome`, `grid`, `editor`, `pdf`, `palette`. Callers: `main.rs`.

mod editor;
mod graph;
mod grid;
pub mod hotkeys;
mod palette;
mod pdf;
mod reading;
mod sheet;
mod sheet_grid;
mod welcome;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Key, KeyboardShortcut};
use egui_commonmark::CommonMarkCache;
use uuid::Uuid;

use crate::canvas::CanvasDocument;
use crate::core::ingestion::pdf_doc_id;
use crate::core::search::{self, HybridOptions, SearchHit};
use crate::core::{EMBEDDING_DIM, EMBEDDING_MODEL_ID, IndexStore, IndexingWorker};
use crate::i18n::LocaleManager;
use crate::llm::{self, GenerationEvent, GenerationWorker};
use crate::markdown::wikilink::{self, WikiLink, title_key};
use crate::markdown::editor::SaveError;
use crate::markdown::{EditorMode, MarkdownEditor};
use crate::notes::query::{self, SortMode};
use crate::notes::{Note, Vault, VaultWatcher, tags, trash};
use crate::settings::{self, AppSettings};
use crate::ui::{self, SidebarDocFilter, ToastKind, theme};

use editor::EditorUi;
use graph::GraphView;
use pdf::{PdfRendererState, PdfViewerState};
use sheet::SheetViewerState;

/// How many top-ranked chunks to retrieve for search / chat respectively
/// (§Fase 7). Search shows more candidates since a human skims them, while
/// chat feeds a token-bounded LLM prompt.
const SEARCH_TOP_K: usize = 10;
const CHAT_TOP_K: usize = 5;
/// Candidates fetched from each retrieval route (vector KNN, FTS5) before
/// Reciprocal Rank Fusion narrows them down to the top-K.
const RETRIEVAL_CANDIDATES: usize = 30;
/// How many related documents the editor panel lists.
const RELATED_LIMIT: usize = 6;
/// Nearest neighbors per document considered for graph AI edges.
const SEMANTIC_NEIGHBORS: usize = 3;

/// Pause after the last keystroke in the search box before running the
/// (comparatively expensive) semantic half of search.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(350);

/// Our own saves trigger the file watcher; events within this window after
/// a save are ignored so typing never causes a full vault rescan.
const SELF_WRITE_GRACE: Duration = Duration::from_millis(1500);

type Citation = ui::ChatCitationItem;
type ChatMessage = ui::ChatMessageItem;

#[derive(Debug, Clone)]
enum PromptKind {
    CreateFolder { parent_dir: PathBuf },
    RenameItem { path: PathBuf, is_dir: bool },
}

#[derive(Debug, Clone)]
struct PromptModalState {
    kind: PromptKind,
    title: String,
    message: String,
    value: String,
    placeholder: String,
    confirm_label: String,
}

#[derive(Debug, Clone)]
struct MoveModalState {
    src_path: PathBuf,
    item_name: String,
    is_dir: bool,
    search_filter: String,
}

/// What a toast's "Urungkan" (undo) button reverts.
#[derive(Debug, Clone)]
enum ToastAction {
    RestoreNote {
        trashed_path: PathBuf,
        original_path: PathBuf,
    },
    RestorePath {
        trashed_path: PathBuf,
        original_path: PathBuf,
        /// Registered PDFs (original paths) inside what was trashed.
        pdfs: Vec<PathBuf>,
    },
    ToggleArchived(Uuid),
    /// Restores note bodies rewritten when a renamed note's links were
    /// updated: `(path, previous body)`.
    RestoreBodies(Vec<(PathBuf, String)>),
}

/// What a diagram note's grid card shows besides its title.
pub(super) struct CanvasPreview {
    pub(super) summary: String,
    pub(super) snippet: String,
    pub(super) thumb: crate::canvas::thumb::CanvasThumb,
}

/// Data derived from the vault that used to be recomputed every frame
/// (the file tree even walked the disk). Rebuilt only by
/// `refresh_derived`, whenever the vault or PDF list changes.
#[derive(Default)]
struct Derived {
    file_tree: Option<ui::FileTreeNode>,
    tags: Vec<(String, usize)>,
    counts: ui::SidebarCounts,
    /// PDF and sheet path → file size, for their grid cards.
    file_sizes: HashMap<PathBuf, u64>,
    /// Canvas note path → card preview (summary, snippet, thumbnail).
    canvas_previews: HashMap<PathBuf, CanvasPreview>,
    recent_vaults: Vec<PathBuf>,
    /// Every CSV/XLSX file in the vault (§3.8), for link resolution.
    sheets: Vec<PathBuf>,
}

pub struct MnemonicApp {
    locales: LocaleManager,
    settings: AppSettings,
    theme_mode: ui::ThemeMode,
    applied_theme: Option<ui::ThemeMode>,

    vault: Option<Vault>,
    watcher: Option<VaultWatcher>,
    ignore_watcher_until: Option<Instant>,
    /// A watcher event arrived inside the self-write grace window; it may
    /// also contain a real external change, so rescan once things settle.
    deferred_rescan: bool,
    /// Set after a close was cancelled because saving failed, so a second
    /// close request is honored instead of trapping the user.
    close_blocked_once: bool,
    index: Option<IndexStore>,
    indexer: Option<IndexingWorker>,
    generator: Option<GenerationWorker>,
    index_jobs_pending: usize,
    /// Bumped whenever the index's notes, links or vectors change, so
    /// panels derived from it (backlinks, related, local graph) refresh.
    index_generation: u64,
    derived: Derived,

    /// Full-screen vault graph, when open.
    graph: Option<GraphView>,
    /// The index changed while the graph was open; rebuild its data.
    graph_dirty: bool,

    editor: Option<MarkdownEditor>,
    editor_ui: EditorUi,
    markdown_cache: CommonMarkCache,
    /// Reading themes: built-ins plus plugins (§3.2.5).
    themes: crate::reading_theme::ThemeRegistry,
    /// PDF exports rendering in the background.
    pending_exports: Vec<reading::PendingExport>,

    pdf_renderer: PdfRendererState,
    pdf_documents: Vec<PathBuf>,
    pdf_viewer: Option<PdfViewerState>,
    /// Open CSV/XLSX sheet (§3.8), when a sheet is the current document.
    sheet_viewer: Option<SheetViewerState>,

    doc_filter: SidebarDocFilter,
    sort_mode: SortMode,
    search_text: String,
    search_changed_at: Option<Instant>,
    search_pending_id: Option<Uuid>,
    search_rerank_id: Option<Uuid>,
    search_results: Vec<SearchHit>,
    selection_mode: bool,
    selected: HashSet<Uuid>,

    sidebar_open: bool,
    expanded_folders: HashSet<PathBuf>,

    chat_sidebar_open: bool,
    chat_input: String,
    chat_messages: Vec<ChatMessage>,
    chat_pending_embed_id: Option<Uuid>,
    chat_pending_gen_id: Option<Uuid>,

    show_label_manager: bool,
    tag_rename: Option<(String, String)>,
    confirm_delete: Option<Uuid>,
    confirm_empty_trash: bool,
    prompt_modal: Option<PromptModalState>,
    move_modal: Option<MoveModalState>,
    /// A background indexing/embedding error was already shown this session.
    index_error_shown: bool,
    /// Open while the note in the editor changed on disk and has unsaved
    /// edits; autosave pauses until the user picks a resolution.
    conflict_modal: bool,
    show_shortcuts: bool,
    command_palette: ui::CommandPalette,
    toasts: ui::Toasts<ToastAction>,
    focus_search: bool,
}

impl Default for MnemonicApp {
    fn default() -> Self {
        Self::new()
    }
}

impl MnemonicApp {
    pub fn new() -> MnemonicApp {
        let mut locales = LocaleManager::load(&locales_dir());
        let settings = settings::load();
        if let Some(locale) = &settings.locale {
            locales.set_active(locale);
        }

        let mut app = MnemonicApp {
            locales,
            theme_mode: ui::ThemeMode::from_str_or_default(&settings.theme),
            applied_theme: None,
            sidebar_open: settings.sidebar_open,
            vault: None,
            watcher: None,
            ignore_watcher_until: None,
            deferred_rescan: false,
            close_blocked_once: false,
            index: None,
            // Spawning is cheap (just a background thread); the actual
            // FastEmbed/Candle model only loads lazily on the first
            // submitted job, so this never blocks startup (§6 risk 2).
            indexer: Some(IndexingWorker::spawn()),
            generator: Some(GenerationWorker::spawn()),
            index_jobs_pending: 0,
            index_generation: 0,
            derived: Derived::default(),
            graph: None,
            graph_dirty: false,
            editor: None,
            editor_ui: EditorUi::default(),
            markdown_cache: CommonMarkCache::default(),
            themes: crate::reading_theme::ThemeRegistry::load(&crate::reading_theme::theme_dirs(None)),
            pending_exports: Vec::new(),
            pdf_renderer: PdfRendererState::Uninit,
            pdf_documents: Vec::new(),
            pdf_viewer: None,
            sheet_viewer: None,
            doc_filter: SidebarDocFilter::All,
            sort_mode: SortMode::Modified,
            search_text: String::new(),
            search_changed_at: None,
            search_pending_id: None,
            search_rerank_id: None,
            search_results: Vec::new(),
            selection_mode: false,
            selected: HashSet::new(),
            expanded_folders: HashSet::new(),
            chat_sidebar_open: false,
            chat_input: String::new(),
            chat_messages: Vec::new(),
            chat_pending_embed_id: None,
            chat_pending_gen_id: None,
            show_label_manager: false,
            tag_rename: None,
            confirm_delete: None,
            confirm_empty_trash: false,
            prompt_modal: None,
            index_error_shown: false,
            conflict_modal: false,
            move_modal: None,
            show_shortcuts: false,
            command_palette: ui::CommandPalette::default(),
            toasts: ui::Toasts::default(),
            focus_search: false,
            settings,
        };

        if let Some(path) = app.settings.vault_path.clone().map(PathBuf::from)
            && path.is_dir()
        {
            match Vault::open(path) {
                Ok(vault) => app.activate_vault(vault),
                Err(e) => log::warn!("app: failed to reopen last vault: {e}"),
            }
        }
        app.derived.recent_vaults = app.settings.existing_recent_vaults();
        app
    }

    // ─── i18n, errors, settings ──────────────────────────────────────────

    fn t(&self, key: &str) -> String {
        self.locales.t(key, &[])
    }

    fn t_args(&self, key: &str, args: &[(&str, &str)]) -> String {
        self.locales.t(key, args)
    }

    /// Logs a failure and shows it as an error toast, so a failed
    /// save/delete/move is never silently swallowed (§Fase 10).
    fn report_error(&mut self, context_key: &str, err: impl std::fmt::Display) {
        log::warn!("app: {context_key}: {err}");
        let context = self.t(context_key);
        let msg = self.t_args(
            "error-banner",
            &[("context", &context), ("error", &err.to_string())],
        );
        self.toasts.push(ToastKind::Error, msg);
    }

    fn toast(&mut self, kind: ToastKind, key: &str, args: &[(&str, &str)]) {
        let msg = self.t_args(key, args);
        self.toasts.push(kind, msg);
    }

    fn persist_settings(&mut self) {
        self.settings.theme = self.theme_mode.as_str().to_string();
        self.settings.locale = Some(self.locales.active_locale().to_string());
        self.settings.sidebar_open = self.sidebar_open;
        if let Err(e) = settings::save(&self.settings) {
            log::warn!("app: failed to save settings: {e}");
        }
    }

    // ─── Vault lifecycle ─────────────────────────────────────────────────

    fn activate_vault(&mut self, vault: Vault) {
        self.close_document();
        self.close_graph_view();
        self.doc_filter = SidebarDocFilter::All;
        self.expanded_folders.clear();
        self.search_text.clear();
        self.search_results.clear();
        self.selected.clear();
        self.selection_mode = false;
        self.chat_messages.clear();

        // Purge expired trash on open, per §3.1.4.
        if let Err(e) = trash::purge_expired(&vault.root, trash::DEFAULT_RETENTION) {
            log::warn!("app: trash purge failed: {e}");
        }

        // Set when the vector cache was built by another embedding model
        // (or is new): PDFs must then be re-embedded too, not just notes.
        let mut reindex_pdfs = false;
        match IndexStore::open(&vault.root) {
            Ok(mut index) => {
                match index.ensure_embedding_model(EMBEDDING_MODEL_ID, EMBEDDING_DIM) {
                    Ok(reset) => reindex_pdfs = reset,
                    Err(e) => log::warn!("app: preparing vector index failed: {e:#}"),
                }
                if let Err(e) = index.rebuild(&vault.notes) {
                    log::warn!("app: index rebuild failed: {e}");
                }
                self.index = Some(index);
            }
            Err(e) => log::warn!("app: failed to open index store: {e:#}"),
        }

        // Populate the chunk cache for search/RAG: only notes whose text
        // changed since they were last embedded (incremental, §Fase 2), so
        // reopening a big vault is instant.
        self.index_jobs_pending = 0;
        let stale: Vec<Note> = vault
            .notes
            .iter()
            .filter(|n| !n.frontmatter.trashed)
            .filter(|n| self.note_needs_reindex(n))
            .cloned()
            .collect();
        for note in &stale {
            self.reindex_note(note);
        }

        self.watcher = match VaultWatcher::watch(&vault.root) {
            Ok(w) => Some(w),
            Err(e) => {
                log::warn!("app: failed to start file watcher: {e}");
                None
            }
        };

        self.settings.remember_vault(&vault.root);
        self.persist_settings();
        self.vault = Some(vault);
        self.reload_themes();
        self.refresh_pdf_documents();
        if reindex_pdfs {
            for pdf in self.pdf_documents.clone() {
                self.submit_pdf_for_indexing(pdf);
            }
        }
        self.reindex_changed_sheets();
    }

    fn open_vault_at(&mut self, folder: PathBuf) {
        match Vault::open(folder) {
            Ok(v) => self.activate_vault(v),
            Err(e) => self.report_error("error-context-open-vault", e),
        }
    }

    fn pick_and_open_vault(&mut self) {
        if let Some(folder) = rfd::FileDialog::new()
            .set_title(self.t("vault-pick-folder"))
            .pick_folder()
        {
            self.open_vault_at(folder);
        }
    }

    /// "Create new vault": the user names a new folder, which is created
    /// and seeded with a welcome note explaining the basics.
    fn create_vault(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(self.t("welcome-create-vault"))
            .set_file_name(self.t("welcome-default-vault-name"))
            .save_file()
        else {
            return;
        };
        if let Err(e) = std::fs::create_dir_all(&path) {
            self.report_error("error-context-open-vault", e);
            return;
        }
        let has_notes = walkdir::WalkDir::new(&path)
            .max_depth(3)
            .into_iter()
            .flatten()
            .any(|e| e.path().extension().is_some_and(|x| x == "md"));
        if !has_notes
            && let Err(e) = Note::create(
                &path,
                &self.t("welcome-note-title"),
                &self.t("welcome-note-body"),
            )
        {
            log::warn!("app: failed to seed welcome note: {e}");
        }
        self.open_vault_at(path);
    }

    fn rescan_and_reindex(&mut self) {
        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        if let Err(e) = vault.rescan() {
            log::warn!("app: vault rescan failed: {e}");
            return;
        }
        if let Some(index) = self.index.as_mut()
            && let Err(e) = index.rebuild(&vault.notes)
        {
            log::warn!("app: index rebuild after rescan failed: {e}");
        }
        self.index_changed();
        self.reindex_changed_notes();
        self.reindex_changed_sheets();
        self.reload_editor_if_changed_on_disk();
        self.sync_editor_frontmatter();
        self.refresh_derived();
    }

    /// An open note that was edited outside the app (another editor, a
    /// sync client, an AI agent writing to the vault) is re-read from disk
    /// as long as the editor holds no unsaved changes of its own — the
    /// same live behavior Obsidian has. With unsaved edits the next save
    /// raises a conflict instead (see `save_editor_now`).
    fn reload_editor_if_changed_on_disk(&mut self) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if editor.is_dirty() || self.conflict_modal || !editor.has_external_change() {
            return;
        }
        match editor.reload_from_disk() {
            Ok(()) => {
                let title = editor.note.frontmatter.title.clone();
                self.editor_ui.title_buffer = title.clone();
                self.toast(ToastKind::Info, "toast-note-reloaded", &[("title", &title)]);
            }
            Err(e) => log::warn!("app: reloading externally changed note failed: {e:#}"),
        }
    }

    /// Marks everything derived from the index as stale.
    fn index_changed(&mut self) {
        self.index_generation = self.index_generation.wrapping_add(1);
        self.graph_dirty = self.graph.is_some();
    }

    /// Copies metadata changed elsewhere (tags, pin, color, archive) from
    /// the freshly scanned note into the open editor. The editor only ever
    /// changes the title and body itself, so those are left alone — this way
    /// its next autosave can't write stale metadata back to disk.
    fn sync_editor_frontmatter(&mut self) {
        let (Some(editor), Some(vault)) = (self.editor.as_mut(), self.vault.as_ref()) else {
            return;
        };
        if let Some(disk) = vault.notes.iter().find(|n| n.path == editor.note.path) {
            let fm = &mut editor.note.frontmatter;
            fm.tags = disk.frontmatter.tags.clone();
            fm.pinned = disk.frontmatter.pinned;
            fm.color = disk.frontmatter.color.clone();
            fm.archived = disk.frontmatter.archived;
            fm.reminder = disk.frontmatter.reminder;
        }
    }

    fn refresh_derived(&mut self) {
        let mut derived = Derived {
            recent_vaults: self.settings.existing_recent_vaults(),
            ..Default::default()
        };
        if let Some(vault) = &self.vault {
            derived.file_tree =
                ui::FileTreeNode::build(&vault.root, &vault.notes, &self.pdf_documents);
            derived.sheets = crate::sheet::find_sheets(&vault.root);
            derived.tags = tags::all_tags(&vault.notes);
            let mut c = ui::SidebarCounts::default();
            for note in &vault.notes {
                let fm = &note.frontmatter;
                if fm.trashed {
                    c.trashed += 1;
                    continue;
                }
                if note.is_pure_canvas() {
                    let doc = load_canvas_for_preview(note);
                    derived.canvas_previews.insert(
                        note.path.clone(),
                        CanvasPreview {
                            summary: canvas_summary(&self.locales, &doc),
                            snippet: query::snippet(&doc.extract_searchable_text(), 160),
                            thumb: crate::canvas::thumb::CanvasThumb::from_doc(&doc),
                        },
                    );
                }
                if fm.archived {
                    c.archived += 1;
                } else if note.is_pure_canvas() {
                    c.canvases += 1;
                } else {
                    c.notes += 1;
                }
            }
            c.pdfs = self.pdf_documents.len();
            c.sheets = derived.sheets.len();
            c.all = c.notes + c.canvases + c.pdfs + c.sheets;
            derived.counts = c;
            for file in self.pdf_documents.iter().chain(&derived.sheets) {
                if let Ok(meta) = std::fs::metadata(file) {
                    derived.file_sizes.insert(file.clone(), meta.len());
                }
            }
        }
        self.derived = derived;
    }

    // ─── Background work ─────────────────────────────────────────────────

    /// Submits `note` for background re-chunking + re-embedding (§Fase 7).
    fn reindex_note(&mut self, note: &Note) {
        if let Some(indexer) = &self.indexer {
            indexer.submit_note(note.clone());
            self.index_jobs_pending += 1;
        }
    }

    /// `true` unless the index already holds chunks for exactly this text.
    fn note_needs_reindex(&self, note: &Note) -> bool {
        let Some(index) = self.index.as_ref() else {
            return true;
        };
        let hash = crate::core::ingestion::note_content_hash(note);
        index.needs_reindex(note.frontmatter.id, &hash).unwrap_or(true)
    }

    /// After a rescan: (re)embeds every non-trashed note whose text differs
    /// from what the index holds — this is what makes edits made by an
    /// external editor or an AI agent searchable without a restart — and
    /// prunes chunks of notes that vanished.
    fn reindex_changed_notes(&mut self) {
        let Some(vault) = self.vault.as_ref() else {
            return;
        };
        let live: HashSet<Uuid> = vault
            .notes
            .iter()
            .filter(|n| !n.frontmatter.trashed)
            .map(|n| n.frontmatter.id)
            .collect();
        let stale: Vec<Note> = vault
            .notes
            .iter()
            .filter(|n| !n.frontmatter.trashed)
            .filter(|n| self.note_needs_reindex(n))
            .cloned()
            .collect();
        if let Some(index) = self.index.as_ref()
            && let Err(e) = index.prune_notes_not_in(&live)
        {
            log::warn!("app: pruning stale chunks failed: {e:#}");
        }
        for note in &stale {
            self.reindex_note(note);
        }
    }

    fn submit_pdf_for_indexing(&mut self, path: PathBuf) {
        if let Some(indexer) = &self.indexer {
            indexer.submit_pdf(path);
            self.index_jobs_pending += 1;
        }
    }

    /// Everything that must happen every frame regardless of screen.
    fn poll_background(&mut self, ctx: &egui::Context) {
        if let Some(watcher) = self.watcher.as_mut()
            && watcher.poll_rescan_needed()
        {
            let own_write = self
                .ignore_watcher_until
                .is_some_and(|until| Instant::now() < until);
            if own_write {
                // The burst may also contain a genuine external edit; check
                // once our own writes have settled instead of dropping it.
                self.deferred_rescan = true;
            } else {
                self.deferred_rescan = false;
                self.rescan_and_reindex();
                ctx.request_repaint();
            }
        }
        if self.deferred_rescan
            && self
                .ignore_watcher_until
                .is_none_or(|until| Instant::now() >= until + SELF_WRITE_GRACE)
        {
            self.deferred_rescan = false;
            self.rescan_and_reindex();
            ctx.request_repaint();
        }

        self.poll_indexer_results();
        self.poll_search_and_chat(ctx);
        self.poll_exports();

        // Idle autosave (§3.2.4 debounce). egui only repaints on input, so
        // schedule a wake-up for when the debounce window elapses.
        if let Some(editor) = self.editor.as_ref()
            && editor.is_dirty()
            && !self.conflict_modal
        {
            if editor.should_autosave() {
                self.save_editor_now();
            } else {
                ctx.request_repaint_after(crate::markdown::editor::AUTOSAVE_DEBOUNCE);
            }
        }

        if let Some(changed_at) = self.search_changed_at {
            if changed_at.elapsed() >= SEARCH_DEBOUNCE {
                self.search_changed_at = None;
                self.run_semantic_search();
            } else {
                ctx.request_repaint_after(SEARCH_DEBOUNCE);
            }
        }

        // Keep polling the watcher / indexer while idle so external edits
        // and indexing progress show up without needing a mouse move.
        if self.index_jobs_pending > 0 || self.watcher.is_some() {
            ctx.request_repaint_after(Duration::from_millis(750));
        }
    }

    fn poll_indexer_results(&mut self) {
        let Some(indexer) = &self.indexer else { return };
        let results = indexer.poll_results();
        if results.is_empty() {
            return;
        }
        self.index_jobs_pending = self.index_jobs_pending.saturating_sub(results.len());
        self.index_generation = self.index_generation.wrapping_add(1);
        let mut index_error: Option<String> = None;
        let Some(index) = self.index.as_mut() else {
            return;
        };
        for result in results {
            match result {
                Ok(r) => {
                    match index.replace_chunks(r.doc_id, r.doc_type.as_str(), &r.title, &r.chunks)
                    {
                        Ok(()) if !r.content_hash.is_empty() => {
                            if let Err(e) = index.set_document_hash(r.doc_id, &r.content_hash) {
                                log::warn!("app: failed to record document hash: {e:#}");
                            }
                        }
                        Ok(()) => {}
                        Err(e) => {
                            log::warn!("app: failed to store indexed chunks: {e:#}");
                            index_error = Some(format!("{e:#}"));
                        }
                    }
                }
                Err(e) => {
                    log::warn!("app: background indexing failed: {e}");
                    index_error = Some(e.to_string());
                }
            }
        }
        // Surface background failures once per vault session instead of
        // leaving them in the log only (§Fase 2.6).
        if let Some(msg) = index_error
            && !self.index_error_shown
        {
            self.index_error_shown = true;
            self.report_error("error-context-indexing", msg);
        }
    }

    fn poll_search_and_chat(&mut self, ctx: &egui::Context) {
        let (query_results, rerank_results) = match &self.indexer {
            Some(indexer) => (indexer.poll_query_results(), indexer.poll_rerank_results()),
            None => (Vec::new(), Vec::new()),
        };
        {
            for (id, result) in query_results {
                if self.search_pending_id == Some(id) {
                    self.search_pending_id = None;
                    match result {
                        Ok(embedding) => self.apply_semantic_search(&embedding),
                        Err(e) => log::warn!("app: semantic search failed: {e:#}"),
                    }
                    ctx.request_repaint();
                } else if self.chat_pending_embed_id == Some(id) {
                    self.chat_pending_embed_id = None;
                    match result {
                        Ok(embedding) => self.start_chat_generation(&embedding),
                        Err(e) => {
                            let msg = format!("{}: {e:#}", self.t("chat-error"));
                            self.chat_messages
                                .push(ChatMessage::assistant(msg, Vec::new()));
                        }
                    }
                    ctx.request_repaint();
                }
            }
            for (id, result) in rerank_results {
                if self.search_rerank_id != Some(id) {
                    continue;
                }
                self.search_rerank_id = None;
                match result {
                    Ok(scores) => {
                        let hits = std::mem::take(&mut self.search_results);
                        self.search_results = search::apply_rerank(hits, &scores);
                    }
                    Err(e) => log::warn!("app: reranking search results failed: {e:#}"),
                }
                ctx.request_repaint();
            }
        }
        if self.search_pending_id.is_some() || self.search_rerank_id.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        if let Some(generator) = &self.generator {
            for (id, event) in generator.poll_events() {
                if self.chat_pending_gen_id != Some(id) {
                    continue;
                }
                match event {
                    GenerationEvent::Token(text) => {
                        if let Some(last) = self.chat_messages.last_mut() {
                            last.text.push_str(&text);
                        }
                    }
                    GenerationEvent::Done => self.chat_pending_gen_id = None,
                    GenerationEvent::Error(e) => {
                        if let Some(last) = self.chat_messages.last_mut() {
                            last.text.push_str(&format!("\n⚠ {e}"));
                        }
                        self.chat_pending_gen_id = None;
                    }
                }
                ctx.request_repaint();
            }
        }
        if self.chat_pending_embed_id.is_some() || self.chat_pending_gen_id.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    /// Shows full-text (FTS5) results right away, then kicks off query
    /// embedding; `apply_semantic_search` fuses in the semantic half when
    /// the vector arrives.
    fn run_semantic_search(&mut self) {
        let query_text = self.search_text.trim().to_string();
        self.search_rerank_id = None;
        if query_text.is_empty() {
            self.search_results.clear();
            self.search_pending_id = None;
            return;
        }
        self.search_results = self.hybrid_search(&query_text, None, SEARCH_TOP_K, true);
        let embed_text = search::ParsedQuery::parse(&query_text).text();
        if !embed_text.is_empty()
            && let Some(indexer) = &self.indexer
        {
            self.search_pending_id = Some(indexer.submit_query(embed_text));
        }
    }

    fn apply_semantic_search(&mut self, embedding: &[f32]) {
        let query_text = self.search_text.trim().to_string();
        if query_text.is_empty() {
            return;
        }
        self.search_results = self.hybrid_search(&query_text, Some(embedding), SEARCH_TOP_K, true);
        if self.settings.rerank_search
            && self.search_results.len() > 1
            && let Some(indexer) = &self.indexer
        {
            self.search_rerank_id = Some(indexer.submit_rerank(
                query_text,
                search::rerank_documents(&self.search_results),
            ));
        }
    }

    /// Hybrid retrieval: vector KNN (when `embedding` is known) + FTS5,
    /// fused with Reciprocal Rank Fusion. Failures degrade to whichever
    /// half still works.
    fn hybrid_search(
        &self,
        text: &str,
        embedding: Option<&[f32]>,
        k: usize,
        one_per_doc: bool,
    ) -> Vec<SearchHit> {
        let Some(index) = self.index.as_ref() else {
            return Vec::new();
        };
        // Obsidian-style operators (`tag:`, `path:`, `file:`, `-x`) narrow
        // the candidate set; the free text drives retrieval.
        let parsed = search::ParsedQuery::parse(text);
        let notes_by_path: HashMap<&Path, &Note> = self
            .vault
            .as_ref()
            .map(|v| v.notes.iter().map(|n| (n.path.as_path(), n)).collect())
            .unwrap_or_default();
        let passes = |chunk: &crate::core::DocumentChunk| {
            !parsed.has_filters()
                || notes_by_path
                    .get(chunk.file_path.as_path())
                    .is_some_and(|n| parsed.filters_match(n))
        };
        let fetch = if parsed.has_filters() { RETRIEVAL_CANDIDATES * 4 } else { RETRIEVAL_CANDIDATES };
        let mut semantic = match embedding.map(|e| index.knn_chunks(e, fetch)) {
            Some(Ok(hits)) => hits,
            Some(Err(e)) => {
                log::warn!("app: vector search failed: {e:#}");
                Vec::new()
            }
            None => Vec::new(),
        };
        semantic.retain(|(chunk, _)| passes(chunk));
        let mut keyword = index
            .keyword_chunks(text, fetch)
            .unwrap_or_else(|e| {
                log::warn!("app: keyword search failed: {e:#}");
                Vec::new()
            });
        keyword.retain(|hit| passes(&hit.chunk));
        search::hybrid_rank(
            semantic,
            keyword,
            HybridOptions {
                k,
                min_similarity: llm::SIMILARITY_THRESHOLD,
                one_per_doc,
            },
        )
    }

    fn send_chat_message(&mut self, text: String) {
        let text = text.trim().to_string();
        if text.is_empty()
            || self.chat_pending_embed_id.is_some()
            || self.chat_pending_gen_id.is_some()
        {
            return;
        }
        self.chat_input.clear();
        self.chat_messages.push(ChatMessage::user(text.clone()));
        if let Some(indexer) = &self.indexer {
            self.chat_pending_embed_id = Some(indexer.submit_query(text));
        }
    }

    /// Retrieves context chunks for the resolved question embedding, builds
    /// the grounded RAG prompt (§3.4 point 2), and streams the answer into a
    /// new assistant bubble tagged with its citations.
    fn start_chat_generation(&mut self, embedding: &[f32]) {
        let question = self
            .chat_messages
            .last()
            .map(|m| m.text.clone())
            .unwrap_or_default();

        let hits = self.hybrid_search(&question, Some(embedding), CHAT_TOP_K, false);
        let context = llm::select_context(&hits, llm::SIMILARITY_THRESHOLD);
        let citations: Vec<Citation> = context
            .iter()
            .map(|c| Citation {
                file_path: c.file_path.clone(),
                page_index: c.page_num.map(|p| p.saturating_sub(1)),
                label: self.document_label(&c.file_path, c.page_num),
            })
            .collect();
        let prompt = llm::build_rag_prompt(&context, &question);

        self.chat_messages
            .push(ChatMessage::assistant(String::new(), citations));
        if let Some(generator) = &self.generator {
            self.chat_pending_gen_id = Some(generator.submit(prompt, llm::DEFAULT_MAX_TOKENS));
        }
    }

    /// Human label for a document: the note title (not its uuid file name)
    /// or the PDF file name, plus the page when known.
    fn document_label(&self, path: &Path, page: Option<usize>) -> String {
        let name = self
            .vault
            .as_ref()
            .and_then(|v| v.notes.iter().find(|n| n.path == path))
            .map(|n| n.frontmatter.title.clone())
            .unwrap_or_else(|| {
                path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string())
            });
        match page {
            Some(row) if crate::sheet::is_sheet_path(path) => self.t_args(
                "chat-citation-row",
                &[("name", &name), ("row", &row.to_string())],
            ),
            Some(p) => self.t_args(
                "chat-citation-page",
                &[("name", &name), ("page", &p.to_string())],
            ),
            None => name,
        }
    }

    // ─── Opening & closing documents ─────────────────────────────────────

    /// Saves the open note now. Returns false if saving failed.
    fn save_editor_now(&mut self) -> bool {
        let Some(editor) = self.editor.as_mut() else {
            return true;
        };
        if !editor.is_dirty() {
            return true;
        }
        let path_before = editor.note.path.clone();
        match editor.autosave() {
            Ok(()) => {
                self.editor_ui.save_failed = false;
                self.ignore_watcher_until = Some(Instant::now() + SELF_WRITE_GRACE);
                let saved = editor.note.clone();
                let path_changed = saved.path != path_before;
                let mut old_title = None;
                if let Some(vault) = self.vault.as_mut() {
                    match vault
                        .notes
                        .iter_mut()
                        .find(|n| n.frontmatter.id == saved.frontmatter.id || n.path == path_before)
                    {
                        Some(existing) => {
                            if existing.frontmatter.title != saved.frontmatter.title {
                                old_title = Some(existing.frontmatter.title.clone());
                            }
                            *existing = saved.clone();
                        }
                        None => vault.notes.push(saved.clone()),
                    }
                }
                if path_changed {
                    // The file tree shows file names; keep it in step with
                    // the rename.
                    self.refresh_derived();
                }
                // Keep backlinks/graph current without a full rescan.
                if let Some(index) = self.index.as_mut()
                    && let Err(e) = index.upsert_note(&saved)
                {
                    log::warn!("app: updating note in index failed: {e:#}");
                }
                self.index_changed();
                if let Some(old) = old_title {
                    self.refresh_derived();
                    self.propagate_rename(
                        saved.frontmatter.id,
                        &old,
                        &saved.frontmatter.title,
                    );
                }
                true
            }
            Err(SaveError::Conflict) => {
                editor.postpone_autosave();
                self.conflict_modal = true;
                false
            }
            Err(SaveError::Io(e)) => {
                editor.postpone_autosave();
                if !self.editor_ui.save_failed {
                    self.editor_ui.save_failed = true;
                    self.report_error("error-context-autosave", e);
                }
                false
            }
        }
    }

    /// Applies the user's answer to the "changed on disk" dialog.
    fn resolve_conflict(&mut self, choice: ui::ConflictChoice) {
        self.conflict_modal = false;
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        match choice {
            ui::ConflictChoice::Cancel => editor.postpone_autosave(),
            ui::ConflictChoice::Reload => {
                if let Err(e) = editor.reload_from_disk() {
                    self.report_error("error-context-autosave", e);
                    return;
                }
                self.editor_ui.title_buffer = editor.note.frontmatter.title.clone();
                self.rescan_and_reindex();
            }
            ui::ConflictChoice::Overwrite => {
                match editor.force_save() {
                    Ok(()) => {
                        self.ignore_watcher_until = Some(Instant::now() + SELF_WRITE_GRACE);
                        let note = editor.note.clone();
                        self.reindex_note(&note);
                        self.rescan_and_reindex();
                    }
                    Err(e) => self.report_error("error-context-autosave", e),
                }
            }
            ui::ConflictChoice::SaveCopy => match editor.save_conflict_copy() {
                Ok(copy) => {
                    self.ignore_watcher_until = Some(Instant::now() + SELF_WRITE_GRACE);
                    let title = copy.frontmatter.title.clone();
                    self.rescan_and_reindex();
                    self.toast(ToastKind::Success, "toast-conflict-copy-saved", &[("title", &title)]);
                }
                Err(e) => self.report_error("error-context-save-note", e),
            },
        }
    }

    /// Opens today's daily note (`Daily/YYYY-MM-DD.md`), creating it from
    /// `Templates/Daily.md` when it doesn't exist yet (Obsidian's Daily
    /// notes, §Fase 1.5). ⌘D / palette.
    fn open_daily_note(&mut self) {
        let Some(root) = self.vault.as_ref().map(|v| v.root.clone()) else {
            return;
        };
        let now = chrono::Local::now();
        let path = crate::notes::templates::daily_note_path(&root, now);
        if let Some(note) = self.note_by_path(&path).or_else(|| Note::load(&path).ok()) {
            self.open_note(note);
            return;
        }
        self.close_document();
        if self.editor.is_some() {
            return;
        }
        let title = crate::notes::templates::daily_note_title(now);
        let body = crate::notes::templates::daily_note_body(&root, now);
        let dir = root.join(crate::notes::templates::DAILY_DIR);
        match Note::create(&dir, &title, &body) {
            Ok(note) => {
                self.expanded_folders.insert(dir);
                self.rescan_and_reindex();
                self.open_note(note);
            }
            Err(e) => self.report_error("error-context-create-note", e),
        }
    }

    /// Inserts the template at `path` (placeholders expanded) at the end
    /// of the open note (Obsidian's Templates, §Fase 1.5).
    fn insert_template(&mut self, path: &Path) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(e) => {
                self.report_error("error-context-open-vault", e);
                return;
            }
        };
        let (_, template_body) = crate::notes::frontmatter::parse(&raw);
        let expanded = crate::notes::templates::expand(
            &template_body,
            &editor.note.frontmatter.title,
            chrono::Local::now(),
        );
        let mut body = editor.note.body.clone();
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        body.push_str(&expanded);
        editor.set_body(body);
    }

    /// Renames every legacy `<uuid>.md` note file after its title
    /// (Obsidian's file-name-is-note-name convention). Links are title
    /// based, so nothing else needs rewriting. Palette command.
    fn migrate_uuid_file_names(&mut self) {
        self.close_document();
        if self.editor.is_some() {
            return;
        }
        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        let mut renamed = 0usize;
        let mut first_error = None;
        for note in vault.notes.iter_mut() {
            if !note.has_uuid_file_name() || note.frontmatter.trashed {
                continue;
            }
            match note.sync_file_name_with_title() {
                Ok(Some(_)) => renamed += 1,
                Ok(None) => {}
                Err(e) => {
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                }
            }
        }
        if let Some(e) = first_error {
            self.report_error("error-context-move-file", e);
        }
        self.ignore_watcher_until = Some(Instant::now() + SELF_WRITE_GRACE);
        self.rescan_and_reindex();
        self.toast(
            ToastKind::Success,
            "toast-filenames-migrated",
            &[("count", &renamed.to_string())],
        );
    }

    /// Applies the title typed into the top bar / inline title to the open
    /// note (renaming its file and updating links via the save path).
    fn commit_title_buffer(&mut self) {
        let title = self.editor_ui.title_buffer.trim().to_string();
        let mut changed = false;
        if let Some(editor) = self.editor.as_mut() {
            if title.is_empty() {
                self.editor_ui.title_buffer = editor.note.frontmatter.title.clone();
            } else if title != editor.note.frontmatter.title {
                editor.set_title(title);
                changed = true;
            }
        }
        if changed {
            self.save_editor_now();
        }
    }

    /// Saves and closes whatever document is open, returning to the grid.
    fn close_document(&mut self) {
        if let Some(editor) = self.editor.as_ref() {
            let note = editor.note.clone();
            let was_dirty = editor.is_dirty();
            let saved = self.save_editor_now();
            if !saved {
                // Keep the editor open rather than silently dropping edits.
                return;
            }
            self.editor = None;
            if was_dirty {
                self.reindex_note(&note);
            }
            self.rescan_and_reindex();
        }
        self.pdf_viewer = None;
        // A sheet that fails to save stays open so its edits aren't lost.
        if self.sheet_viewer.is_some() && self.save_sheet_now() {
            self.sheet_viewer = None;
        }
    }

    /// Whether a sheet or note document is open (PDFs and the graph don't
    /// hold unsaved edits).
    fn document_open(&self) -> bool {
        self.editor.is_some() || self.pdf_viewer.is_some() || self.sheet_viewer.is_some()
    }

    fn open_note(&mut self, note: Note) {
        if self
            .editor
            .as_ref()
            .is_some_and(|e| e.note.path == note.path)
        {
            return;
        }
        self.close_document();
        if self.editor.is_some() || self.sheet_viewer.is_some() {
            return; // current document failed to save; stay on it
        }
        self.editor_ui = EditorUi::for_title(&note.frontmatter.title);
        let root = self.vault.as_ref().map(|v| v.root.clone());
        let mut editor = MarkdownEditor::open_in(note, root.as_deref());
        if self.doc_filter == SidebarDocFilter::WhiteboardsOnly && editor.canvas.is_some() {
            editor.mode = EditorMode::Edgeless;
        }
        self.editor = Some(editor);
    }

    /// Opens any file (note, canvas, PDF, sheet) by path.
    fn open_file_by_path(&mut self, path: PathBuf) {
        if crate::sheet::is_sheet_path(&path) {
            if self.sheet_viewer.as_ref().is_some_and(|v| v.path == path) {
                return;
            }
            self.close_document();
            if self.editor.is_none() && self.sheet_viewer.is_none() {
                self.open_sheet(path);
            }
            return;
        }
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
        {
            self.close_document();
            if self.editor.is_none() {
                self.open_pdf(path);
            }
            return;
        }
        let note = self.note_by_path(&path).or_else(|| Note::load(&path).ok());
        match note {
            Some(note) => self.open_note(note),
            None => self.toast(ToastKind::Error, "toast-open-failed", &[]),
        }
    }

    fn create_note(&mut self, parent_dir: Option<PathBuf>, canvas: bool) {
        let Some(root) = self.vault.as_ref().map(|v| v.root.clone()) else {
            return;
        };
        let target_dir = parent_dir.unwrap_or_else(|| root.clone());
        self.close_document();
        if self.editor.is_some() {
            return;
        }
        let result = if canvas {
            Note::create_canvas(&target_dir, &self.t("canvas-untitled"))
        } else {
            Note::create(&target_dir, &self.t("editor-untitled"), "")
        };
        match result {
            Ok(note) => {
                if target_dir != root {
                    self.expanded_folders.insert(target_dir);
                }
                if matches!(
                    self.doc_filter,
                    SidebarDocFilter::Archived | SidebarDocFilter::Trashed
                ) {
                    self.doc_filter = SidebarDocFilter::All;
                }
                self.rescan_and_reindex();
                self.open_note(note);
                // Select the placeholder title so typing immediately names
                // the new note.
                self.editor_ui.select_title = !canvas;
            }
            Err(e) => self.report_error("error-context-create-note", e),
        }
    }

    /// Follows a clicked `[[wikilink]]` reference (`Title`, `Title#Heading`
    /// or `file.pdf#page=N`), creating the note if it doesn't exist yet —
    /// Obsidian's click-to-create (§3.2.2).
    fn navigate_wikilink(&mut self, reference: &str) {
        let link = WikiLink::parse(reference);
        if link.target.is_empty() {
            return;
        }
        self.close_graph_view();
        let key = title_key(&link.target);

        if crate::sheet::is_sheet_path(std::path::Path::new(&link.target)) {
            let sheet = self.derived.sheets.iter().find(|p| {
                p.file_name()
                    .is_some_and(|n| title_key(&n.to_string_lossy()) == key)
            });
            match sheet.cloned() {
                // `[[Budget.csv#row=12]]` jumps to a data row.
                Some(path) => match link.heading.as_deref().and_then(|h| h.strip_prefix("row=")) {
                    Some(row) => self.open_chunk_source(path, row.trim().parse().ok()),
                    None => self.open_file_by_path(path),
                },
                None => self.toast(ToastKind::Error, "toast-link-sheet-missing", &[("name", &link.target)]),
            }
            return;
        }

        if link.is_pdf() {
            let pdf = self.pdf_documents.iter().find(|p| {
                p.file_name()
                    .is_some_and(|n| title_key(&n.to_string_lossy()) == key)
            });
            match pdf.cloned() {
                Some(path) => {
                    self.close_document();
                    if self.editor.is_none() {
                        match link.page() {
                            Some(page) => self.open_pdf_at_page(path, page - 1),
                            None => self.open_pdf(path),
                        }
                    }
                }
                None => self.toast(ToastKind::Error, "toast-link-pdf-missing", &[("name", &link.target)]),
            }
            return;
        }

        // Title, stem, alias or `Folder/Name`; among same-named notes the
        // one next to the note being read wins (Obsidian's rule).
        let source = self.editor.as_ref().map(|e| e.note.path.clone());
        let existing = self.vault.as_ref().and_then(|vault| {
            let index = wikilink::WikilinkIndex::build(&vault.notes);
            let path = index.resolve_from(&link.target, source.as_deref())?.to_path_buf();
            vault.notes.iter().find(|n| n.path == path).cloned()
        });
        if let Some(note) = existing {
            self.open_note(note);
            if let (Some(heading), Some(editor)) = (&link.heading, self.editor.as_mut()) {
                // Headings and `^block` anchors are scrolled to in the
                // rendered (Live) view.
                if editor.mode == EditorMode::Source {
                    editor.mode = EditorMode::Live;
                }
                if let Some(block_id) = heading.strip_prefix('^') {
                    editor.scroll_to_block(block_id);
                } else {
                    editor.scroll_to_heading(&crate::markdown::renderer::slugify(heading));
                }
            }
            return;
        }
        let Some(root) = self.vault.as_ref().map(|v| v.root.clone()) else {
            return;
        };
        self.close_document();
        if self.editor.is_some() {
            return;
        }
        // `[[Folder/New]]` creates `New` inside `Folder/` (never outside
        // the vault or in a hidden folder).
        let (dir, title) = match link.target.rsplit_once(['/', '\\']) {
            Some((folder, name)) => {
                let safe = std::path::Path::new(folder).components().all(|c| {
                    matches!(c, std::path::Component::Normal(p) if !p.to_string_lossy().starts_with('.'))
                });
                if !safe {
                    self.toast(ToastKind::Error, "toast-link-folder-invalid", &[("name", &link.target)]);
                    return;
                }
                (root.join(folder), name.trim().to_string())
            }
            None => (root, link.target.clone()),
        };
        let created = std::fs::create_dir_all(&dir)
            .map_err(anyhow::Error::from)
            .and_then(|()| Note::create(&dir, &title, ""));
        match created {
            Ok(note) => {
                self.rescan_and_reindex();
                self.open_note(note);
            }
            Err(e) => self.report_error("error-context-create-note", e),
        }
    }

    /// After note `renamed_id` changed title from `old_title` to
    /// `new_title`, rewrites `[[old_title…]]` links in every other note
    /// (Obsidian's "update internal links"), with an undo toast.
    fn propagate_rename(&mut self, renamed_id: Uuid, old_title: &str, new_title: &str) {
        if title_key(old_title) == title_key(new_title) && old_title.trim() == new_title.trim() {
            return;
        }
        let open_path = self.editor.as_ref().map(|e| e.note.path.clone());
        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        // The renamed note's folder, so `[[Folder/Old]]` links follow too.
        let folder = vault
            .notes
            .iter()
            .find(|n| n.frontmatter.id == renamed_id)
            .and_then(|n| n.path.parent()?.strip_prefix(&vault.root).ok())
            .map(|p| p.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"));
        let mut previous: Vec<(PathBuf, String)> = Vec::new();
        let mut save_error = None;
        let mut editor_body = None;
        for note in vault.notes.iter_mut() {
            if note.frontmatter.id == renamed_id || note.frontmatter.trashed {
                continue;
            }
            let Some(body) = wikilink::rewrite_link_target_in(&note.body, old_title, new_title, folder.as_deref())
            else {
                continue;
            };
            if open_path.as_ref() == Some(&note.path) {
                // The open editor owns this body; update it there so its
                // autosave doesn't write the old links back.
                editor_body = Some(body);
                continue;
            }
            let old_body = std::mem::replace(&mut note.body, body);
            match note.save() {
                Ok(()) => previous.push((note.path.clone(), old_body)),
                Err(e) => {
                    note.body = old_body;
                    save_error = Some(e);
                }
            }
        }
        let mut updated = previous.len();
        if let (Some(body), Some(editor)) = (editor_body, self.editor.as_mut()) {
            previous.push((editor.note.path.clone(), editor.note.body.clone()));
            editor.set_body(body);
            updated += 1;
        }
        if let Some(e) = save_error {
            self.report_error("error-context-save-note", e);
        }
        if updated == 0 {
            return;
        }
        self.ignore_watcher_until = Some(Instant::now() + SELF_WRITE_GRACE);
        let changed: Vec<Note> = self
            .vault
            .as_ref()
            .map(|v| {
                v.notes
                    .iter()
                    .filter(|n| previous.iter().any(|(p, _)| *p == n.path))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        for note in &changed {
            if let Some(index) = self.index.as_mut()
                && let Err(e) = index.upsert_note(note)
            {
                log::warn!("app: updating links in index failed: {e:#}");
            }
            self.reindex_note(note);
        }
        self.index_changed();
        let msg = self.t_args("toast-links-updated", &[("count", &updated.to_string())]);
        let undo = self.t("toast-undo");
        self.toasts.push_with_action(
            ToastKind::Success,
            msg,
            undo,
            ToastAction::RestoreBodies(previous),
        );
    }

    // ─── File operations (with undo) ─────────────────────────────────────

    fn note_by_path(&self, path: &Path) -> Option<Note> {
        self.vault
            .as_ref()
            .and_then(|v| v.notes.iter().find(|n| n.path == path).cloned())
    }

    fn trash_note(&mut self, path: &Path) {
        let Some(root) = self.vault.as_ref().map(|v| v.root.clone()) else {
            return;
        };
        if self.editor.as_ref().is_some_and(|e| e.note.path == path) {
            if !self.save_editor_now() {
                return; // keep the unsaved edits open rather than trash stale content
            }
            self.editor = None;
        }
        let Some(note) = self.note_by_path(path) else {
            return;
        };
        let id = note.frontmatter.id;
        let title = note.frontmatter.title.clone();
        let original_path = note.path.clone();
        match note.move_to_trash(&root) {
            Ok(trashed) => {
                if let Some(index) = self.index.as_ref()
                    && let Err(e) = index.delete_chunks_for_doc(id)
                {
                    log::warn!("app: failed to purge chunks for trashed note: {e}");
                }
                self.selected.remove(&id);
                self.rescan_and_reindex();
                let msg = self.t_args("toast-note-trashed", &[("title", &title)]);
                let undo = self.t("toast-undo");
                self.toasts.push_with_action(
                    ToastKind::Success,
                    msg,
                    undo,
                    ToastAction::RestoreNote {
                        trashed_path: trashed.path,
                        original_path,
                    },
                );
            }
            Err(e) => self.report_error("error-context-trash-note", e),
        }
    }

    fn restore_note(&mut self, trashed_path: &Path, target: Option<PathBuf>) {
        let Some(root) = self.vault.as_ref().map(|v| v.root.clone()) else {
            return;
        };
        let Some(note) = self.note_by_path(trashed_path) else {
            return;
        };
        let title = note.frontmatter.title.clone();
        let result = match target {
            Some(original) => note.restore_to(original),
            None => note.restore_from_trash(&root),
        };
        match result {
            Ok(restored) => {
                self.rescan_and_reindex();
                self.reindex_note(&restored);
                self.toast(
                    ToastKind::Success,
                    "toast-note-restored",
                    &[("title", &title)],
                );
            }
            Err(e) => self.report_error("error-context-move-note", e),
        }
    }

    /// Moves a PDF, folder, or other file to `.trash/` with undo. Notes are
    /// routed to `trash_note` so their `trashed` flag is set.
    fn trash_path(&mut self, path: &Path, is_dir: bool, name: &str) {
        let Some(root) = self.vault.as_ref().map(|v| v.root.clone()) else {
            return;
        };
        if !is_dir && self.note_by_path(path).is_some() {
            self.trash_note(path);
            return;
        }
        if self
            .editor
            .as_ref()
            .is_some_and(|e| e.note.path.starts_with(path))
        {
            if !self.save_editor_now() {
                return;
            }
            self.editor = None;
        }
        if self
            .pdf_viewer
            .as_ref()
            .is_some_and(|p| p.path.starts_with(path))
        {
            self.pdf_viewer = None;
        }
        if self
            .sheet_viewer
            .as_ref()
            .is_some_and(|v| v.path.starts_with(path))
        {
            // Unsaved edits of a sheet being trashed go with it; the file
            // is restorable from Trash.
            self.sheet_viewer = None;
        }

        let pdfs: Vec<PathBuf> = self
            .pdf_documents
            .iter()
            .filter(|p| p.starts_with(path))
            .cloned()
            .collect();
        let note_ids: Vec<Uuid> = self
            .vault
            .as_ref()
            .map(|v| {
                v.notes
                    .iter()
                    .filter(|n| n.path.starts_with(path))
                    .map(|n| n.frontmatter.id)
                    .collect()
            })
            .unwrap_or_default();

        match trash::move_path_to_trash(&root, path) {
            Ok(trashed_path) => {
                if let Some(index) = self.index.as_ref() {
                    for pdf in &pdfs {
                        let _ = index.remove_pdf_document(pdf);
                        let _ = index.delete_chunks_for_doc(pdf_doc_id(pdf));
                    }
                    for id in note_ids {
                        let _ = index.delete_chunks_for_doc(id);
                    }
                }
                self.expanded_folders.retain(|p| !p.starts_with(path));
                self.refresh_pdf_documents();
                self.rescan_and_reindex();
                let msg = self.t_args("toast-item-trashed", &[("title", name)]);
                let undo = self.t("toast-undo");
                self.toasts.push_with_action(
                    ToastKind::Success,
                    msg,
                    undo,
                    ToastAction::RestorePath {
                        trashed_path,
                        original_path: path.to_path_buf(),
                        pdfs,
                    },
                );
            }
            Err(e) => {
                let key = if is_dir {
                    "error-context-delete-folder"
                } else {
                    "error-context-delete-note"
                };
                self.report_error(key, e);
            }
        }
    }

    fn apply_toast_action(&mut self, action: ToastAction) {
        match action {
            ToastAction::RestoreNote {
                trashed_path,
                original_path,
            } => self.restore_note(&trashed_path, Some(original_path)),
            ToastAction::RestorePath {
                trashed_path,
                original_path,
                pdfs,
            } => match trash::restore_path(&trashed_path, &original_path) {
                Ok(()) => {
                    if let Some(index) = self.index.as_ref() {
                        for pdf in &pdfs {
                            let _ = index.add_pdf_document(pdf);
                        }
                    }
                    for pdf in pdfs {
                        self.submit_pdf_for_indexing(pdf);
                    }
                    self.refresh_pdf_documents();
                    self.rescan_and_reindex();
                    let restored: Vec<Note> = self
                        .vault
                        .as_ref()
                        .map(|v| {
                            v.notes
                                .iter()
                                .filter(|n| n.path.starts_with(&original_path))
                                .cloned()
                                .collect()
                        })
                        .unwrap_or_default();
                    for note in &restored {
                        self.reindex_note(note);
                    }
                    self.toast(ToastKind::Success, "toast-restored", &[]);
                }
                Err(e) => self.report_error("error-context-move-file", e),
            },
            ToastAction::ToggleArchived(id) => {
                self.mutate_note(id, |n| n.frontmatter.archived = !n.frontmatter.archived);
            }
            ToastAction::RestoreBodies(bodies) => {
                for (path, body) in bodies {
                    if let Some(editor) = self.editor.as_mut()
                        && editor.note.path == path
                    {
                        editor.set_body(body);
                        continue;
                    }
                    if let Some(mut note) = self.note_by_path(&path) {
                        note.body = body;
                        if let Err(e) = note.save() {
                            self.report_error("error-context-save-note", e);
                            continue;
                        }
                        self.reindex_note(&note);
                    }
                }
                self.ignore_watcher_until = Some(Instant::now() + SELF_WRITE_GRACE);
                self.rescan_and_reindex();
            }
        }
    }

    /// Moves a file or folder into `dest_dir`, keeping the open editor and
    /// registered PDFs pointed at the new location.
    fn move_item_to_folder(&mut self, src_path: PathBuf, dest_dir: PathBuf) {
        if !src_path.exists() || dest_dir.starts_with(&src_path) {
            return;
        }
        let Some(file_name) = src_path.file_name() else {
            return;
        };
        if src_path.parent() == Some(dest_dir.as_path()) {
            return;
        }
        let target_path = trash::unique_path_in(&dest_dir, file_name);
        self.save_editor_now();

        if let Err(e) = std::fs::rename(&src_path, &target_path) {
            self.report_error("error-context-move-file", e);
            return;
        }

        if let Some(editor) = &mut self.editor
            && let Ok(rel) = editor.note.path.strip_prefix(&src_path)
        {
            editor.note.path = if rel.as_os_str().is_empty() {
                target_path.clone()
            } else {
                target_path.join(rel)
            };
        }
        self.relocate_pdfs(&src_path, &target_path);
        self.relocate_open_sheet(&src_path, &target_path);
        let is_root = self.vault.as_ref().is_some_and(|v| v.root == dest_dir);
        if !is_root {
            self.expanded_folders.insert(dest_dir.clone());
        }
        self.rescan_and_reindex();

        let folder = if is_root {
            self.t("move-modal-root")
        } else {
            dest_dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        };
        self.toast(ToastKind::Success, "toast-moved", &[("folder", &folder)]);
    }

    /// Re-registers indexed PDFs under `old_prefix` at their new location.
    fn relocate_pdfs(&mut self, old_prefix: &Path, new_prefix: &Path) {
        let moved: Vec<(PathBuf, PathBuf)> = self
            .pdf_documents
            .iter()
            .filter_map(|p| {
                let rel = p.strip_prefix(old_prefix).ok()?;
                let new = if rel.as_os_str().is_empty() {
                    new_prefix.to_path_buf()
                } else {
                    new_prefix.join(rel)
                };
                Some((p.clone(), new))
            })
            .collect();
        if moved.is_empty() {
            return;
        }
        if let Some(index) = self.index.as_ref() {
            for (old, new) in &moved {
                let _ = index.remove_pdf_document(old);
                let _ = index.delete_chunks_for_doc(pdf_doc_id(old));
                let _ = index.add_pdf_document(new);
            }
        }
        if let Some(viewer) = self.pdf_viewer.as_mut()
            && let Some((_, new)) = moved.iter().find(|(old, _)| *old == viewer.path)
        {
            viewer.path = new.clone();
        }
        for (_, new) in moved {
            self.submit_pdf_for_indexing(new);
        }
        self.refresh_pdf_documents();
    }

    fn rename_item(&mut self, path: PathBuf, is_dir: bool, new_name: String) {
        if is_dir {
            let Some(parent) = path.parent() else { return };
            let new_path = parent.join(&new_name);
            if new_path.exists() {
                self.toast(ToastKind::Error, "toast-name-taken", &[("name", &new_name)]);
                return;
            }
            self.save_editor_now();
            if let Err(e) = std::fs::rename(&path, &new_path) {
                self.report_error("error-context-rename-folder", e);
                return;
            }
            if self.expanded_folders.remove(&path) {
                self.expanded_folders.insert(new_path.clone());
            }
            if let Some(editor) = &mut self.editor
                && let Ok(rel) = editor.note.path.strip_prefix(&path)
            {
                editor.note.path = new_path.join(rel);
            }
            self.relocate_pdfs(&path, &new_path);
            self.rescan_and_reindex();
            return;
        }

        if let Some(mut note) = self.note_by_path(&path) {
            let open_here = self.editor.as_ref().is_some_and(|e| e.note.path == path);
            if open_here {
                if let Some(editor) = self.editor.as_mut() {
                    editor.set_title(new_name.clone());
                }
                self.editor_ui.title_buffer = new_name;
                self.save_editor_now();
            } else {
                let old_title = std::mem::replace(&mut note.frontmatter.title, new_name);
                if let Err(e) = note.save() {
                    self.report_error("error-context-save-note", e);
                    return;
                }
                if let Err(e) = note.sync_file_name_with_title() {
                    self.report_error("error-context-move-file", e);
                }
                self.reindex_note(&note);
                self.propagate_rename(note.frontmatter.id, &old_title, &note.frontmatter.title);
            }
            self.rescan_and_reindex();
            return;
        }

        // Any other file (e.g. a PDF): rename on disk, keeping its extension.
        let ext = path.extension().map(|e| e.to_string_lossy().to_string());
        let file_name = match ext {
            Some(ext)
                if !new_name
                    .to_lowercase()
                    .ends_with(&format!(".{}", ext.to_lowercase())) =>
            {
                format!("{new_name}.{ext}")
            }
            _ => new_name,
        };
        let Some(parent) = path.parent() else { return };
        let new_path = parent.join(&file_name);
        if new_path.exists() {
            self.toast(
                ToastKind::Error,
                "toast-name-taken",
                &[("name", &file_name)],
            );
            return;
        }
        if let Err(e) = std::fs::rename(&path, &new_path) {
            self.report_error("error-context-move-file", e);
            return;
        }
        self.relocate_pdfs(&path, &new_path);
        self.relocate_open_sheet(&path, &new_path);
        self.rescan_and_reindex();
    }

    /// Loads note `id`, applies `f`, saves, and re-syncs the index.
    fn mutate_note(&mut self, id: Uuid, f: impl FnOnce(&mut Note)) {
        let Some(mut note) = self
            .vault
            .as_ref()
            .and_then(|v| v.notes.iter().find(|n| n.frontmatter.id == id).cloned())
        else {
            return;
        };
        f(&mut note);
        if let Err(e) = note.save() {
            self.report_error("error-context-save-note", e);
            return;
        }
        self.rescan_and_reindex();
    }

    fn import_pdf_dialog(&mut self) {
        let Some(files) = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .pick_files()
        else {
            return;
        };
        let count = files.len();
        let last = files.last().cloned();
        for file in files {
            self.register_pdf(file);
        }
        self.refresh_pdf_documents();
        if count == 1 {
            if let Some(path) = last {
                self.close_document();
                if self.editor.is_none() {
                    self.open_pdf(path);
                }
            }
        } else if count > 1 {
            self.toast(
                ToastKind::Success,
                "toast-pdfs-imported",
                &[("count", &count.to_string())],
            );
        }
    }

    // ─── Keyboard shortcuts ──────────────────────────────────────────────

    fn modal_open(&self) -> bool {
        self.prompt_modal.is_some()
            || self.conflict_modal
            || self.move_modal.is_some()
            || self.confirm_delete.is_some()
            || self.confirm_empty_trash
            || self.show_label_manager
            || self.show_shortcuts
            || self.command_palette.is_open()
            || self
                .pdf_viewer
                .as_ref()
                .is_some_and(|v| v.has_dialog_open())
    }

    /// The chord bound to `action` (`config.toml` `[hotkeys]`, else the
    /// default), parsed; an unparseable user chord logs once per frame and
    /// falls back to the default.
    fn hotkey(&self, action: &str) -> Option<KeyboardShortcut> {
        let default = hotkeys::default_chord(action)?;
        let chord = self.settings.chord(action, default);
        hotkeys::parse_chord(chord).or_else(|| {
            log::warn!("hotkeys: cannot parse `{chord}` for `{action}`, using `{default}`");
            hotkeys::parse_chord(default)
        })
    }

    /// Display glyphs for `action`'s chord (`⌘K`), for hints and the
    /// cheat sheet.
    pub(super) fn hotkey_label(&self, action: &str) -> String {
        let default = hotkeys::default_chord(action).unwrap_or("");
        hotkeys::display(self.settings.chord(action, default))
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let pressed = |app: &Self, action: &str| {
            app.hotkey(action)
                .is_some_and(|s| ctx.input_mut(|i| i.consume_shortcut(&s)))
        };
        let palette = pressed(self, "palette");
        let new_note = pressed(self, "new_note");
        let find = pressed(self, "search");
        let save = pressed(self, "save");
        let toggle_source = pressed(self, "toggle_source");
        let print = pressed(self, "print");
        let sidebar = pressed(self, "sidebar");
        let ai = pressed(self, "ai");
        let shortcuts = pressed(self, "shortcuts");
        let graph = pressed(self, "graph");
        let daily = pressed(self, "daily");

        if palette && self.vault.is_some() {
            self.command_palette.toggle();
        }
        if shortcuts {
            self.show_shortcuts = !self.show_shortcuts;
        }
        if self.vault.is_none() || self.modal_open() {
            return;
        }
        if new_note {
            self.create_note(None, false);
        }
        if daily {
            self.open_daily_note();
        }
        if find {
            self.close_document();
            self.close_graph_view();
            self.focus_search = true;
        }
        if graph {
            if self.graph.is_some() && !self.document_open() {
                self.close_graph_view();
            } else {
                self.open_graph_view();
            }
        }
        if save && self.editor.is_some() && self.save_editor_now() {
            self.toast(ToastKind::Success, "editor-saved", &[]);
        }
        if save && self.sheet_viewer.is_some() && self.save_sheet_now() {
            self.toast(ToastKind::Success, "editor-saved", &[]);
        }
        if print && self.editor.is_some() {
            self.export_open_note(ui::ExportKind::Print);
        }
        if toggle_source && let Some(editor) = self.editor.as_mut() {
            editor.mode = match editor.mode {
                EditorMode::Source => EditorMode::Live,
                EditorMode::Live => EditorMode::Source,
                other => other,
            };
        }
        if sidebar {
            self.sidebar_open = !self.sidebar_open;
            self.persist_settings();
        }
        if ai {
            self.chat_sidebar_open = !self.chat_sidebar_open;
            if self.chat_sidebar_open {
                ctx.memory_mut(|m| m.request_focus(ui::ChatSidebarDrawer::input_id()));
            }
        }

        // Esc steps back out of a document — only when nothing has keyboard
        // focus (a first Esc leaves the text field, a second one goes back)
        // and no popup or autocomplete menu is showing.
        let editing_canvas_text = self
            .editor
            .as_ref()
            .is_some_and(|e| e.canvas_interaction.editing_text_elem.is_some());
        if (self.document_open() || self.graph.is_some())
            && !ctx.egui_wants_keyboard_input()
            && !egui::Popup::is_any_open(ctx)
            && !self.editor_ui.popup_visible
            && !editing_canvas_text
            && ctx.input(|i| i.key_pressed(Key::Escape))
        {
            self.go_back();
        }
    }

    /// Steps back one level: document → graph (if it was open) → home.
    fn go_back(&mut self) {
        if self.document_open() {
            self.close_document();
        } else {
            self.close_graph_view();
        }
    }

    // ─── Screen chrome ───────────────────────────────────────────────────

    fn show_top_bar(&mut self, ui: &mut egui::Ui) {
        let vault_open = self.vault.is_some();
        let graph_open = self.graph.is_some() && !self.document_open();
        let pdf_title = self
            .pdf_viewer
            .as_ref()
            .map(|v| sheet::file_name(&v.path))
            .or_else(|| {
                // Sheets reuse the plain document title bar; `•` marks
                // unsaved edits.
                self.sheet_viewer.as_ref().map(|v| {
                    let dirty = if v.is_dirty() { " •" } else { "" };
                    format!("{}{dirty}", sheet::file_name(&v.path))
                })
            });
        let (reading_themes, note_theme) = self.theme_menu_entries();
        let context = if !vault_open {
            ui::TopBarContext::Welcome
        } else if let Some(editor) = self.editor.as_ref() {
            ui::TopBarContext::Editor {
                title: &mut self.editor_ui.title_buffer,
                mode: match editor.mode {
                    EditorMode::Live | EditorMode::Source => ui::EditorModeTab::Note,
                    EditorMode::Edgeless | EditorMode::Split => ui::EditorModeTab::Canvas,
                },
                save_state: if self.editor_ui.save_failed {
                    ui::SaveState::Failed
                } else if editor.is_dirty() {
                    ui::SaveState::Pending
                } else {
                    ui::SaveState::Saved
                },
                can_undo: editor.can_undo(),
                can_redo: editor.can_redo(),
                outline_open: self.settings.show_outline,
            }
        } else if let Some(title) = pdf_title {
            ui::TopBarContext::Pdf { title }
        } else if self.graph.is_some() {
            ui::TopBarContext::Pdf {
                title: self.t("graph-title"),
            }
        } else {
            ui::TopBarContext::Home {
                search: &mut self.search_text,
            }
        };
        let mut state = ui::TopBarState {
            context,
            vault_open,
            sidebar_open: self.sidebar_open,
            chat_open: self.chat_sidebar_open,
            graph_open,
            theme_mode: self.theme_mode,
            indexing_jobs: self.index_jobs_pending,
            reading_themes,
            reading_theme: self.settings.reading_theme.clone(),
            note_theme,
        };
        let events = ui::TopBar::show(ui, &self.locales, &mut state);

        for event in events {
            match event {
                ui::TopBarEvent::ToggleSidebar => {
                    self.sidebar_open = !self.sidebar_open;
                    self.persist_settings();
                }
                ui::TopBarEvent::ToggleChat => {
                    self.chat_sidebar_open = !self.chat_sidebar_open;
                }
                ui::TopBarEvent::OpenCommandPalette => self.command_palette.open(),
                ui::TopBarEvent::SetTheme(mode) => {
                    self.theme_mode = mode;
                    self.persist_settings();
                }
                ui::TopBarEvent::SetLanguage(locale) => {
                    self.locales.set_active(&locale);
                    self.persist_settings();
                }
                ui::TopBarEvent::OpenVaultPicker => self.pick_and_open_vault(),
                ui::TopBarEvent::ShowShortcuts => self.show_shortcuts = true,
                ui::TopBarEvent::SearchChanged => {
                    self.search_changed_at = Some(Instant::now());
                    if self.search_text.trim().is_empty() {
                        self.search_results.clear();
                    }
                }
                ui::TopBarEvent::Back => self.go_back(),
                ui::TopBarEvent::ToggleGraph => {
                    if self.graph.is_some() && !self.document_open() {
                        self.close_graph_view();
                    } else {
                        self.open_graph_view();
                    }
                }
                ui::TopBarEvent::CommitTitle => self.commit_title_buffer(),
                ui::TopBarEvent::SetEditorMode(tab) => {
                    if let Some(editor) = self.editor.as_mut() {
                        if editor.mode.shows_canvas() {
                            editor.sync_canvas_to_body();
                        }
                        editor.mode = match tab {
                            ui::EditorModeTab::Note => EditorMode::Live,
                            ui::EditorModeTab::Canvas => {
                                editor.ensure_canvas();
                                EditorMode::Edgeless
                            }
                        };
                    }
                }
                ui::TopBarEvent::Undo => {
                    if let Some(editor) = self.editor.as_mut() {
                        editor.undo();
                    }
                }
                ui::TopBarEvent::Redo => {
                    if let Some(editor) = self.editor.as_mut() {
                        editor.redo();
                    }
                }
                ui::TopBarEvent::ToggleOutline => {
                    self.settings.show_outline = !self.settings.show_outline;
                    self.persist_settings();
                }
                ui::TopBarEvent::SetReadingTheme(id) => self.set_reading_theme(&id),
                ui::TopBarEvent::ReloadThemes => {
                    self.reload_themes();
                    let count = self.themes.list().len().to_string();
                    self.toast(ToastKind::Info, "reading-theme-reloaded", &[("count", &count)]);
                }
                ui::TopBarEvent::OpenThemeFolder => self.open_theme_folder(),
                ui::TopBarEvent::Export(kind) => self.export_open_note(kind),
            }
        }

        // Focus requests that need the widgets to exist first.
        if self.focus_search {
            ui.ctx()
                .memory_mut(|m| m.request_focus(ui::top_bar::search_field_id()));
            self.focus_search = false;
        }
        if self.editor_ui.select_title {
            let id = ui::top_bar::title_field_id();
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                let len = self.editor_ui.title_buffer.chars().count();
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(len),
                    )));
                state.store(ui.ctx(), id);
                ui.ctx().memory_mut(|m| m.request_focus(id));
                self.editor_ui.select_title = false;
            }
        }
    }

    fn show_sidebar(&mut self, ui: &mut egui::Ui) {
        let Some(vault) = self.vault.as_ref() else {
            return;
        };
        let vault_name = vault
            .root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Vault".to_string());
        let active_file = self
            .editor
            .as_ref()
            .map(|e| e.note.path.as_path())
            .or_else(|| self.pdf_viewer.as_ref().map(|p| p.path.as_path()))
            .or_else(|| self.sheet_viewer.as_ref().map(|s| s.path.as_path()));
        let state = ui::SidebarState {
            vault_root: &vault.root,
            vault_name: &vault_name,
            recent_vaults: &self.derived.recent_vaults,
            active_file_path: active_file,
            expanded_folders: &self.expanded_folders,
            file_tree: self.derived.file_tree.as_ref(),
            current_filter: &self.doc_filter,
            home_active: !self.document_open(),
            all_tags: &self.derived.tags,
            counts: self.derived.counts,
        };
        let was_open = self.sidebar_open;
        let events = ui::SidebarDrawer::show(ui, &self.locales, &state, &mut self.sidebar_open);
        if was_open != self.sidebar_open {
            self.persist_settings();
        }
        for event in events {
            self.handle_sidebar_event(event);
        }
    }

    fn handle_sidebar_event(&mut self, event: ui::SidebarEvent) {
        use ui::SidebarEvent as E;
        match event {
            E::SelectFilter(filter) => {
                self.close_document();
                self.close_graph_view();
                self.doc_filter = filter;
                self.selected.clear();
                self.selection_mode = false;
            }
            E::OpenFile(path) => self.open_file_by_path(path),
            E::ImportPdf => self.import_pdf_dialog(),
            E::ManageLabels => self.show_label_manager = true,
            E::OpenVaultPicker => self.pick_and_open_vault(),
            E::CreateVault => self.create_vault(),
            E::OpenRecentVault(path) => self.open_vault_at(path),
            E::RescanVault => {
                self.refresh_pdf_documents();
                self.rescan_and_reindex();
            }
            E::CreateNote { parent_dir } => self.create_note(parent_dir, false),
            E::NewCanvas { parent_dir } => self.create_note(parent_dir, true),
            E::NewSheet { parent_dir } => self.create_sheet(parent_dir),
            E::ImportSheet => self.import_sheet_dialog(),
            E::CreateFolder { parent_dir } => {
                self.prompt_modal = Some(PromptModalState {
                    kind: PromptKind::CreateFolder { parent_dir },
                    title: self.t("folder-new-title"),
                    message: self.t("folder-new-message"),
                    value: String::new(),
                    placeholder: self.t("folder-new-placeholder"),
                    confirm_label: self.t("folder-new-confirm"),
                });
            }
            E::RenameItem {
                path,
                is_dir,
                current_name,
            } => {
                self.prompt_modal = Some(PromptModalState {
                    kind: PromptKind::RenameItem { path, is_dir },
                    title: self.t(if is_dir {
                        "rename-folder-title"
                    } else {
                        "rename-file-title"
                    }),
                    message: self.t_args("rename-message", &[("name", &current_name)]),
                    value: current_name,
                    placeholder: self.t("rename-placeholder"),
                    confirm_label: self.t("rename-confirm"),
                });
            }
            E::MoveItemPrompt { path, is_dir, name } => {
                self.move_modal = Some(MoveModalState {
                    src_path: path,
                    item_name: name,
                    is_dir,
                    search_filter: String::new(),
                });
            }
            E::DeleteItem { path, is_dir, name } => self.trash_path(&path, is_dir, &name),
            E::DirectMove { src_path, dest_dir } => self.move_item_to_folder(src_path, dest_dir),
            E::ToggleFolder(path) => {
                if !self.expanded_folders.remove(&path) {
                    self.expanded_folders.insert(path);
                }
            }
            E::ExpandAllFolders => {
                if let (Some(tree), Some(vault)) = (&self.derived.file_tree, &self.vault) {
                    let mut dirs = Vec::new();
                    tree.collect_directories(&vault.root, &mut dirs);
                    self.expanded_folders
                        .extend(dirs.into_iter().map(|(d, _)| d));
                }
            }
            E::CollapseAllFolders => self.expanded_folders.clear(),
        }
    }

    fn show_chat_panel(&mut self, ui: &mut egui::Ui) {
        let busy = self.chat_pending_embed_id.is_some() || self.chat_pending_gen_id.is_some();
        let mut state = ui::ChatSidebarState {
            messages: &self.chat_messages,
            busy,
            input_text: &mut self.chat_input,
        };
        let Some(event) = ui::ChatSidebarDrawer::show(ui, &self.locales, &mut state) else {
            return;
        };
        match event {
            ui::ChatSidebarEvent::SendMessage(text) => self.send_chat_message(text),
            ui::ChatSidebarEvent::ClearHistory => self.chat_messages.clear(),
            ui::ChatSidebarEvent::Close => self.chat_sidebar_open = false,
            ui::ChatSidebarEvent::OpenCitation(citation) => {
                let page = citation.page_index.map(|p| p + 1);
                self.open_chunk_source(citation.file_path, page);
            }
        }
    }

    fn show_modals(&mut self, ctx: &egui::Context) {
        let cancel = self.t("confirm-cancel");

        if self.conflict_modal {
            let name = self
                .editor
                .as_ref()
                .and_then(|e| e.note.path.file_name().map(|n| n.to_string_lossy().to_string()))
                .unwrap_or_default();
            if let Some(choice) = ui::ConflictModal::show(ctx, &self.locales, &name) {
                self.resolve_conflict(choice);
            }
        }

        if let Some(mut modal) = self.prompt_modal.take() {
            match ui::PromptInputModal::show(
                ctx,
                &modal.title,
                &modal.message,
                &mut modal.value,
                &modal.placeholder,
                &modal.confirm_label,
                &cancel,
            ) {
                Some(true) => {
                    let value = modal.value.trim().to_string();
                    match modal.kind {
                        PromptKind::CreateFolder { parent_dir } => {
                            let new_folder = parent_dir.join(&value);
                            if new_folder.exists() {
                                self.toast(
                                    ToastKind::Error,
                                    "toast-name-taken",
                                    &[("name", &value)],
                                );
                            } else if let Err(e) = std::fs::create_dir_all(&new_folder) {
                                self.report_error("error-context-create-folder", e);
                            } else {
                                self.expanded_folders.insert(parent_dir);
                                self.expanded_folders.insert(new_folder);
                                self.rescan_and_reindex();
                            }
                        }
                        PromptKind::RenameItem { path, is_dir } => {
                            self.rename_item(path, is_dir, value)
                        }
                    }
                }
                Some(false) => {}
                None => self.prompt_modal = Some(modal),
            }
        }

        if let Some(mut modal) = self.move_modal.take() {
            let mut folders = Vec::new();
            if let (Some(tree), Some(vault)) = (&self.derived.file_tree, &self.vault) {
                tree.collect_directories(&vault.root, &mut folders);
            }
            if modal.is_dir {
                folders.retain(|(p, _)| !p.starts_with(&modal.src_path));
            }
            match ui::MoveFolderModal::show(
                ctx,
                &self.locales,
                &modal.item_name,
                &folders,
                &mut modal.search_filter,
            ) {
                Some(ui::MoveChoice::Root) => {
                    if let Some(root) = self.vault.as_ref().map(|v| v.root.clone()) {
                        self.move_item_to_folder(modal.src_path, root);
                    }
                }
                Some(ui::MoveChoice::Folder(dest)) => {
                    self.move_item_to_folder(modal.src_path, dest)
                }
                Some(ui::MoveChoice::Cancel) => {}
                None => self.move_modal = Some(modal),
            }
        }

        if let Some(id) = self.confirm_delete {
            let (title, body, yes) = (
                self.t("confirm-delete-title"),
                self.t("confirm-delete-body"),
                self.t("confirm-yes"),
            );
            if let Some(confirmed) = ui::ConfirmModal::show(ctx, &title, &body, &yes, &cancel, true)
            {
                self.confirm_delete = None;
                if confirmed {
                    self.delete_permanently(&[id]);
                }
            }
        }

        if self.confirm_empty_trash {
            let (title, body, yes) = (
                self.t("confirm-empty-trash-title"),
                self.t("confirm-empty-trash-body"),
                self.t("confirm-empty-trash-yes"),
            );
            if let Some(confirmed) = ui::ConfirmModal::show(ctx, &title, &body, &yes, &cancel, true)
            {
                self.confirm_empty_trash = false;
                if confirmed {
                    let ids: Vec<Uuid> = self
                        .vault
                        .as_ref()
                        .map(|v| {
                            v.notes
                                .iter()
                                .filter(|n| n.frontmatter.trashed)
                                .map(|n| n.frontmatter.id)
                                .collect()
                        })
                        .unwrap_or_default();
                    self.delete_permanently(&ids);
                }
            }
        }

        if self.show_label_manager {
            let mut draft = self
                .tag_rename
                .as_ref()
                .map(|(_, d)| d.clone())
                .unwrap_or_default();
            let mut target = self.tag_rename.as_ref().map(|(t, _)| t.clone());
            let event = ui::LabelManagerModal::show(
                ctx,
                &self.locales,
                &self.derived.tags,
                &mut draft,
                &mut target,
            );
            self.tag_rename = target.map(|t| (t, draft));
            match event {
                Some(ui::LabelManagerEvent::Rename { old_tag, new_tag }) => {
                    self.rename_tag(&old_tag, &new_tag)
                }
                Some(ui::LabelManagerEvent::Delete(tag)) => self.delete_tag(&tag),
                Some(ui::LabelManagerEvent::Close) => {
                    self.show_label_manager = false;
                    self.tag_rename = None;
                }
                None => {}
            }
        }

        if self.show_shortcuts {
            let chords: Vec<(String, &str)> = hotkeys::ACTION_LABELS
                .iter()
                .map(|(action, label)| (self.hotkey_label(action), *label))
                .collect();
            if ui::ShortcutsModal::show(ctx, &self.locales, &chords) {
                self.show_shortcuts = false;
            }
        }
    }

    fn delete_permanently(&mut self, ids: &[Uuid]) {
        let notes: Vec<Note> = self
            .vault
            .as_ref()
            .map(|v| {
                v.notes
                    .iter()
                    .filter(|n| ids.contains(&n.frontmatter.id))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let mut deleted = 0;
        for note in notes {
            let id = note.frontmatter.id;
            if let Err(e) = note.delete_permanently() {
                self.report_error("error-context-delete-note", e);
                continue;
            }
            deleted += 1;
            // Purge cached chunks so stale text can't surface in search (§5).
            if let Some(index) = self.index.as_ref()
                && let Err(e) = index.delete_chunks_for_doc(id)
            {
                log::warn!("app: failed to purge chunks for deleted note: {e}");
            }
        }
        self.rescan_and_reindex();
        if deleted > 0 {
            self.toast(
                ToastKind::Info,
                "toast-deleted-permanently",
                &[("count", &deleted.to_string())],
            );
        }
    }

    fn rename_tag(&mut self, old: &str, new: &str) {
        self.save_editor_now();
        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        let mut save_error = None;
        for i in tags::rename_tag(&mut vault.notes, old, new) {
            if let Err(e) = vault.notes[i].save() {
                save_error = Some(e);
            }
        }
        if let Some(e) = save_error {
            self.report_error("error-context-save-note", e);
        }
        if matches!(&self.doc_filter, SidebarDocFilter::Tag(t) if t.eq_ignore_ascii_case(old)) {
            self.doc_filter = SidebarDocFilter::Tag(new.to_string());
        }
        self.rescan_and_reindex();
    }

    fn delete_tag(&mut self, tag: &str) {
        self.save_editor_now();
        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        let mut save_error = None;
        for i in tags::remove_tag(&mut vault.notes, tag) {
            if let Err(e) = vault.notes[i].save() {
                save_error = Some(e);
            }
        }
        if let Some(e) = save_error {
            self.report_error("error-context-save-note", e);
        }
        if matches!(&self.doc_filter, SidebarDocFilter::Tag(t) if t.eq_ignore_ascii_case(tag)) {
            self.doc_filter = SidebarDocFilter::All;
        }
        self.rescan_and_reindex();
    }
}

impl eframe::App for MnemonicApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.applied_theme != Some(self.theme_mode) {
            theme::apply_theme(&ctx, self.theme_mode);
            self.applied_theme = Some(self.theme_mode);
        }

        // Never lose edits on quit: flush the open note before the window
        // closes (works regardless of which eframe backend is compiled in).
        if ctx.input(|i| i.viewport().close_requested())
            && !(self.save_editor_now() & self.save_sheet_now())
        {
            if self.close_blocked_once {
                log::error!("app: closing with unsaved changes after a failed save");
            } else {
                self.close_blocked_once = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.toast(ToastKind::Error, "toast-close-unsaved", &[]);
            }
        }

        self.poll_background(&ctx);
        self.handle_shortcuts(&ctx);

        self.show_top_bar(ui);
        if self.vault.is_some() {
            self.show_sidebar(ui);
            if self.chat_sidebar_open {
                self.show_chat_panel(ui);
            }
        }

        egui::CentralPanel::default()
            .frame(theme::content_frame())
            .show(ui, |ui| {
                if self.vault.is_none() {
                    self.show_welcome(ui);
                } else if self.editor.is_some() {
                    self.show_editor(ui);
                } else if self.sheet_viewer.is_some() {
                    self.show_sheet_viewer(ui);
                } else if self.pdf_viewer.is_some() {
                    self.show_pdf_viewer(ui);
                } else if self.graph.is_some() {
                    self.show_graph(ui);
                } else {
                    self.show_grid(ui);
                }
            });

        self.show_modals(&ctx);
        self.show_command_palette(&ctx);
        if let Some(action) = self.toasts.show(&ctx) {
            self.apply_toast_action(action);
        }
    }
}

/// Finds the `locales/` directory: next to the executable (and the macOS
/// bundle's `Resources`), then the working directory. If none exists,
/// `LocaleManager` falls back to the locales embedded in the binary.
fn locales_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        for candidate in [dir.join("locales"), dir.join("../Resources/locales")] {
            if candidate.is_dir() {
                return candidate;
            }
        }
    }
    PathBuf::from("locales")
}

/// The diagram of a canvas note for its card: the `.canvas` sidecar when
/// there is one (bound text resolved from the body), else the legacy
/// in-body formats.
fn load_canvas_for_preview(note: &Note) -> CanvasDocument {
    if note.has_sidecar
        && let Ok(json) = std::fs::read_to_string(note.sidecar_path())
        && let Ok(doc) = CanvasDocument::from_json_canvas_str(
            &json,
            &note.frontmatter.title,
            None,
            &|b| crate::markdown::blocks::block_text(&note.body, &b.block_id),
        )
    {
        return doc;
    }
    CanvasDocument::from_markdown_body(&note.frontmatter.title, &note.body)
}

/// Localized "3 sticky notes · 2 shapes" line for canvas cards.
fn canvas_summary(tr: &LocaleManager, doc: &CanvasDocument) -> String {
    use crate::canvas::CanvasElement;
    let (mut notes, mut shapes, mut connectors, mut strokes) = (0, 0, 0, 0);
    for elem in &doc.elements {
        match elem {
            CanvasElement::StickyNote { .. } => notes += 1,
            CanvasElement::Shape { .. } => shapes += 1,
            CanvasElement::Connector { .. } => connectors += 1,
            CanvasElement::FreehandStroke { .. } => strokes += 1,
            _ => {}
        }
    }
    let parts: Vec<String> = [
        ("canvas-count-sticky", notes),
        ("canvas-count-shapes", shapes),
        ("canvas-count-connectors", connectors),
        ("canvas-count-strokes", strokes),
    ]
    .into_iter()
    .filter(|(_, n)| *n > 0)
    .map(|(key, n)| tr.t(key, &[("count", &n.to_string())]))
    .collect();
    if parts.is_empty() {
        tr.t("canvas-count-empty", &[])
    } else {
        parts.join(" · ")
    }
}
