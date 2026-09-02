//! Top-level `eframe::App` state & layout. Fase 1 shipped a minimal vault
//! picker + flat note list; Fase 2 (§3.2, §5) adds the note editor: Source
//! / Live Preview / Reading modes, interactive checklists, wikilinks with
//! `[[` autocomplete, a backlinks panel, and a heading outline. Richer
//! Keep-style grid UI (colors, pinning, drag-reorder) lands in Fase 3.
//! Fase 7 (§5) adds a top-bar tab switch between the Notes grid, a
//! Search tab (`core::search`'s combined keyword+semantic ranking), and
//! a Chat tab (RAG over the vault via `llm::build_rag_prompt` +
//! `llm::GenerationWorker`) — this is also where `core::IndexingWorker`
//! and `llm::GenerationWorker` get spawned and wired in for the first
//! time. Callers: `main.rs`.

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{Context, Result};
use egui_commonmark::CommonMarkCache;
use uuid::Uuid;

use crate::canvas::{
    self, CanvasDocument, CanvasElement, CanvasElementId, CanvasTool, InteractionState,
};
use crate::core::ingestion::pdf_doc_id;
use crate::core::search::{self, SearchHit};
use crate::core::{DocumentChunk, IndexStore, IndexingWorker};
use crate::i18n::LocaleManager;
use crate::llm::{self, GenerationEvent, GenerationWorker};
use crate::markdown::editor::{
    char_index_to_byte_offset, slash_menu_triggered, slash_templates, wikilink_autocomplete_query,
};
use crate::markdown::{EditorMode, MarkdownEditor, WikilinkIndex, wikilink};
use crate::notes::query::{self, GridFilter, SortMode};
use crate::notes::{Note, Vault, VaultWatcher, tags, trash};
use crate::pdf::annotator::{Annotation, AnnotationKind};
use crate::pdf::editor::DocumentMetadata;
use crate::pdf::{self, PdfRenderer, annotator as pdf_annotator, editor as pdf_editor};
use crate::ui::{self, theme};

/// How many top-ranked chunks to retrieve for the Search tab / Chat tab
/// respectively (§Fase 7). Search shows more candidates than chat's RAG
/// context window since a human is skimming results, while chat feeds
/// straight into a token-bounded LLM prompt.
const SEARCH_TOP_K: usize = 10;
const CHAT_TOP_K: usize = 5;

/// Default rendered page width (§Fase 8), in pixels — wide enough to read
/// comfortably, small enough that re-rendering on zoom/page changes stays
/// snappy on the UI thread (see `pdf::renderer::PdfRenderer`'s doc comment
/// on why rendering is synchronous rather than backgrounded in this
/// phase).
const DEFAULT_PDF_ZOOM_WIDTH: u16 = 900;

/// Default annotation color (§Fase 9) — a translucent-when-drawn yellow,
/// the conventional highlighter color, in the `(r, g, b)` `0.0..=1.0`
/// form `pdf::annotator::Annotation::color` expects.
const DEFAULT_ANNOTATION_COLOR: [f32; 3] = [1.0, 0.92, 0.23];

/// Minimum on-screen drag distance (pixels) before a drag over the PDF
/// canvas counts as "the user drew a rectangle" rather than "the user
/// clicked" (§Fase 9). A click still places an annotation — just at a
/// small default size anchored at the click point — so a single tap
/// works for sticky notes without forcing a drag every time.
const MIN_ANNOTATION_DRAG_PX: f32 = 4.0;

/// Which top-level tab is showing in the central panel (only relevant
/// while no note is open for editing and no PDF is open for viewing —
/// `show_editor`/`show_pdf_viewer` always take over regardless of `view`,
/// same precedence Fase 7 established for the note editor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Notes,  // unified: Markdown notes + PDFs in one masonry grid
    Canvas, // Standalone AFFiNE Edgeless Whiteboard Canvas
    Search,
    Chat,
}

/// Filter for the unified document grid — controls which document types
/// and states are shown. Extends `GridFilter` to also handle PDF-only.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DocFilter {
    All,
    NotesOnly,
    PdfsOnly,
    WhiteboardsOnly,
    Archived,
    Trashed,
    Tag(String),
}

/// One clickable reference an assistant reply grounded its answer on
/// (§3.4 point 4) — carries enough to jump straight to the source instead
/// of just naming it, added in Fase 8 once the PDF viewer existed to jump
/// to.
#[derive(Clone)]
struct Citation {
    file_path: PathBuf,
    /// `None` for a note citation; `Some(0-based page)` for a PDF one.
    page_index: Option<usize>,
    label: String,
}

/// One rendered bubble in the Chat tab's transcript. `Clone` lets
/// `show_chat` snapshot the transcript into a plain local before entering
/// nested `egui` closures — same reasoning as `show_grid`'s
/// `let notes: Vec<Note> = ...clone()`.
#[derive(Clone)]
struct ChatMessage {
    role: ChatRole,
    text: String,
    /// Source citations the assistant grounded its reply on — empty for
    /// user messages and for a reply that found no relevant context.
    citations: Vec<Citation>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChatRole {
    User,
    Assistant,
}

impl ChatMessage {
    fn user(text: String) -> ChatMessage {
        ChatMessage {
            role: ChatRole::User,
            text,
            citations: Vec::new(),
        }
    }

    fn assistant(text: String, citations: Vec<Citation>) -> ChatMessage {
        ChatMessage {
            role: ChatRole::Assistant,
            text,
            citations,
        }
    }
}

/// Lazily-constructed `PdfRenderer` state (§Fase 8), mirroring
/// `core::indexer`'s `EmbedderState`: binding to the native PDFium library
/// only happens on first use (never blocking `MnemonicApp::new()`), and a
/// failure is remembered so a missing library doesn't retry the same slow
/// failure on every frame the PDF viewer is open.
enum PdfRendererState {
    Uninit,
    Ready(PdfRenderer),
    Failed(String),
}

/// State for the currently-open PDF (§Fase 8) — mutually exclusive with
/// `MnemonicApp::editor`, same as the note editor takes over the central
/// panel regardless of `view`.
struct PdfViewerState {
    path: PathBuf,
    page_count: usize,
    /// 0-based.
    page_index: usize,
    zoom_width: u16,
    texture: Option<egui::TextureHandle>,
    /// `(page_index, zoom_width)` the current `texture` was rendered for —
    /// re-rendered whenever this no longer matches the live values.
    rendered_key: Option<(usize, u16)>,
    render_error: Option<String>,
    /// The current page's true size in PDF points (§Fase 9), cached
    /// alongside `texture`/`rendered_key` so it's only recomputed when the
    /// page actually changes, not every frame — used to convert the
    /// annotation canvas's on-screen drag rectangles into
    /// `Annotation::rect`'s PDF user-space coordinates.
    page_size_points: Option<(f32, f32)>,
    /// 1-based inclusive page range for the Split operation's UI.
    split_from: u32,
    split_to: u32,
    delete_confirm: bool,
    op_status: String,

    // Annotation canvas (§Fase 9, §3.5 point 2). `staged_annotations`
    // haven't been written to any file yet — they only get baked in when
    // the user hits Save/Export (`bake_pdf_changes`); until then they're
    // just drawn as an overlay on top of the rendered bitmap.
    annotate_tool: Option<AnnotationKind>,
    annotate_color: [f32; 3],
    drag_start: Option<egui::Pos2>,
    staged_annotations: Vec<Annotation>,
    /// A placed-but-not-yet-confirmed sticky note / text injection,
    /// waiting on the text prompt window for its `contents`.
    pending_annotation: Option<PendingAnnotation>,
    pending_annotation_text: String,

    // Metadata editor (§Fase 9, §3.5 point 2's "Metadata Editor" bullet).
    // Loaded from `input`'s current `/Info` dict on open so the fields
    // always start accurate, then carried along unconditionally into
    // every Save/Export (a harmless no-op rewrite when unedited).
    show_metadata_editor: bool,
    metadata_title: String,
    metadata_author: String,
    metadata_keywords: String,

    // Save/Export (§Fase 9, §3.5 point 3).
    show_save_confirm: bool,
}

/// One placed-but-unconfirmed sticky note / text injection (§Fase 9),
/// waiting on the user's text before it's added to `staged_annotations`.
/// `rect` is already in PDF user-space points (`Annotation::rect`'s
/// coordinate system), computed at drag-release time.
struct PendingAnnotation {
    kind: AnnotationKind,
    page: u32,
    rect: (f32, f32, f32, f32),
}

/// What the user asked for while looking at the PDF viewer's page-ops
/// toolbar. `app.rs::apply_pdf_viewer_action` applies these (each one
/// writes to a new output file, per `pdf::editor`'s never-in-place design)
/// after `show_pdf_viewer`'s `egui` closures have all returned — same
/// deferred-action pattern as `GridAction`.
enum PdfViewerAction {
    RotateCurrentPage {
        path: PathBuf,
        page: u32,
        degrees: i64,
    },
    DeleteCurrentPage {
        path: PathBuf,
        page: u32,
    },
    Split {
        path: PathBuf,
        from: u32,
        to: u32,
    },
    Merge {
        path: PathBuf,
    },
    /// "Save" (§3.5 point 3, overwrite half): bakes `annotations` +
    /// `metadata` into `path` in place, auto-backed-up via
    /// `pdf::editor::save_over`.
    SaveOver {
        path: PathBuf,
        annotations: Vec<Annotation>,
        metadata: DocumentMetadata,
    },
    /// "Export" (§3.5 point 3, save-as half): bakes the same changes into
    /// a user-chosen new file, `path` itself left untouched.
    ExportAs {
        path: PathBuf,
        annotations: Vec<Annotation>,
        metadata: DocumentMetadata,
    },
}

/// What the user asked for while looking at the PDF library tab
/// (§Fase 8). Applied after `show_pdf_library`'s `egui` closures have all
/// returned, same pattern as `GridAction`/`PdfViewerAction`.
enum PdfLibraryAction {
    Import(PathBuf),
    Open(PathBuf),
    Remove(PathBuf),
}

pub struct MnemonicApp {
    locales: LocaleManager,
    vault: Option<Vault>,
    watcher: Option<VaultWatcher>,
    index: Option<IndexStore>,
    indexer: Option<IndexingWorker>,
    generator: Option<GenerationWorker>,
    quick_capture_text: String,
    status: String,
    editor: Option<MarkdownEditor>,
    markdown_cache: CommonMarkCache,
    grid_filter: GridFilter,
    doc_filter: DocFilter,
    sort_mode: SortMode,
    search_text: String,
    selection_mode: bool,
    selected: HashSet<Uuid>,
    show_label_manager: bool,
    tag_rename: Option<(String, String)>,
    confirm_delete: Option<Uuid>,

    /// Whether the floating sidebar overlay is currently open.
    sidebar_open: bool,

    view: View,

    // Search tab (§Fase 7).
    search_query: String,
    search_pending_id: Option<Uuid>,
    search_results: Vec<SearchHit>,
    search_status: String,

    // Chat tab (§Fase 7).
    chat_input: String,
    chat_messages: Vec<ChatMessage>,
    chat_pending_embed_id: Option<Uuid>,
    chat_pending_gen_id: Option<Uuid>,

    // PDF viewer & operations (§Fase 8).
    pdf_renderer: PdfRendererState,
    pdf_documents: Vec<PathBuf>,
    pdf_viewer: Option<PdfViewerState>,

    // AFFiNE Command Palette (⌘K) & Standalone Whiteboard Canvas.
    show_command_palette: bool,
    command_palette_query: String,
    theme_mode: ui::ThemeMode,
    command_palette: ui::CommandPalette,
    standalone_canvas: Option<CanvasDocument>,
    standalone_canvas_interaction: InteractionState,
}

impl MnemonicApp {
    pub fn new() -> MnemonicApp {
        let locales_dir = locales_dir();
        let locales = LocaleManager::load(&locales_dir);

        let mut app = MnemonicApp {
            locales,
            vault: None,
            watcher: None,
            index: None,
            // Spawning is cheap (just a background thread); the actual
            // FastEmbed/Candle model only loads lazily on the first
            // submitted job, so this never blocks startup (§6 risk 2).
            indexer: Some(IndexingWorker::spawn()),
            generator: Some(GenerationWorker::spawn()),
            quick_capture_text: String::new(),
            status: String::new(),
            editor: None,
            markdown_cache: CommonMarkCache::default(),
            grid_filter: GridFilter::All,
            doc_filter: DocFilter::All,
            sort_mode: SortMode::Modified,
            search_text: String::new(),
            selection_mode: false,
            selected: HashSet::new(),
            show_label_manager: false,
            tag_rename: None,
            confirm_delete: None,
            sidebar_open: false,
            view: View::Notes,
            search_query: String::new(),
            search_pending_id: None,
            search_results: Vec::new(),
            search_status: String::new(),
            chat_input: String::new(),
            chat_messages: Vec::new(),
            chat_pending_embed_id: None,
            chat_pending_gen_id: None,
            pdf_renderer: PdfRendererState::Uninit,
            pdf_documents: Vec::new(),
            pdf_viewer: None,
            show_command_palette: false,
            command_palette_query: String::new(),
            theme_mode: ui::ThemeMode::Dark,
            command_palette: ui::CommandPalette::default(),
            standalone_canvas: None,
            standalone_canvas_interaction: InteractionState::new(),
        };

        if let Some(result) = Vault::load_last() {
            match result {
                Ok(vault) => app.activate_vault(vault),
                Err(e) => log::warn!("app: failed to reopen last vault: {e}"),
            }
        }

        app
    }

    fn activate_vault(&mut self, vault: Vault) {
        // Purge expired trash on open, per §3.1.4.
        if let Err(e) = trash::purge_expired(&vault.root, trash::DEFAULT_RETENTION) {
            log::warn!("app: trash purge failed: {e}");
        }

        match IndexStore::open(&vault.root) {
            Ok(mut index) => {
                if let Err(e) = index.rebuild(&vault.notes) {
                    log::warn!("app: index rebuild failed: {e}");
                }
                self.index = Some(index);
            }
            Err(e) => log::warn!("app: failed to open index store: {e}"),
        }
        self.refresh_pdf_documents();

        // Fase 7: populate the document_chunks cache (search/RAG
        // retrieval) for every note in the vault. Cheap to do in full
        // here since this only runs once per vault open; per-note
        // resubmission on later edits happens at the specific mutation
        // sites instead (see `reindex_note`), not by resubmitting the
        // whole vault again.
        if let Some(indexer) = &self.indexer {
            for note in &vault.notes {
                indexer.submit_note(note.clone());
            }
        }

        self.watcher = match VaultWatcher::watch(&vault.root) {
            Ok(w) => Some(w),
            Err(e) => {
                log::warn!("app: failed to start file watcher: {e}");
                None
            }
        };

        self.vault = Some(vault);
    }

    fn rescan_and_reindex(&mut self) {
        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        if let Err(e) = vault.rescan() {
            log::warn!("app: vault rescan failed: {e}");
            return;
        }
        if let Some(index) = self.index.as_mut() {
            if let Err(e) = index.rebuild(&vault.notes) {
                log::warn!("app: index rebuild after rescan failed: {e}");
            }
        }
    }

    /// Submits `note` for background re-chunking + re-embedding (§Fase 7)
    /// so an edited note's `document_chunks` stay in sync with its saved
    /// content. Non-blocking — the result lands later in
    /// `poll_indexer_results`. Called from the specific mutation sites
    /// (editor autosave/close, single-field grid actions) rather than
    /// from `rescan_and_reindex`, so a full vault re-embed isn't
    /// triggered by every autosave tick.
    fn reindex_note(&self, note: &Note) {
        if let Some(indexer) = &self.indexer {
            indexer.submit_note(note.clone());
        }
    }

    /// Drains chunk/embedding results produced by the background
    /// `IndexingWorker` and writes them into the SQLite cache. Call once
    /// per frame (same shape as the watcher/autosave polls above).
    fn poll_indexer_results(&mut self) {
        let Some(indexer) = &self.indexer else { return };
        let results = indexer.poll_results();
        if results.is_empty() {
            return;
        }
        let Some(index) = self.index.as_mut() else {
            return;
        };
        for result in results {
            match result {
                Ok(r) => {
                    if let Err(e) = index.replace_chunks(r.doc_id, r.doc_type.as_str(), &r.chunks) {
                        log::warn!("app: failed to store indexed chunks: {e}");
                    }
                }
                Err(e) => log::warn!("app: background indexing failed: {e}"),
            }
        }
    }

    /// Drains query-embedding results (Search tab / Chat tab) and
    /// generation events (Chat tab) from their respective background
    /// workers, dispatching each to whichever request is currently
    /// pending. Call once per frame.
    fn poll_search_and_chat(&mut self, ctx: &egui::Context) {
        if let Some(indexer) = &self.indexer {
            for (id, result) in indexer.poll_query_results() {
                if self.search_pending_id == Some(id) {
                    self.search_pending_id = None;
                    match result {
                        Ok(embedding) => self.apply_semantic_search(&embedding),
                        Err(e) => self.search_status = format!("{}: {e:#}", self.t("search-error")),
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
    }

    /// Combines the just-resolved query embedding's semantic hits with
    /// the keyword hits already computed by `run_search`, replacing
    /// `search_results` with the merged, de-duplicated list.
    fn apply_semantic_search(&mut self, embedding: &[f32]) {
        let Some(index) = self.index.as_ref() else {
            return;
        };
        let chunks = match index.all_chunks() {
            Ok(c) => c,
            Err(e) => {
                self.search_status = format!("{}: {e:#}", self.t("search-error"));
                return;
            }
        };
        let semantic =
            search::semantic_search(embedding, &chunks, SEARCH_TOP_K, llm::SIMILARITY_THRESHOLD);
        let notes: Vec<Note> = self
            .vault
            .as_ref()
            .map(|v| v.notes.clone())
            .unwrap_or_default();
        let keyword_notes = search::keyword_search(&notes, &self.search_query);
        self.search_results = search::merge_results(semantic, &keyword_notes);
    }

    /// Runs the keyword half of search immediately (so the tab never sits
    /// empty) and kicks off background query embedding for the semantic
    /// half — `apply_semantic_search` (via `poll_search_and_chat`) merges
    /// it in once the embedding resolves.
    fn run_search(&mut self) {
        let query_text = self.search_query.trim().to_string();
        self.search_status.clear();
        if query_text.is_empty() {
            self.search_results.clear();
            self.search_pending_id = None;
            return;
        }

        let notes: Vec<Note> = self
            .vault
            .as_ref()
            .map(|v| v.notes.clone())
            .unwrap_or_default();
        let keyword_notes = search::keyword_search(&notes, &query_text);
        self.search_results = search::merge_results(Vec::new(), &keyword_notes);

        if let Some(indexer) = &self.indexer {
            self.search_pending_id = Some(indexer.submit_query(query_text));
        }
    }

    /// Pushes the user's chat message and kicks off background query
    /// embedding — `start_chat_generation` (via `poll_search_and_chat`)
    /// takes over once the embedding resolves.
    fn send_chat_message(&mut self) {
        let text = self.chat_input.trim().to_string();
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

    /// Retrieves top-scoring context chunks for the just-resolved query
    /// embedding, builds the strict-grounding RAG prompt (§3.4 point 2),
    /// appends an empty assistant bubble tagged with its citations, and
    /// submits the prompt to the background `GenerationWorker` — its
    /// streamed tokens get appended to that bubble by
    /// `poll_search_and_chat`.
    fn start_chat_generation(&mut self, embedding: &[f32]) {
        let question = self
            .chat_messages
            .last()
            .map(|m| m.text.clone())
            .unwrap_or_default();

        let context = match self.index.as_ref().map(|i| i.all_chunks()) {
            Some(Ok(chunks)) => {
                let vectors: Vec<Vec<f32>> = chunks.iter().map(|c| c.embedding.clone()).collect();
                let scored = crate::core::top_k(embedding, &vectors, CHAT_TOP_K);
                let all_chunks: Vec<DocumentChunk> = chunks.into_iter().map(|c| c.chunk).collect();
                llm::select_context(&scored, &all_chunks, llm::SIMILARITY_THRESHOLD)
            }
            Some(Err(e)) => {
                log::warn!("app: failed to load chunks for chat retrieval: {e}");
                Vec::new()
            }
            None => Vec::new(),
        };
        let citations: Vec<Citation> = context
            .iter()
            .map(|c| Citation {
                file_path: c.file_path.clone(),
                page_index: c.page_num.map(|p| p.saturating_sub(1)),
                label: match c.page_num {
                    Some(p) => format!("{} (Halaman {p})", c.file_path.display()),
                    None => c.file_path.display().to_string(),
                },
            })
            .collect();
        let prompt = llm::build_rag_prompt(&context, &question);

        self.chat_messages
            .push(ChatMessage::assistant(String::new(), citations));

        if let Some(generator) = &self.generator {
            self.chat_pending_gen_id = Some(generator.submit(prompt, llm::DEFAULT_MAX_TOKENS));
        }
    }

    fn t(&self, key: &str) -> String {
        self.locales.t(key, &[])
    }

    /// Records a user-facing failure (§6 "error handling" polish, Fase 10):
    /// logs it via `log::warn!` *and* sets the status banner
    /// (`show_status_banner`), so a failed save/delete/move is never
    /// silently swallowed — several mutation paths below used to only log,
    /// which meant e.g. a failed autosave gave the user no indication
    /// anything was wrong until they noticed missing edits much later.
    /// `context_key` is an already-translated-lookup Fluent key describing
    /// what was being attempted (e.g. `"error-context-autosave"`).
    fn report_error(&mut self, context_key: &str, err: impl std::fmt::Display) {
        log::warn!("app: {context_key}: {err}");
        let context = self.t(context_key);
        self.status = self.locales.t(
            "error-banner",
            &[("context", &context), ("error", &err.to_string())],
        );
    }

    /// A dismissible red banner for the most recent failure recorded via
    /// `report_error`. Rendered once, above every view (grid/search/chat/
    /// editor/pdf/vault-picker) in `ui()`'s `CentralPanel`, so a failure
    /// while e.g. the editor is open stays visible even after navigating
    /// away from it — previously this only rendered inside `show_grid`,
    /// invisible from every other view (§Fase 10).
    fn show_status_banner(&mut self, ui: &mut egui::Ui) {
        if self.status.is_empty() {
            return;
        }
        ui.horizontal(|ui| {
            ui.colored_label(egui::Color32::RED, &self.status);
            if ui.small_button("×").clicked() {
                self.status.clear();
            }
        });
        ui.separator();
    }

    /// Open a note for editing. Only called from the note-list view, which
    /// is itself only shown while `self.editor` is `None`, so there's
    /// nothing to flush here.
    fn open_note(&mut self, note: Note) {
        self.editor = Some(MarkdownEditor::open(note));
    }

    /// Resolve a clicked `[[wikilink]]` to an existing note, or — per
    /// §3.2.2 — auto-create a new (empty) note with that title if none
    /// exists yet, then open it for editing.
    fn navigate_wikilink(&mut self, title: &str) {
        let existing = self.vault.as_ref().and_then(|vault| {
            vault
                .notes
                .iter()
                .find(|n| !n.frontmatter.trashed && n.frontmatter.title.eq_ignore_ascii_case(title))
                .cloned()
        });

        if let Some(note) = existing {
            self.editor = Some(MarkdownEditor::open(note));
            return;
        }

        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        match Note::create(&vault.root, title, "") {
            Ok(note) => {
                self.editor = Some(MarkdownEditor::open(note));
                self.rescan_and_reindex();
            }
            Err(e) => self.report_error("error-context-create-note", e),
        }
    }

    /// Lazily binds the native PDFium library on first use, remembering a
    /// failure so a missing library doesn't retry the same slow lookup on
    /// every frame the PDF viewer is open (§Fase 8, mirrors
    /// `core::indexer`'s `EmbedderState`).
    fn ensure_pdf_renderer(&mut self) -> Result<&PdfRenderer, &str> {
        if matches!(self.pdf_renderer, PdfRendererState::Uninit) {
            self.pdf_renderer = match PdfRenderer::new() {
                Ok(renderer) => PdfRendererState::Ready(renderer),
                Err(e) => PdfRendererState::Failed(format!("{e:#}")),
            };
        }
        match &self.pdf_renderer {
            PdfRendererState::Ready(renderer) => Ok(renderer),
            PdfRendererState::Failed(msg) => Err(msg.as_str()),
            PdfRendererState::Uninit => unreachable!("resolved just above"),
        }
    }

    /// Reloads the PDF library tab's list from the index store (§Fase 8).
    fn refresh_pdf_documents(&mut self) {
        self.pdf_documents = self
            .index
            .as_ref()
            .and_then(|index| index.list_pdf_documents().ok())
            .unwrap_or_default();
    }

    /// Registers `path` as an imported PDF, submits it for background
    /// chunking + embedding (finally giving `IndexingWorker::submit_pdf` a
    /// caller — §5/§Fase 7 left it unused until this phase), and opens it
    /// in the viewer.
    fn import_pdf(&mut self, path: PathBuf) {
        if let Some(index) = self.index.as_ref()
            && let Err(e) = index.add_pdf_document(&path)
        {
            log::warn!("app: failed to register imported pdf: {e}");
        }
        if let Some(indexer) = &self.indexer {
            indexer.submit_pdf(path.clone());
        }
        self.refresh_pdf_documents();
        self.open_pdf(path);
    }

    /// Opens `path` in the PDF viewer at its first page, closing the note
    /// editor if one was open (mutually exclusive, same as opening a note
    /// closes the PDF viewer). Page count comes from the PDFium renderer
    /// when available, falling back to the text extractor (already used
    /// for search ingestion) so the viewer still has a usable page count
    /// even when no PDFium library is installed — only the bitmap itself
    /// is unavailable in that case.
    fn open_pdf(&mut self, path: PathBuf) {
        self.editor = None;

        let page_count = match self.ensure_pdf_renderer() {
            Ok(renderer) => renderer.page_count(&path).ok(),
            Err(_) => None,
        }
        .or_else(|| pdf::extract_pages(&path).ok().map(|pages| pages.len()))
        .unwrap_or(1)
        .max(1);

        // Best-effort: an unreadable/missing /Info dict just means empty
        // fields, not a failure to open the viewer (§Fase 9).
        let metadata = pdf_editor::get_metadata(&path).unwrap_or_default();

        self.pdf_viewer = Some(PdfViewerState {
            path,
            page_count,
            page_index: 0,
            zoom_width: DEFAULT_PDF_ZOOM_WIDTH,
            texture: None,
            rendered_key: None,
            render_error: None,
            page_size_points: None,
            split_from: 1,
            split_to: page_count as u32,
            delete_confirm: false,
            op_status: String::new(),
            annotate_tool: None,
            annotate_color: DEFAULT_ANNOTATION_COLOR,
            drag_start: None,
            staged_annotations: Vec::new(),
            pending_annotation: None,
            pending_annotation_text: String::new(),
            show_metadata_editor: false,
            metadata_title: metadata.title,
            metadata_author: metadata.author,
            metadata_keywords: metadata.keywords,
            show_save_confirm: false,
        });
    }

    /// Opens `path` in the PDF viewer and jumps straight to `page_index`
    /// (0-based) — the "jump-to-source" behavior §3.4 point 4 and §3.5
    /// point 1 both call for, used by search results and chat citations.
    fn open_pdf_at_page(&mut self, path: PathBuf, page_index: usize) {
        self.open_pdf(path);
        if let Some(viewer) = self.pdf_viewer.as_mut() {
            viewer.page_index = page_index.min(viewer.page_count.saturating_sub(1));
        }
    }

    /// Common tail of every PDF page operation (§Fase 8): registers the
    /// newly-written `output` file and opens it, so the user immediately
    /// sees the result instead of having to reopen it manually from the
    /// library tab.
    fn finish_pdf_operation(&mut self, output: PathBuf) {
        self.import_pdf(output);
        let msg = self.t("pdf-op-success");
        if let Some(viewer) = self.pdf_viewer.as_mut() {
            viewer.op_status = msg;
        }
    }

    /// Applies one PDF page operation. The four page-ops (rotate/delete/
    /// split/merge) always write to a user-chosen new output file rather
    /// than overwriting the source (see `pdf::editor`'s doc comment);
    /// `SaveOver`/`ExportAs` (§3.5 point 3, Fase 9) are the two
    /// exceptions — both bake pending annotations + metadata via
    /// `bake_pdf_changes` first, then either overwrite `path` in place
    /// (auto-backed-up) or copy the result to a new file. Called once per
    /// accumulated `PdfViewerAction` after `show_pdf_viewer`'s `egui`
    /// closures have all returned.
    fn apply_pdf_viewer_action(&mut self, action: PdfViewerAction) {
        let t_op_error = self.t("pdf-op-error");
        let result = match &action {
            PdfViewerAction::RotateCurrentPage {
                path,
                page,
                degrees,
            } => rfd::FileDialog::new()
                .add_filter("PDF", &["pdf"])
                .save_file()
                .map(|output| {
                    pdf_editor::rotate(path, &[*page], *degrees, &output).map(|()| output)
                }),
            PdfViewerAction::DeleteCurrentPage { path, page } => rfd::FileDialog::new()
                .add_filter("PDF", &["pdf"])
                .save_file()
                .map(|output| pdf_editor::delete_pages(path, &[*page], &output).map(|()| output)),
            PdfViewerAction::Split { path, from, to } => {
                let pages: Vec<u32> = (*from..=*to).collect();
                rfd::FileDialog::new()
                    .add_filter("PDF", &["pdf"])
                    .save_file()
                    .map(|output| pdf_editor::split(path, &pages, &output).map(|()| output))
            }
            PdfViewerAction::Merge { path } => rfd::FileDialog::new()
                .add_filter("PDF", &["pdf"])
                .pick_file()
                .and_then(|other| {
                    rfd::FileDialog::new()
                        .add_filter("PDF", &["pdf"])
                        .save_file()
                        .map(|output| {
                            pdf_editor::merge(&[path.as_path(), other.as_path()], &output)
                                .map(|()| output)
                        })
                }),
            PdfViewerAction::SaveOver {
                path,
                annotations,
                metadata,
            } => {
                Some(
                    bake_pdf_changes(path, annotations, metadata).and_then(|staged| {
                        let backup = pdf_editor::save_over(path, &staged)?;
                        let _ = std::fs::remove_file(&staged); // best-effort: staged is a temp file
                        Ok(backup)
                    }),
                )
            }
            PdfViewerAction::ExportAs {
                path,
                annotations,
                metadata,
            } => rfd::FileDialog::new()
                .add_filter("PDF", &["pdf"])
                .save_file()
                .map(|target| {
                    bake_pdf_changes(path, annotations, metadata).and_then(|staged| {
                        std::fs::copy(&staged, &target).with_context(|| {
                            format!("copying staged PDF to {}", target.display())
                        })?;
                        let _ = std::fs::remove_file(&staged);
                        Ok(target)
                    })
                }),
        };

        match result {
            Some(Ok(output)) => {
                if matches!(action, PdfViewerAction::SaveOver { .. }) {
                    // Overwritten in place: stay on the same path (unlike
                    // every other op, which opens a brand-new file) —
                    // just clear the now-baked-in staged edits and force
                    // a re-render so they show up.
                    if let Some(viewer) = self.pdf_viewer.as_mut() {
                        viewer.staged_annotations.clear();
                        viewer.rendered_key = None;
                    }
                    let msg = self.locales.t(
                        "pdf-save-success",
                        &[("backup", &output.display().to_string())],
                    );
                    if let Some(viewer) = self.pdf_viewer.as_mut() {
                        viewer.op_status = msg;
                    }
                } else {
                    // `ExportAs` and the four page-ops all end here:
                    // `finish_pdf_operation` opens `output` fresh via
                    // `open_pdf`, which already starts with empty
                    // `staged_annotations` and metadata reloaded from the
                    // new file — nothing left over from the old viewer to
                    // clear.
                    self.finish_pdf_operation(output);
                }
            }
            Some(Err(e)) => {
                if let Some(viewer) = self.pdf_viewer.as_mut() {
                    viewer.op_status = format!("{t_op_error}: {e:#}");
                }
            }
            None => {} // user cancelled the file dialog
        }
    }

    /// Applies one PDF library action. Called once per accumulated
    /// `PdfLibraryAction` after `show_pdf_library`'s `egui` closures have
    /// all returned, same pattern as `apply_grid_action`.
    fn apply_pdf_library_action(&mut self, action: PdfLibraryAction) {
        match action {
            PdfLibraryAction::Import(path) => self.import_pdf(path),
            PdfLibraryAction::Open(path) => self.open_pdf(path),
            PdfLibraryAction::Remove(path) => {
                if let Some(index) = self.index.as_ref() {
                    if let Err(e) = index.remove_pdf_document(&path) {
                        log::warn!("app: failed to remove pdf document: {e}");
                    }
                    if let Err(e) = index.delete_chunks_for_doc(pdf_doc_id(&path)) {
                        log::warn!("app: failed to purge chunks for removed pdf: {e}");
                    }
                }
                self.refresh_pdf_documents();
            }
        }
    }

    /// Renders the open note: mode tabs, outline + backlinks side panel,
    /// and the content area (raw `TextEdit` in Source mode, the
    /// interactive renderer otherwise). Everything the inner `egui`
    /// closures need is resolved into plain local variables up front so
    /// none of them have to borrow `self` — see the comment below.
    fn show_editor(&mut self, ui: &mut egui::Ui) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };

        let t_back = self.t("editor-back");
        let t_source = self.t("editor-mode-source");
        let t_live_preview = self.t("editor-mode-live-preview");
        let t_reading = self.t("editor-mode-reading");
        let t_undo = self.t("editor-undo");
        let t_redo = self.t("editor-redo");
        let t_outline = self.t("editor-outline");
        let t_backlinks = self.t("editor-backlinks");
        let t_backlinks_empty = self.t("editor-backlinks-empty");
        let t_word_count = self.locales.t(
            "editor-word-count",
            &[("count", &editor.word_count().to_string())],
        );
        let t_reading_time = self.locales.t(
            "editor-reading-time",
            &[("minutes", &editor.reading_time_minutes().to_string())],
        );

        let outline = editor.outline();
        let wikilink_index = self.vault.as_ref().map(|v| WikilinkIndex::build(&v.notes));
        let backlink_titles: Vec<String> = self
            .vault
            .as_ref()
            .map(|v| {
                wikilink::backlinks_for(
                    &editor.note.frontmatter.title,
                    editor.note.frontmatter.id,
                    &v.notes,
                )
                .into_iter()
                .map(|n| n.frontmatter.title.clone())
                .collect()
            })
            .unwrap_or_default();
        // Borrowed once up front so the render closures below only ever
        // touch plain locals, never `self` — keeps disjoint-field borrows
        // out of nested `egui` closures entirely.
        let cache = &mut self.markdown_cache;

        let mut close_requested = false;
        let mut navigate_to: Option<String> = None;
        let mut scroll_to_slug: Option<String> = None;

        let is_edgeless = editor.mode == EditorMode::Edgeless;
        let is_dark = ui.visuals().dark_mode;

        ui.horizontal(|ui| {
            if ui.button(t_back.as_str()).clicked() {
                close_requested = true;
            }
            ui.heading(editor.note.frontmatter.title.as_str());

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // AFFiNE Dual-State Switcher: [ 📄 Page | 🎨 Edgeless ]
                let is_page = editor.mode != EditorMode::Edgeless;
                let page_btn = egui::Button::new(egui::RichText::new("📄 Page").size(13.0).color(
                    if is_page {
                        theme::GLASS_ACCENT_HOVER
                    } else {
                        theme::GLASS_TEXT_SECONDARY
                    },
                ))
                .fill(if is_page {
                    theme::GLASS_SURFACE_HIGH
                } else {
                    egui::Color32::TRANSPARENT
                })
                .corner_radius(egui::CornerRadius::same(theme::ROUNDING_SM));

                let edgeless_btn =
                    egui::Button::new(egui::RichText::new("🎨 Edgeless").size(13.0).color(
                        if is_edgeless {
                            theme::GLASS_ACCENT_HOVER
                        } else {
                            theme::GLASS_TEXT_SECONDARY
                        },
                    ))
                    .fill(if is_edgeless {
                        theme::GLASS_SURFACE_HIGH
                    } else {
                        egui::Color32::TRANSPARENT
                    })
                    .corner_radius(egui::CornerRadius::same(theme::ROUNDING_SM));

                if ui.add(edgeless_btn).clicked() {
                    editor.mode = EditorMode::Edgeless;
                    editor.ensure_canvas();
                }
                if ui.add(page_btn).clicked() {
                    editor.sync_canvas_to_body();
                    editor.mode = EditorMode::LivePreview;
                }

                ui.add_space(8.0);

                if !is_edgeless {
                    ui.selectable_value(&mut editor.mode, EditorMode::Reading, t_reading.as_str());
                    ui.selectable_value(
                        &mut editor.mode,
                        EditorMode::LivePreview,
                        t_live_preview.as_str(),
                    );
                    ui.selectable_value(&mut editor.mode, EditorMode::Source, t_source.as_str());
                }

                ui.separator();
                if ui.button(t_redo.as_str()).clicked() {
                    editor.redo();
                }
                if ui.button(t_undo.as_str()).clicked() {
                    editor.undo();
                }
            });
        });

        if !is_edgeless {
            ui.label(format!("{t_word_count} · {t_reading_time}"));
            ui.separator();

            egui::Panel::right("editor_side_panel")
                .resizable(true)
                .default_size(220.0)
                .show(ui, |ui| {
                    ui.heading(t_outline.as_str());
                    for heading in &outline {
                        let indent = "  ".repeat(heading.level.saturating_sub(1) as usize);
                        if ui.link(format!("{indent}{}", heading.title)).clicked() {
                            scroll_to_slug = Some(heading.slug.clone());
                        }
                    }

                    ui.separator();
                    ui.heading(t_backlinks.as_str());
                    if backlink_titles.is_empty() {
                        ui.label(t_backlinks_empty.as_str());
                    } else {
                        for title in &backlink_titles {
                            if ui.link(title.as_str()).clicked() {
                                navigate_to = Some(title.clone());
                            }
                        }
                    }
                });
        }

        egui::CentralPanel::default().show(ui, |ui| {
            if editor.mode == EditorMode::Edgeless {
                editor.ensure_canvas();
                if let Some(canvas) = editor.canvas.as_mut() {
                    if Self::show_canvas_surface(canvas, &mut editor.canvas_interaction, ui, is_dark) {
                        editor.sync_canvas_to_body();
                    }
                }
            } else {
                egui::ScrollArea::vertical().show_viewport(ui, |ui, viewport| match editor.mode {
                    EditorMode::Source => {
                        let mut body = editor.note.body.clone();
                        let output = egui::TextEdit::multiline(&mut body)
                            .desired_width(f32::INFINITY)
                            .desired_rows(20)
                            .show(ui);
                        if body != editor.note.body {
                            editor.set_body(body.clone());
                        }

                        let Some(range) = output.cursor_range else {
                            return;
                        };
                        let byte = char_index_to_byte_offset(&body, range.primary.index.0);
                        let before = &body[..byte];

                        if slash_menu_triggered(before) {
                            ui.horizontal_wrapped(|ui| {
                                for tmpl in slash_templates() {
                                    if ui.button(tmpl.label).clicked() {
                                        let mut new_body = body.clone();
                                        new_body.replace_range(byte - 1..byte, tmpl.insert);
                                        editor.set_body(new_body);
                                    }
                                }
                            });
                        } else if let Some(query) = wikilink_autocomplete_query(before) {
                            let suggestions = wikilink_index
                                .as_ref()
                                .map(|idx| idx.suggestions(&query, 8))
                                .unwrap_or_default();
                            if !suggestions.is_empty() {
                                ui.horizontal_wrapped(|ui| {
                                    for title in &suggestions {
                                        if ui.button(title).clicked() {
                                            let mut new_body = body.clone();
                                            new_body.replace_range(
                                                byte - query.len()..byte,
                                                &format!("{title}]]"),
                                            );
                                            editor.set_body(new_body);
                                        }
                                    }
                                });
                            }
                        }
                    }
                    EditorMode::LivePreview | EditorMode::Reading => {
                        let outcome = editor.render(ui, cache, viewport);
                        if let Some(new_body) = outcome.updated_body {
                            editor.set_body(new_body);
                        }
                        if let Some(title) = outcome.clicked_wikilink {
                            navigate_to = Some(title);
                        }
                    }
                    EditorMode::Edgeless => {}
                });
            }
        });

        if let Some(slug) = scroll_to_slug {
            self.markdown_cache.scroll_to_id_target_mut().replace(slug);
        }

        if close_requested {
            if editor.is_dirty() {
                if let Err(e) = editor.autosave() {
                    self.report_error("error-context-autosave", e);
                }
            }
            self.rescan_and_reindex();
            self.reindex_note(&editor.note);
        } else if let Some(title) = navigate_to {
            if editor.is_dirty() {
                if let Err(e) = editor.autosave() {
                    self.report_error("error-context-autosave", e);
                }
            }
            self.rescan_and_reindex();
            self.reindex_note(&editor.note);
            self.navigate_wikilink(&title);
        } else {
            self.editor = Some(editor);
        }
    }

    /// Renders an interactive 2D infinite spatial canvas surface for Whiteboard / Edgeless mode.
    /// Returns `true` if any element or viewport modification occurred.
    fn show_canvas_surface(
        canvas: &mut CanvasDocument,
        interaction: &mut InteractionState,
        ui: &mut egui::Ui,
        is_dark: bool,
    ) -> bool {
        let (response, painter) = ui.allocate_painter(
            ui.available_size_before_wrap(),
            egui::Sense::click_and_drag(),
        );
        let screen_rect = response.rect;
        let origin = screen_rect.min;
        let mut modified = false;

        // 1. Zoom & Pan input handling
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        let zoom_delta = ui.input(|i| i.zoom_delta());
        let ctrl_pressed = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);

        if (zoom_delta - 1.0).abs() > 1e-4 {
            if let Some(hover_pos) = response.hover_pos() {
                canvas.viewport.zoom_at(zoom_delta, hover_pos, origin);
                modified = true;
            }
        } else if ctrl_pressed && scroll_delta.y != 0.0 {
            let zoom_factor = if scroll_delta.y > 0.0 { 1.1 } else { 0.9 };
            if let Some(hover_pos) = response.hover_pos() {
                canvas.viewport.zoom_at(zoom_factor, hover_pos, origin);
                modified = true;
            }
        } else if scroll_delta != egui::Vec2::ZERO {
            canvas
                .viewport
                .add_pan_vec(scroll_delta / canvas.viewport.zoom);
            modified = true;
        }

        // 2. Draw Infinite Dot Grid
        canvas.viewport.draw_grid(&painter, screen_rect, is_dark);

        // 3. Pointer drag & click events for tools
        if response.drag_started() {
            if let Some(pos) = response.interact_pointer_pos() {
                let world_pos = canvas.viewport.screen_to_world(pos, origin);
                interaction.is_dragging = true;
                interaction.drag_start_world = Some([world_pos.x, world_pos.y]);
                interaction.drag_current_world = Some([world_pos.x, world_pos.y]);

                if interaction.active_tool == CanvasTool::Pen {
                    interaction.current_freehand_points = vec![[world_pos.x, world_pos.y]];
                }
            }
        } else if response.dragged() {
            let drag_delta = response.drag_delta();
            if let Some(pos) = response.interact_pointer_pos() {
                let world_pos = canvas.viewport.screen_to_world(pos, origin);
                interaction.drag_current_world = Some([world_pos.x, world_pos.y]);

                match interaction.active_tool {
                    CanvasTool::Pan => {
                        canvas
                            .viewport
                            .add_pan_vec(drag_delta / canvas.viewport.zoom);
                        modified = true;
                    }
                    CanvasTool::Pen => {
                        interaction
                            .current_freehand_points
                            .push([world_pos.x, world_pos.y]);
                    }
                    CanvasTool::Select => {
                        if let Some(start) = interaction.drag_start_world {
                            let start_world = egui::Pos2::new(start[0], start[1]);
                            let zoom = canvas.viewport.zoom;
                            if let Some(elem) = canvas.element_at(start_world) {
                                let elem_id = elem.id();
                                if let Some(target) = canvas.get_element_mut(elem_id) {
                                    target.translate(drag_delta / zoom);
                                    modified = true;
                                }
                            } else {
                                canvas.viewport.add_pan_vec(drag_delta / zoom);
                                modified = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        } else if response.drag_stopped() {
            if let (Some(start), Some(curr)) =
                (interaction.drag_start_world, interaction.drag_current_world)
            {
                let min_x = start[0].min(curr[0]);
                let min_y = start[1].min(curr[1]);
                let max_x = start[0].max(curr[0]);
                let max_y = start[1].max(curr[1]);
                let w = (max_x - min_x).max(80.0);
                let h = (max_y - min_y).max(50.0);

                match interaction.active_tool {
                    CanvasTool::StickyNote => {
                        canvas.add_element(CanvasElement::StickyNote {
                            id: CanvasElementId::new(),
                            pos: [start[0], start[1]],
                            size: [w.max(180.0), h.max(120.0)],
                            text: "Catatan Baru".to_string(),
                            color: interaction.primary_color,
                        });
                        interaction.active_tool = CanvasTool::Select;
                        modified = true;
                    }
                    CanvasTool::Shape(kind) => {
                        canvas.add_element(CanvasElement::Shape {
                            id: CanvasElementId::new(),
                            kind,
                            rect: [min_x, min_y, min_x + w.max(140.0), min_y + h.max(80.0)],
                            stroke_color: interaction.primary_color,
                            stroke_width: interaction.stroke_width,
                            fill_color: None,
                            text: String::new(),
                        });
                        interaction.active_tool = CanvasTool::Select;
                        modified = true;
                    }
                    CanvasTool::Connector => {
                        canvas.add_element(CanvasElement::Connector {
                            id: CanvasElementId::new(),
                            from_elem: None,
                            to_elem: None,
                            from_pos: start,
                            to_pos: curr,
                            routing: crate::canvas::ConnectorRouting::Straight,
                            stroke_color: interaction.primary_color,
                            stroke_width: interaction.stroke_width,
                            label: String::new(),
                            arrow_end: true,
                        });
                        interaction.active_tool = CanvasTool::Select;
                        modified = true;
                    }
                    CanvasTool::Pen => {
                        if interaction.current_freehand_points.len() >= 2 {
                            canvas.add_element(CanvasElement::FreehandStroke {
                                id: CanvasElementId::new(),
                                points: std::mem::take(&mut interaction.current_freehand_points),
                                color: interaction.primary_color,
                                width: interaction.stroke_width,
                            });
                            modified = true;
                        }
                    }
                    CanvasTool::Eraser => {
                        let click_pos = egui::Pos2::new(start[0], start[1]);
                        if let Some(elem) = canvas.element_at(click_pos) {
                            let id = elem.id();
                            canvas.remove_element(id);
                            modified = true;
                        }
                    }
                    _ => {}
                }
            }

            interaction.is_dragging = false;
            interaction.drag_start_world = None;
            interaction.drag_current_world = None;
        }

        // 4. Draw all canvas elements
        let hovered_id = response.hover_pos().and_then(|p| {
            let wp = canvas.viewport.screen_to_world(p, origin);
            canvas.element_at(wp).map(|e| e.id())
        });

        for elem in &canvas.elements {
            let is_sel = hovered_id == Some(elem.id());
            canvas::draw_element(&painter, &canvas.viewport, origin, elem, is_sel, is_dark);
        }

        // Draw live pen stroke in progress
        if interaction.active_tool == CanvasTool::Pen
            && interaction.current_freehand_points.len() >= 2
        {
            let screen_pts: Vec<egui::Pos2> = interaction
                .current_freehand_points
                .iter()
                .map(|p| {
                    canvas
                        .viewport
                        .world_to_screen(egui::Pos2::new(p[0], p[1]), origin)
                })
                .collect();
            let stroke_c = egui::Color32::from_rgb(
                (interaction.primary_color[0] * 255.0) as u8,
                (interaction.primary_color[1] * 255.0) as u8,
                (interaction.primary_color[2] * 255.0) as u8,
            );
            for w in screen_pts.windows(2) {
                painter.line_segment(
                    [w[0], w[1]],
                    (interaction.stroke_width * canvas.viewport.zoom, stroke_c),
                );
            }
        }

        // 5. Double click to edit sticky note / shape text
        if response.double_clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let world_pos = canvas.viewport.screen_to_world(pos, origin);
                if let Some(elem) = canvas.element_at(world_pos) {
                    interaction.editing_text_elem = Some(elem.id());
                }
            }
        }

        // Inline text editor overlay
        if let Some(editing_id) = interaction.editing_text_elem {
            let elem_info = canvas.get_element(editing_id).map(|e| {
                let b_rect = e.bounding_rect();
                let text = match e {
                    CanvasElement::StickyNote { text, .. } => text.clone(),
                    CanvasElement::Shape { text, .. } => text.clone(),
                    CanvasElement::Connector { label, .. } => label.clone(),
                    _ => String::new(),
                };
                (b_rect, text)
            });

            if let Some((b_rect, mut text_buf)) = elem_info {
                let s_rect = canvas.viewport.world_rect_to_screen(b_rect, origin);
                let mut close_edit = false;
                let mut changed = false;

                egui::Area::new(egui::Id::new("canvas_inline_text_edit_area"))
                    .fixed_pos(s_rect.min)
                    .order(egui::Order::Foreground)
                    .show(ui.ctx(), |ui| {
                        ui.set_max_width(s_rect.width().max(180.0));
                        let frame = egui::Frame {
                            inner_margin: egui::Margin::same(8),
                            outer_margin: egui::Margin::ZERO,
                            corner_radius: egui::CornerRadius::same(theme::ROUNDING_SM),
                            fill: if is_dark {
                                theme::BG_CARD_DARK
                            } else {
                                egui::Color32::WHITE
                            },
                            stroke: egui::Stroke::new(1.5, theme::ACCENT_BLUE),
                            shadow: egui::Shadow::NONE,
                        };
                        frame.show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("✏ Edit Teks")
                                        .size(11.0)
                                        .color(theme::ACCENT_BLUE),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.small_button("Selesai ✓").clicked() {
                                            close_edit = true;
                                        }
                                    },
                                );
                            });
                            let edit = egui::TextEdit::multiline(&mut text_buf)
                                .desired_width(s_rect.width().max(160.0))
                                .desired_rows(3);
                            let resp = ui.add(edit);
                            if resp.changed() {
                                changed = true;
                                modified = true;
                            }
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                close_edit = true;
                            }
                        });
                    });

                if changed {
                    if let Some(elem) = canvas.get_element_mut(editing_id) {
                        match elem {
                            CanvasElement::StickyNote { text, .. } => *text = text_buf,
                            CanvasElement::Shape { text, .. } => *text = text_buf,
                            CanvasElement::Connector { label, .. } => *label = text_buf,
                            _ => {}
                        }
                    }
                }

                if close_edit {
                    interaction.editing_text_elem = None;
                }
            } else {
                interaction.editing_text_elem = None;
            }
        }

        let screen_rect_ctx = ui.ctx().viewport_rect();

        // 6. Floating Left Tool Dock (Whiteboard Tools)
        let toolbar_pos = egui::pos2(14.0, (screen_rect_ctx.center().y - 140.0).max(60.0));
        egui::Area::new(egui::Id::new("mnemonic_canvas_left_toolbar_area"))
            .fixed_pos(toolbar_pos)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                if let Some(event) = ui::LeftToolbar::show_canvas(
                    ui,
                    interaction.active_tool,
                    false,
                    false,
                ) {
                    match event {
                        ui::LeftToolbarEvent::SelectCanvasTool(tool) => {
                            interaction.active_tool = tool;
                        }
                        _ => {}
                    }
                }
            });

        // 7. Floating Zoom HUD pill (Kanan Bawah)
        let zoom_hud_pos = egui::pos2(
            (screen_rect_ctx.max.x - 125.0).max(10.0),
            (screen_rect_ctx.max.y - 48.0).max(10.0),
        );
        egui::Area::new(egui::Id::new("mnemonic_canvas_zoom_hud_area"))
            .fixed_pos(zoom_hud_pos)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                if let Some(event) = ui::CanvasHud::show_zoom_hud(ui, canvas.viewport.zoom) {
                    match event {
                        ui::CanvasHudEvent::ZoomIn => {
                            canvas.viewport.zoom = (canvas.viewport.zoom * 1.15).min(5.0);
                            modified = true;
                        }
                        ui::CanvasHudEvent::ZoomOut => {
                            canvas.viewport.zoom = (canvas.viewport.zoom / 1.15).max(0.2);
                            modified = true;
                        }
                        ui::CanvasHudEvent::ResetZoom => {
                            canvas.viewport.zoom = 1.0;
                            modified = true;
                        }
                        _ => {}
                    }
                }
            });

        // 8. Floating Style HUD pill (Tengah Bawah)
        let style_hud_pos = egui::pos2(
            (screen_rect_ctx.center().x - 145.0).max(10.0),
            (screen_rect_ctx.max.y - 48.0).max(10.0),
        );
        egui::Area::new(egui::Id::new("mnemonic_canvas_style_hud_area"))
            .fixed_pos(style_hud_pos)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                if let Some(event) = ui::CanvasHud::show_style_hud(
                    ui,
                    interaction.primary_color,
                    interaction.stroke_width,
                ) {
                    match event {
                        ui::CanvasHudEvent::SetStrokeColor(col) => {
                            interaction.primary_color = col;
                        }
                        ui::CanvasHudEvent::SetStrokeWidth(w) => {
                            interaction.stroke_width = w;
                        }
                        _ => {}
                    }
                }
            });

        modified
    }

    /// Renders the standalone Whiteboard Canvas workspace (§AFFiNE Edgeless Workspace).
    fn show_standalone_canvas(&mut self, ui: &mut egui::Ui) {
        if self.standalone_canvas.is_none() {
            let mut canvas = CanvasDocument::new("Whiteboard");
            canvas.add_element(CanvasElement::StickyNote {
                id: CanvasElementId::new(),
                pos: [100.0, 100.0],
                size: [240.0, 130.0],
                text: "🎨 Selamat datang di Whiteboard Canvas!\n\nGunakan Tool Dock di sebelah kiri dan bawah untuk membuat Sticky Notes, Shapes, Connectors, atau Coretan Pena.".to_string(),
                color: crate::canvas::tools::PALETTE_STICKY_YELLOW,
            });
            self.standalone_canvas = Some(canvas);
        }

        let is_dark = ui.visuals().dark_mode;
        if let Some(canvas) = self.standalone_canvas.as_mut() {
            Self::show_canvas_surface(canvas, &mut self.standalone_canvas_interaction, ui, is_dark);
        }
    }

    /// Renders the AFFiNE-style Omnibox Command Palette (`Cmd+K` / `Ctrl+K`).
    fn show_command_palette_modal(&mut self, ctx: &egui::Context) {
        if !self.command_palette.is_open() {
            return;
        }

        let mut commands = Vec::new();
        if self.vault.is_some() {
            commands.push(ui::PaletteCommand {
                id: "new_note",
                category: "Catatan",
                icon: egui_icons::icons::ICON_NOTE_ADD.codepoint,
                label: "Catatan Baru",
                hint: "⌘N",
            });
            commands.push(ui::PaletteCommand {
                id: "new_canvas",
                category: "Kanvas",
                icon: egui_icons::icons::ICON_DRAW.codepoint,
                label: "Whiteboard Kanvas Baru",
                hint: "",
            });
            commands.push(ui::PaletteCommand {
                id: "nav_notes",
                category: "Navigasi",
                icon: egui_icons::icons::ICON_DESCRIPTION.codepoint,
                label: "Buka Grid Catatan",
                hint: "",
            });
            commands.push(ui::PaletteCommand {
                id: "nav_canvas",
                category: "Navigasi",
                icon: egui_icons::icons::ICON_DRAW.codepoint,
                label: "Buka Whiteboard Kanvas",
                hint: "",
            });
            commands.push(ui::PaletteCommand {
                id: "nav_search",
                category: "Navigasi",
                icon: egui_icons::icons::ICON_SEARCH.codepoint,
                label: "Pencarian Semantik RAG",
                hint: "",
            });
            commands.push(ui::PaletteCommand {
                id: "nav_chat",
                category: "Navigasi",
                icon: egui_icons::icons::ICON_AUTO_AWESOME.codepoint,
                label: "Tanya Asisten AI (Qwen)",
                hint: "",
            });
            commands.push(ui::PaletteCommand {
                id: "import_pdf",
                category: "PDF",
                icon: egui_icons::icons::ICON_UPLOAD.codepoint,
                label: "Impor Dokumen PDF",
                hint: "",
            });
            commands.push(ui::PaletteCommand {
                id: "manage_tags",
                category: "Label",
                icon: egui_icons::icons::ICON_PALETTE.codepoint,
                label: "Kelola Label & Tag",
                hint: "",
            });
        }
        commands.push(ui::PaletteCommand {
            id: "switch_vault",
            category: "Vault",
            icon: egui_icons::icons::ICON_FOLDER_OPEN.codepoint,
            label: "Pilih / Buka Folder Vault...",
            hint: "",
        });
        commands.push(ui::PaletteCommand {
            id: "toggle_theme",
            category: "Tampilan",
            icon: egui_icons::icons::ICON_DARK_MODE.codepoint,
            label: "Ganti Tema (Gelap / Terang)",
            hint: "",
        });

        if let Some(cmd_id) = self.command_palette.show(ctx, &commands) {
            match cmd_id {
                "new_note" => {
                    if let Some(vault) = self.vault.as_mut() {
                        if let Ok(note) = Note::create(&vault.root, "Catatan Baru", "") {
                            self.rescan_and_reindex();
                            self.open_note(note);
                        }
                    }
                }
                "new_canvas" => {
                    if let Some(vault) = self.vault.as_mut() {
                        if let Ok(note) = Note::create_canvas(&vault.root, "Kanvas Baru") {
                            self.rescan_and_reindex();
                            self.open_note(note);
                        }
                    }
                }
                "nav_notes" => {
                    self.view = View::Notes;
                    self.doc_filter = DocFilter::All;
                    self.editor = None;
                    self.pdf_viewer = None;
                }
                "nav_canvas" => {
                    self.view = View::Notes;
                    self.doc_filter = DocFilter::WhiteboardsOnly;
                    self.editor = None;
                    self.pdf_viewer = None;
                }
                "nav_search" => {
                    self.view = View::Search;
                    self.editor = None;
                    self.pdf_viewer = None;
                }
                "nav_chat" => {
                    self.view = View::Chat;
                    self.editor = None;
                    self.pdf_viewer = None;
                }
                "import_pdf" => {
                    if let Some(file) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() {
                        self.import_pdf(file);
                    }
                }
                "manage_tags" => self.show_label_manager = true,
                "switch_vault" => {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        match Vault::open(folder) {
                            Ok(v) => self.activate_vault(v),
                            Err(e) => self.report_error("error-context-open-vault", e),
                        }
                    }
                }
                "toggle_theme" => self.theme_mode = self.theme_mode.toggled(),
                _ => {}
            }
        }
    }

    /// Applies one grid action against the vault/disk, then re-syncs the
    /// in-memory index. Called once per accumulated `GridAction` after
    /// `show_grid`'s `egui` closures have all returned.
    fn apply_grid_action(&mut self, action: GridAction) {
        match action {
            GridAction::Open(path) => {
                let found = self
                    .vault
                    .as_ref()
                    .and_then(|v| v.notes.iter().find(|n| n.path == path).cloned());
                if let Some(note) = found {
                    self.open_note(note);
                }
            }
            GridAction::TogglePin(id) => {
                self.mutate_note(id, |n| n.frontmatter.pinned = !n.frontmatter.pinned)
            }
            GridAction::SetColor(id, color) => {
                self.mutate_note(id, |n| n.frontmatter.color = color)
            }
            GridAction::ToggleArchived(id) => {
                self.mutate_note(id, |n| n.frontmatter.archived = !n.frontmatter.archived)
            }
            GridAction::Trash(id) => self.move_note(id, |n, root| n.move_to_trash(root)),
            GridAction::Restore(id) => self.move_note(id, |n, root| n.restore_from_trash(root)),
            GridAction::DeletePermanently(id) => {
                let Some(vault) = self.vault.as_ref() else {
                    return;
                };
                let Some(note) = vault.notes.iter().find(|n| n.frontmatter.id == id).cloned()
                else {
                    return;
                };
                if let Err(e) = note.delete_permanently() {
                    self.report_error("error-context-delete-note", e);
                }
                // Purge any cached chunks for the deleted note so stale
                // text can't surface in search/chat retrieval (§5).
                if let Some(index) = self.index.as_ref()
                    && let Err(e) = index.delete_chunks_for_doc(id)
                {
                    log::warn!("app: failed to purge chunks for deleted note: {e}");
                }
                self.rescan_and_reindex();
            }
            GridAction::BatchArchive(ids) => {
                for id in ids {
                    self.mutate_note(id, |n| n.frontmatter.archived = true);
                }
            }
            GridAction::BatchTrash(ids) => {
                for id in ids {
                    self.move_note(id, |n, root| n.move_to_trash(root));
                }
            }
            GridAction::RenameTag(old, new) => {
                let Some(vault) = self.vault.as_mut() else {
                    return;
                };
                // Collected rather than reported inline: `vault` (borrowed
                // from `self.vault`) stays alive for the whole loop, and
                // `report_error` needs `&mut self` as a whole — so the
                // report has to wait until the loop (and `vault`'s borrow)
                // has ended. Only the last failure surfaces in the banner
                // if several notes in the batch fail to save.
                let mut save_error = None;
                for i in tags::rename_tag(&mut vault.notes, &old, &new) {
                    if let Err(e) = vault.notes[i].save() {
                        log::warn!("app: failed to save note after tag rename: {e}");
                        save_error = Some(e);
                    }
                }
                if let Some(e) = save_error {
                    self.report_error("error-context-save-note", e);
                }
                self.rescan_and_reindex();
            }
            GridAction::DeleteTag(tag) => {
                let Some(vault) = self.vault.as_mut() else {
                    return;
                };
                let mut save_error = None; // see RenameTag's comment above
                for i in tags::remove_tag(&mut vault.notes, &tag) {
                    if let Err(e) = vault.notes[i].save() {
                        log::warn!("app: failed to save note after tag delete: {e}");
                        save_error = Some(e);
                    }
                }
                if let Some(e) = save_error {
                    self.report_error("error-context-save-note", e);
                }
                self.rescan_and_reindex();
            }
        }
    }

    /// Loads the note `id`, applies `f` to its frontmatter, saves, and
    /// re-syncs the index. Used for the simple single-field toggles (pin,
    /// color, archive).
    fn mutate_note(&mut self, id: Uuid, f: impl FnOnce(&mut Note)) {
        let Some(vault) = self.vault.as_ref() else {
            return;
        };
        let Some(mut note) = vault.notes.iter().find(|n| n.frontmatter.id == id).cloned() else {
            return;
        };
        f(&mut note);
        if let Err(e) = note.save() {
            self.report_error("error-context-save-note", e);
            return;
        }
        self.rescan_and_reindex();
        self.reindex_note(&note);
    }

    /// Loads the note `id` and applies a move operation (trash/restore)
    /// that needs the vault root, then re-syncs the index.
    fn move_note(
        &mut self,
        id: Uuid,
        f: impl FnOnce(Note, &std::path::Path) -> anyhow::Result<Note>,
    ) {
        let Some(vault) = self.vault.as_ref() else {
            return;
        };
        let Some(note) = vault.notes.iter().find(|n| n.frontmatter.id == id).cloned() else {
            return;
        };
        let root = vault.root.clone();
        if let Err(e) = f(note, &root) {
            self.report_error("error-context-move-note", e);
            return;
        }
        self.rescan_and_reindex();
    }

    /// The unified document grid (Notes + PDFs) with Pinterest-style masonry
    /// layout. The fixed left sidebar has been replaced by a floating overlay
    /// (see `show_sidebar_overlay` in `ui()`). The sort/filter bar is here
    /// inline. `DocFilter` (set via the sidebar) controls what's shown.
    fn show_grid(&mut self, ui: &mut egui::Ui) {
        let t_sort_modified = self.t("sort-modified");
        let t_sort_created = self.t("sort-created");
        let t_sort_title = self.t("sort-title");
        let t_sort_color = self.t("sort-color");
        let t_selection_on = self.t("selection-mode-on");
        let t_selection_off = self.t("selection-mode-off");
        let t_selection_archive = self.t("selection-archive");
        let t_selection_trash = self.t("selection-trash");
        let t_empty = self.t("vault-empty");
        let t_empty_filtered = self.t("grid-empty-filtered");
        let t_confirm_title = self.t("confirm-delete-title");
        let t_confirm_body = self.t("confirm-delete-body");
        let t_confirm_yes = self.t("confirm-yes");
        let card = CardStrings {
            pin: self.t("notes-pin"),
            unpin: self.t("notes-unpin"),
            archive: self.t("card-archive"),
            unarchive: self.t("card-unarchive"),
            trash: self.t("card-trash"),
            restore: self.t("card-restore"),
            delete_permanent: self.t("card-delete-permanent"),
        };

        let notes: Vec<Note> = self
            .vault
            .as_ref()
            .map(|v| v.notes.clone())
            .unwrap_or_default();
        let all_tags = tags::all_tags(&notes);
        let pdf_docs = self.pdf_documents.clone();
        let doc_filter = self.doc_filter.clone();

        let mut sort_mode = self.sort_mode;
        let mut selection_mode = self.selection_mode;
        let mut selected = self.selected.clone();
        let mut show_label_manager = self.show_label_manager;
        let mut tag_rename = self.tag_rename.clone();
        let mut confirm_delete = self.confirm_delete;
        let mut actions: Vec<GridAction> = Vec::new();
        let mut open_pdf: Option<std::path::PathBuf> = None;

        // Spacing so content starts below floating top bar
        ui.add_space(theme::TOPBAR_HEIGHT + 14.0);

        // ── Floating sort / selection toolbar ──────────────────────────────
        egui::Frame::NONE
            .fill(egui::Color32::TRANSPARENT)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Sort picker — compact combo
                    ui.label(
                        egui::RichText::new("↕")
                            .color(theme::GLASS_TEXT_FAINT)
                            .size(13.0),
                    );
                    egui::ComboBox::from_id_salt("sort_mode")
                        .selected_text(
                            egui::RichText::new(match sort_mode {
                                SortMode::Modified => &t_sort_modified,
                                SortMode::Created => &t_sort_created,
                                SortMode::Title => &t_sort_title,
                                SortMode::Color => &t_sort_color,
                            })
                            .color(theme::GLASS_TEXT_SECONDARY)
                            .size(12.5),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut sort_mode,
                                SortMode::Modified,
                                &t_sort_modified,
                            );
                            ui.selectable_value(&mut sort_mode, SortMode::Created, &t_sort_created);
                            ui.selectable_value(&mut sort_mode, SortMode::Title, &t_sort_title);
                            ui.selectable_value(&mut sort_mode, SortMode::Color, &t_sort_color);
                        });

                    ui.add_space(8.0);

                    // Multi-select toggle
                    let sel_label = if selection_mode {
                        &t_selection_off
                    } else {
                        &t_selection_on
                    };
                    let sel_btn = egui::Button::new(
                        egui::RichText::new(sel_label)
                            .color(theme::GLASS_TEXT_SECONDARY)
                            .size(12.5),
                    )
                    .fill(egui::Color32::TRANSPARENT);
                    if ui.add(sel_btn).clicked() {
                        selection_mode = !selection_mode;
                        if !selection_mode {
                            selected.clear();
                        }
                    }

                    if selection_mode && !selected.is_empty() {
                        ui.add_space(8.0);
                        let archive_btn = egui::Button::new(
                            egui::RichText::new(&t_selection_archive)
                                .color(theme::GLASS_TEXT_SECONDARY)
                                .size(12.5),
                        )
                        .fill(egui::Color32::TRANSPARENT);
                        if ui.add(archive_btn).clicked() {
                            actions
                                .push(GridAction::BatchArchive(selected.iter().copied().collect()));
                            selected.clear();
                        }
                        let trash_btn = egui::Button::new(
                            egui::RichText::new(&t_selection_trash)
                                .color(theme::GLASS_ERROR)
                                .size(12.5),
                        )
                        .fill(egui::Color32::TRANSPARENT);
                        if ui.add(trash_btn).clicked() {
                            actions
                                .push(GridAction::BatchTrash(selected.iter().copied().collect()));
                            selected.clear();
                        }
                    }
                });
            });

        ui.add_space(8.0);

        // ── Compute which notes/PDFs to show ───────────────────────────────
        // Apply grid_filter (legacy) mapped from doc_filter for backward compat.
        let grid_filter_mapped = match &doc_filter {
            DocFilter::All | DocFilter::NotesOnly | DocFilter::WhiteboardsOnly => GridFilter::All,
            DocFilter::PdfsOnly => GridFilter::All, // PDFs handled separately below
            DocFilter::Archived => GridFilter::Archived,
            DocFilter::Trashed => GridFilter::Trashed,
            DocFilter::Tag(t) => GridFilter::Tag(t.clone()),
        };
        let mut visible = query::filter_notes(&notes, &grid_filter_mapped, &self.search_text);
        query::sort_notes(&mut visible, sort_mode);
        let in_trash_view = matches!(&doc_filter, DocFilter::Trashed);

        // PDFs shown only if filter is All or PdfsOnly, not in trash/archived
        let show_pdfs = matches!(&doc_filter, DocFilter::All | DocFilter::PdfsOnly)
            && !in_trash_view
            && !matches!(&doc_filter, DocFilter::Archived);

        // Filter PDFs by search text
        let visible_pdfs: Vec<std::path::PathBuf> = if show_pdfs {
            pdf_docs
                .iter()
                .filter(|p| {
                    if self.search_text.is_empty() {
                        true
                    } else {
                        p.file_name()
                            .map(|n| n.to_string_lossy().to_lowercase())
                            .unwrap_or_default()
                            .contains(&self.search_text.to_lowercase())
                    }
                })
                .cloned()
                .collect()
        } else {
            Vec::new()
        };

        // Hide notes when PdfsOnly filter is active; or filter to canvas notes if WhiteboardsOnly
        let visible_notes: Vec<&Note> = if matches!(&doc_filter, DocFilter::PdfsOnly) {
            Vec::new()
        } else if matches!(&doc_filter, DocFilter::WhiteboardsOnly) {
            visible.into_iter().filter(|n| n.is_canvas()).collect()
        } else {
            visible
        };

        let total_items = visible_notes.len() + visible_pdfs.len();
        let has_notes_in_vault = !notes.is_empty();

        if !has_notes_in_vault && pdf_docs.is_empty() {
            // Empty vault state — centered message
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(
                    egui::RichText::new("✏")
                        .size(48.0)
                        .color(theme::GLASS_TEXT_FAINT),
                );
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new(&t_empty)
                        .size(14.0)
                        .color(theme::GLASS_TEXT_SECONDARY),
                );
            });
            return;
        }

        if total_items == 0 {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(
                    egui::RichText::new(&t_empty_filtered)
                        .size(14.0)
                        .color(theme::GLASS_TEXT_SECONDARY),
                );
            });
        } else {
            // ── Pinterest masonry grid ─────────────────────────────────────
            // Determine column count based on available width.
            egui::ScrollArea::vertical().show(ui, |ui| {
                let available_width = ui.available_width();
                let col_count: usize = if available_width < 480.0 {
                    1
                } else if available_width < 760.0 {
                    2
                } else if available_width < 1100.0 {
                    3
                } else {
                    4
                };
                let gap = theme::GRID_GAP;
                let col_width =
                    (available_width - gap * (col_count as f32 - 1.0)) / col_count as f32;

                // Render all items in a horizontal row of `col_count` columns.
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(gap, 0.0);

                    // Pre-allocate column containers with fixed width
                    for col_idx in 0..col_count {
                        let is_last = col_idx == col_count - 1;
                        let this_col_width = if is_last {
                            // Last column takes any remaining space
                            ui.available_width()
                        } else {
                            col_width
                        };

                        ui.allocate_ui(egui::vec2(this_col_width, ui.available_height()), |ui| {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing = egui::vec2(0.0, gap);

                                // Note cards for this column
                                for (item_idx, note) in visible_notes.iter().enumerate() {
                                    if item_idx % col_count == col_idx {
                                        render_note_card_glass(
                                            ui,
                                            note,
                                            &card,
                                            selection_mode,
                                            selected.contains(&note.frontmatter.id),
                                            in_trash_view,
                                            &mut actions,
                                            &mut selected,
                                            &mut confirm_delete,
                                        );
                                    }
                                }

                                // PDF cards for this column
                                let pdf_offset = visible_notes.len();
                                for (item_idx, path) in visible_pdfs.iter().enumerate() {
                                    if (item_idx + pdf_offset) % col_count == col_idx {
                                        render_pdf_card_glass(ui, path, &mut open_pdf);
                                    }
                                }
                            });
                        });
                    }
                });
            });
        }

        // ── Dialogs ────────────────────────────────────────────────────────
        if let Some(id) = confirm_delete {
            if let Some(confirmed) = ui::ConfirmModal::show(
                ui.ctx(),
                &t_confirm_title,
                &t_confirm_body,
                &t_confirm_yes,
                true,
            ) {
                if confirmed {
                    actions.push(GridAction::DeletePermanently(id));
                }
                confirm_delete = None;
            }
        }

        if show_label_manager {
            let mut rename_draft = tag_rename
                .as_ref()
                .map(|(_, d)| d.clone())
                .unwrap_or_default();
            let mut target_tag = tag_rename.as_ref().map(|(t, _)| t.clone());
            if let Some(evt) = ui::LabelManagerModal::show(
                ui.ctx(),
                &all_tags,
                &mut rename_draft,
                &mut target_tag,
            ) {
                match evt {
                    ui::LabelManagerEvent::Rename { old_tag, new_tag } => {
                        actions.push(GridAction::RenameTag(old_tag, new_tag));
                        tag_rename = None;
                    }
                    ui::LabelManagerEvent::Delete(tag) => {
                        actions.push(GridAction::DeleteTag(tag));
                        tag_rename = None;
                    }
                    ui::LabelManagerEvent::Close => {
                        show_label_manager = false;
                        tag_rename = None;
                    }
                }
            } else if let Some(target) = target_tag {
                tag_rename = Some((target, rename_draft));
            }
        }

        // ── Write back ─────────────────────────────────────────────────────
        self.sort_mode = sort_mode;
        self.selection_mode = selection_mode;
        self.selected = selected;
        self.show_label_manager = show_label_manager;
        self.tag_rename = tag_rename;
        self.confirm_delete = confirm_delete;

        for action in actions {
            self.apply_grid_action(action);
        }
        if let Some(path) = open_pdf {
            self.open_pdf(path);
        }
    }

    /// The Search tab (§Fase 7): a query box plus one unified, scrollable
    /// result list mixing keyword and semantic hits (see `core::search`).
    /// Clicking a result opens the underlying note directly when its
    /// `doc_id` resolves to one; PDF results (Fase 8+) will need a
    /// dedicated viewer to jump to instead.
    fn show_search(&mut self, ui: &mut egui::Ui) {
        let t_placeholder = self.t("search-placeholder");
        let t_button = self.t("search-button");
        let t_prompt = self.t("search-prompt");
        let t_no_results = self.t("search-no-results");

        let mut open_note: Option<Uuid> = None;
        // A hit with a page number came from a PDF chunk (notes never set
        // `page_num`, per `core::ingestion`) — §Fase 8 gives it somewhere
        // to jump to.
        let mut open_pdf_hit: Option<(PathBuf, usize)> = None;

        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(theme::TOPBAR_HEIGHT + 14.0);
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.search_query)
                        .hint_text(t_placeholder.as_str())
                        .desired_width(320.0),
                );
                let enter_pressed =
                    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button(&t_button).clicked() || enter_pressed {
                    self.run_search();
                }
            });

            if !self.search_status.is_empty() {
                ui.colored_label(egui::Color32::RED, &self.search_status);
            }
            ui.separator();

            if self.search_query.trim().is_empty() {
                ui.label(&t_prompt);
            } else if self.search_results.is_empty() {
                ui.label(&t_no_results);
            } else {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for hit in &self.search_results {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let title = hit
                                    .chunk
                                    .file_path
                                    .file_stem()
                                    .map(|s| s.to_string_lossy().to_string())
                                    .unwrap_or_else(|| hit.chunk.file_path.display().to_string());
                                if ui.link(egui::RichText::new(title).strong()).clicked() {
                                    match hit.chunk.page_num {
                                        Some(page) => {
                                            open_pdf_hit = Some((
                                                hit.chunk.file_path.clone(),
                                                page.saturating_sub(1),
                                            ))
                                        }
                                        None => open_note = Some(hit.chunk.doc_id),
                                    }
                                }
                                if let Some(score) = hit.score {
                                    ui.label(format!("{:.0}%", score * 100.0));
                                }
                            });
                            ui.label(query::snippet(&hit.chunk.text_content, 220));
                        });
                    }
                });
            }
        });

        if let Some(id) = open_note {
            let found = self
                .vault
                .as_ref()
                .and_then(|v| v.notes.iter().find(|n| n.frontmatter.id == id).cloned());
            if let Some(note) = found {
                self.open_note(note);
            }
        } else if let Some((path, page_index)) = open_pdf_hit {
            self.open_pdf_at_page(path, page_index);
        }
    }

    /// The Chat tab (§Fase 7, §3.4): a scrollable bubble transcript plus
    /// an input row. Sending a message embeds the question in the
    /// background, retrieves top-scoring context chunks once that
    /// resolves, and streams the LLM's grounded reply token-by-token —
    /// see `send_chat_message`/`start_chat_generation`/
    /// `poll_search_and_chat`.
    fn show_chat(&mut self, ui: &mut egui::Ui) {
        let t_placeholder = self.t("chat-placeholder");
        let t_send = self.t("chat-send");
        let t_empty = self.t("chat-empty");
        let t_sources = self.t("chat-sources");
        let t_thinking = self.t("chat-thinking");

        let busy = self.chat_pending_embed_id.is_some() || self.chat_pending_gen_id.is_some();
        let messages = self.chat_messages.clone();
        let mut clicked_citation: Option<Citation> = None;

        egui::CentralPanel::default().show(ui, |ui| {
            egui::Panel::bottom("chat_input_row").show(ui, |ui| {
                ui.horizontal(|ui| {
                    let input = egui::TextEdit::singleline(&mut self.chat_input)
                        .hint_text(t_placeholder.as_str())
                        .desired_width(f32::INFINITY);
                    let response = ui.add(input);
                    let enter_pressed =
                        response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let can_send = !busy && !self.chat_input.trim().is_empty();
                    let clicked = ui
                        .add_enabled(can_send, egui::Button::new(&t_send))
                        .clicked();
                    if can_send && (clicked || enter_pressed) {
                        self.send_chat_message();
                    }
                });
            });

            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(theme::TOPBAR_HEIGHT + 14.0);
                if messages.is_empty() {
                    ui.label(&t_empty);
                }
                for msg in &messages {
                    let is_user = msg.role == ChatRole::User;
                    let layout = if is_user {
                        egui::Layout::top_down(egui::Align::Max)
                    } else {
                        egui::Layout::top_down(egui::Align::Min)
                    };
                    ui.with_layout(layout, |ui| {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_max_width(ui.available_width() * 0.75);
                            ui.label(&msg.text);
                            if !msg.citations.is_empty() {
                                ui.separator();
                                ui.label(egui::RichText::new(&t_sources).small().weak());
                                for citation in &msg.citations {
                                    // Clickable — §3.4 point 4's "jump straight to the source",
                                    // finally reachable now that Fase 8 has a PDF viewer to jump to.
                                    if ui
                                        .link(egui::RichText::new(&citation.label).small())
                                        .clicked()
                                    {
                                        clicked_citation = Some(citation.clone());
                                    }
                                }
                            }
                        });
                    });
                }
                if busy {
                    ui.label(&t_thinking);
                }
            });
        });

        if let Some(citation) = clicked_citation {
            match citation.page_index {
                Some(page_index) => self.open_pdf_at_page(citation.file_path, page_index),
                None => {
                    let found = self.vault.as_ref().and_then(|v| {
                        v.notes
                            .iter()
                            .find(|n| n.path == citation.file_path)
                            .cloned()
                    });
                    if let Some(note) = found {
                        self.open_note(note);
                    }
                }
            }
        }
    }

    /// The PDF tab (§Fase 8): an import button plus the list of
    /// previously imported PDFs (`core::storage::IndexStore`'s
    /// `pdf_documents` table). Follows `show_grid`'s pattern of only
    /// touching plain locals inside the `egui` closures, deferring all
    /// mutation to `apply_pdf_library_action` afterward.
    fn show_pdf_library(&mut self, ui: &mut egui::Ui) {
        let t_import = self.t("pdf-import");
        let t_empty = self.t("pdf-library-empty");
        let t_open = self.t("pdf-open");
        let t_remove = self.t("pdf-remove");

        let documents = self.pdf_documents.clone();
        let mut actions: Vec<PdfLibraryAction> = Vec::new();

        egui::CentralPanel::default().show(ui, |ui| {
            if ui.button(&t_import).clicked()
                && let Some(file) = rfd::FileDialog::new()
                    .add_filter("PDF", &["pdf"])
                    .pick_file()
            {
                actions.push(PdfLibraryAction::Import(file));
            }

            ui.separator();

            if documents.is_empty() {
                ui.label(&t_empty);
            } else {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for path in &documents {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let name = path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_else(|| path.display().to_string());
                                ui.label(name);
                                if ui.button(&t_open).clicked() {
                                    actions.push(PdfLibraryAction::Open(path.clone()));
                                }
                                if ui.button(&t_remove).clicked() {
                                    actions.push(PdfLibraryAction::Remove(path.clone()));
                                }
                            });
                        });
                    }
                });
            }
        });

        for action in actions {
            self.apply_pdf_library_action(action);
        }
    }

    /// The PDF viewer (§Fase 8, §3.5 point 1): page navigation + zoom, the
    /// rendered page bitmap, a page-ops toolbar (rotate/delete/split/
    /// merge, §3.5 point 2), and — new in Fase 9 — an annotation canvas
    /// (highlight/underline/sticky-note/text-injection, drawn by
    /// dragging over the rendered page), a metadata editor, and
    /// Save/Export. Takes over the central panel exactly like
    /// `show_editor` does for notes. Rendering the current page into a
    /// texture happens up front (outside any `egui` closure) so the rest
    /// of this method can follow `show_editor`'s "closures only ever
    /// touch plain locals" convention.
    fn show_pdf_viewer(&mut self, ui: &mut egui::Ui) {
        let Some(mut viewer) = self.pdf_viewer.take() else {
            return;
        };

        let key = (viewer.page_index, viewer.zoom_width);
        if viewer.rendered_key != Some(key) {
            match self.ensure_pdf_renderer() {
                Ok(renderer) => {
                    // Zoom doesn't change the page's true point size, but
                    // recomputing on every zoom change too is harmless —
                    // this still only reopens the doc when `key` changes,
                    // not every frame.
                    viewer.page_size_points = renderer
                        .page_size_points(&viewer.path, viewer.page_index)
                        .ok();
                    match renderer.render_page(&viewer.path, viewer.page_index, viewer.zoom_width) {
                        Ok(page) => {
                            let image = egui::ColorImage::from_rgba_unmultiplied(
                                [page.width, page.height],
                                &page.rgba,
                            );
                            viewer.texture = Some(ui.ctx().load_texture(
                                "pdf-page",
                                image,
                                egui::TextureOptions::LINEAR,
                            ));
                            viewer.render_error = None;
                        }
                        Err(e) => {
                            viewer.texture = None;
                            viewer.render_error = Some(format!("{e:#}"));
                        }
                    }
                }
                Err(msg) => {
                    viewer.texture = None;
                    viewer.render_error = Some(msg.to_string());
                    viewer.page_size_points = None;
                }
            }
            viewer.rendered_key = Some(key);
        }

        let t_back = self.t("pdf-back");
        let t_page_of = self.locales.t(
            "pdf-page-of",
            &[
                ("current", &(viewer.page_index + 1).to_string()),
                ("total", &viewer.page_count.to_string()),
            ],
        );
        let t_zoom = self.t("pdf-zoom");
        let t_rotate_left = self.t("pdf-rotate-left");
        let t_rotate_right = self.t("pdf-rotate-right");
        let t_delete_page = self.t("pdf-delete-page");
        let t_delete_confirm = self.t("pdf-delete-page-confirm");
        let t_confirm_yes = self.t("confirm-yes");
        let t_confirm_cancel = self.t("confirm-cancel");
        let t_split = self.t("pdf-split");
        let t_split_to = self.t("pdf-split-to");
        let t_split_go = self.t("pdf-split-go");
        let t_merge = self.t("pdf-merge");
        let t_render_unavailable = self.t("pdf-render-unavailable");
        let t_annotate_none = self.t("pdf-annotate-none");
        let t_annotate_highlight = self.t("pdf-annotate-highlight");
        let t_annotate_underline = self.t("pdf-annotate-underline");
        let t_annotate_sticky = self.t("pdf-annotate-sticky");
        let t_annotate_text = self.t("pdf-annotate-text");
        let t_annotate_color = self.t("pdf-annotate-color");
        let t_annotate_sticky_prompt = self.t("pdf-annotate-sticky-prompt");
        let t_annotate_text_prompt = self.t("pdf-annotate-text-prompt");
        let t_annotate_add = self.t("pdf-annotate-add");
        let t_annotate_cancel = self.t("pdf-annotate-cancel");
        let t_metadata_button = self.t("pdf-metadata-button");
        let t_metadata_window_title = self.t("pdf-metadata-window-title");
        let t_metadata_field_title = self.t("pdf-metadata-field-title");
        let t_metadata_field_author = self.t("pdf-metadata-field-author");
        let t_metadata_field_keywords = self.t("pdf-metadata-field-keywords");
        let t_metadata_close = self.t("pdf-metadata-close");
        let t_save = self.t("pdf-save");
        let t_save_confirm = self.t("pdf-save-confirm");
        let t_export = self.t("pdf-export");

        let path = viewer.path.clone();
        let page_count = viewer.page_count;
        let texture = viewer.texture.clone();
        let render_error = viewer.render_error.clone();
        let page_size_points = viewer.page_size_points;

        let mut page_index = viewer.page_index;
        let mut zoom_width = viewer.zoom_width;
        let mut split_from = viewer.split_from;
        let mut split_to = viewer.split_to;
        let mut delete_confirm = viewer.delete_confirm;
        let mut back_requested = false;
        let mut actions: Vec<PdfViewerAction> = Vec::new();

        // Annotation canvas + metadata editor + save/export (§Fase 9).
        let mut annotate_tool = viewer.annotate_tool;
        let mut annotate_color = viewer.annotate_color;
        let mut drag_start = viewer.drag_start;
        let mut staged_annotations = std::mem::take(&mut viewer.staged_annotations);
        let mut pending_annotation = viewer.pending_annotation.take();
        let mut pending_annotation_text = std::mem::take(&mut viewer.pending_annotation_text);
        let mut show_metadata_editor = viewer.show_metadata_editor;
        let mut metadata_title = std::mem::take(&mut viewer.metadata_title);
        let mut metadata_author = std::mem::take(&mut viewer.metadata_author);
        let mut metadata_keywords = std::mem::take(&mut viewer.metadata_keywords);
        let mut show_save_confirm = viewer.show_save_confirm;
        let mut export_requested = false;

        egui::Panel::top("pdf_top_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button(&t_back).clicked() {
                    back_requested = true;
                }
                ui.separator();
                if ui
                    .add_enabled(page_index > 0, egui::Button::new("⏴"))
                    .clicked()
                {
                    page_index -= 1;
                }
                ui.label(&t_page_of);
                if ui
                    .add_enabled(page_index + 1 < page_count, egui::Button::new("⏵"))
                    .clicked()
                {
                    page_index += 1;
                }
                ui.separator();
                ui.label(&t_zoom);
                ui.add(
                    egui::DragValue::new(&mut zoom_width)
                        .range(200..=3000)
                        .speed(10),
                );
            });
        });

        egui::Panel::top("pdf_annotate_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(annotate_tool.is_none(), &t_annotate_none)
                    .clicked()
                {
                    annotate_tool = None;
                }
                if ui
                    .selectable_label(
                        annotate_tool == Some(AnnotationKind::Highlight),
                        &t_annotate_highlight,
                    )
                    .clicked()
                {
                    annotate_tool = Some(AnnotationKind::Highlight);
                }
                if ui
                    .selectable_label(
                        annotate_tool == Some(AnnotationKind::Underline),
                        &t_annotate_underline,
                    )
                    .clicked()
                {
                    annotate_tool = Some(AnnotationKind::Underline);
                }
                if ui
                    .selectable_label(
                        annotate_tool == Some(AnnotationKind::StickyNote),
                        &t_annotate_sticky,
                    )
                    .clicked()
                {
                    annotate_tool = Some(AnnotationKind::StickyNote);
                }
                if ui
                    .selectable_label(
                        annotate_tool == Some(AnnotationKind::TextInjection),
                        &t_annotate_text,
                    )
                    .clicked()
                {
                    annotate_tool = Some(AnnotationKind::TextInjection);
                }
                ui.separator();
                ui.label(&t_annotate_color);
                ui.color_edit_button_rgb(&mut annotate_color);
                ui.separator();
                if ui.button(&t_metadata_button).clicked() {
                    show_metadata_editor = true;
                }
                ui.separator();
                if ui.button(&t_save).clicked() {
                    show_save_confirm = true;
                }
                if ui.button(&t_export).clicked() {
                    export_requested = true;
                }
            });
        });

        egui::Panel::bottom("pdf_ops_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button(&t_rotate_left).clicked() {
                    actions.push(PdfViewerAction::RotateCurrentPage {
                        path: path.clone(),
                        page: (page_index + 1) as u32,
                        degrees: -90,
                    });
                }
                if ui.button(&t_rotate_right).clicked() {
                    actions.push(PdfViewerAction::RotateCurrentPage {
                        path: path.clone(),
                        page: (page_index + 1) as u32,
                        degrees: 90,
                    });
                }
                if ui.button(&t_delete_page).clicked() {
                    delete_confirm = true;
                }
                ui.separator();
                ui.label(&t_split);
                ui.add(egui::DragValue::new(&mut split_from).range(1..=page_count as u32));
                ui.label(&t_split_to);
                ui.add(egui::DragValue::new(&mut split_to).range(1..=page_count as u32));
                if ui.button(&t_split_go).clicked() {
                    actions.push(PdfViewerAction::Split {
                        path: path.clone(),
                        from: split_from,
                        to: split_to,
                    });
                }
                ui.separator();
                if ui.button(&t_merge).clicked() {
                    actions.push(PdfViewerAction::Merge { path: path.clone() });
                }
            });
            if !viewer.op_status.is_empty() {
                ui.label(&viewer.op_status);
            }
        });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::both().show(ui, |ui| {
                if let Some(err) = &render_error {
                    ui.colored_label(egui::Color32::RED, format!("{t_render_unavailable}: {err}"));
                } else if let Some(tex) = &texture {
                    // A tool is selected → sense drag (to draw an
                    // annotation); otherwise just hover, so the image
                    // doesn't eat scroll/pan interactions when the user
                    // is only reading (§Fase 9).
                    let sense = if annotate_tool.is_some() {
                        egui::Sense::click_and_drag()
                    } else {
                        egui::Sense::hover()
                    };
                    let img_response =
                        ui.add(egui::Image::new((tex.id(), tex.size_vec2())).sense(sense));
                    let img_rect = img_response.rect;
                    let current_page = (page_index + 1) as u32;

                    if let Some((page_w, page_h)) = page_size_points
                        && page_w > 0.0
                        && page_h > 0.0
                    {
                        let painter = ui.painter();
                        for annotation in
                            staged_annotations.iter().filter(|a| a.page == current_page)
                        {
                            let screen_rect =
                                annotation_screen_rect(annotation.rect, img_rect, page_w, page_h);
                            draw_annotation_overlay(
                                painter,
                                screen_rect,
                                annotation.kind,
                                annotation.color,
                                &annotation.contents,
                            );
                        }

                        if let Some(tool) = annotate_tool {
                            if img_response.drag_started() {
                                drag_start = img_response.interact_pointer_pos();
                            }
                            if img_response.dragged()
                                && let (Some(start), Some(current)) =
                                    (drag_start, img_response.interact_pointer_pos())
                            {
                                let live_rect =
                                    egui::Rect::from_two_pos(start, current).intersect(img_rect);
                                let stroke_color = egui::Color32::from_rgb(
                                    (annotate_color[0] * 255.0) as u8,
                                    (annotate_color[1] * 255.0) as u8,
                                    (annotate_color[2] * 255.0) as u8,
                                );
                                painter.rect_stroke(
                                    live_rect,
                                    egui::CornerRadius::ZERO,
                                    (2.0, stroke_color),
                                    egui::StrokeKind::Middle,
                                );
                            }
                            if img_response.drag_stopped()
                                && let Some(start) = drag_start
                            {
                                let end = img_response.interact_pointer_pos().unwrap_or(start);
                                let mut screen_rect =
                                    egui::Rect::from_two_pos(start, end).intersect(img_rect);
                                if screen_rect.width() < MIN_ANNOTATION_DRAG_PX
                                    && screen_rect.height() < MIN_ANNOTATION_DRAG_PX
                                {
                                    // Treat as a click: place a default-sized box anchored at the click point.
                                    screen_rect = egui::Rect::from_min_size(
                                        start,
                                        default_annotation_size_px(tool),
                                    )
                                    .intersect(img_rect);
                                }

                                if screen_rect.width() > 0.5 && screen_rect.height() > 0.5 {
                                    let sx = page_w / img_rect.width();
                                    let sy = page_h / img_rect.height();
                                    let local_left = screen_rect.left() - img_rect.left();
                                    let local_right = screen_rect.right() - img_rect.left();
                                    let local_top = screen_rect.top() - img_rect.top();
                                    let local_bottom = screen_rect.bottom() - img_rect.top();
                                    // PDF y grows upward from the bottom; screen y grows downward from the top.
                                    let pdf_rect = (
                                        local_left * sx,
                                        page_h - local_bottom * sy,
                                        local_right * sx,
                                        page_h - local_top * sy,
                                    );

                                    match tool {
                                        AnnotationKind::Highlight | AnnotationKind::Underline => {
                                            staged_annotations.push(Annotation {
                                                kind: tool,
                                                page: current_page,
                                                rect: pdf_rect,
                                                color: (
                                                    annotate_color[0],
                                                    annotate_color[1],
                                                    annotate_color[2],
                                                ),
                                                contents: String::new(),
                                            });
                                        }
                                        AnnotationKind::StickyNote
                                        | AnnotationKind::TextInjection => {
                                            pending_annotation = Some(PendingAnnotation {
                                                kind: tool,
                                                page: current_page,
                                                rect: pdf_rect,
                                            });
                                            pending_annotation_text.clear();
                                        }
                                    }
                                }
                                drag_start = None;
                            }
                        }
                    }
                }
            });
        });

        if delete_confirm {
            egui::Window::new(&t_delete_page)
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.label(&t_delete_confirm);
                    ui.horizontal(|ui| {
                        if ui.button(&t_confirm_yes).clicked() {
                            actions.push(PdfViewerAction::DeleteCurrentPage {
                                path: path.clone(),
                                page: (page_index + 1) as u32,
                            });
                            delete_confirm = false;
                        }
                        if ui.button(&t_confirm_cancel).clicked() {
                            delete_confirm = false;
                        }
                    });
                });
        }

        // Text prompt for a placed-but-unconfirmed sticky note / text
        // injection (§Fase 9) — highlight/underline commit immediately on
        // drag-release instead, since they don't carry a comment.
        let mut confirm_pending = false;
        let mut cancel_pending = false;
        if let Some(pending) = &pending_annotation {
            let title = match pending.kind {
                AnnotationKind::TextInjection => &t_annotate_text_prompt,
                _ => &t_annotate_sticky_prompt,
            };
            egui::Window::new(title.as_str())
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.text_edit_multiline(&mut pending_annotation_text);
                    ui.horizontal(|ui| {
                        if ui.button(&t_annotate_add).clicked() {
                            confirm_pending = true;
                        }
                        if ui.button(&t_annotate_cancel).clicked() {
                            cancel_pending = true;
                        }
                    });
                });
        }
        if confirm_pending && let Some(pending) = pending_annotation.take() {
            staged_annotations.push(Annotation {
                kind: pending.kind,
                page: pending.page,
                rect: pending.rect,
                color: (annotate_color[0], annotate_color[1], annotate_color[2]),
                contents: pending_annotation_text.clone(),
            });
            pending_annotation_text.clear();
        }
        if cancel_pending {
            pending_annotation = None;
            pending_annotation_text.clear();
        }

        if show_metadata_editor {
            egui::Window::new(&t_metadata_window_title)
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(&t_metadata_field_title);
                        ui.text_edit_singleline(&mut metadata_title);
                    });
                    ui.horizontal(|ui| {
                        ui.label(&t_metadata_field_author);
                        ui.text_edit_singleline(&mut metadata_author);
                    });
                    ui.horizontal(|ui| {
                        ui.label(&t_metadata_field_keywords);
                        ui.text_edit_singleline(&mut metadata_keywords);
                    });
                    if ui.button(&t_metadata_close).clicked() {
                        show_metadata_editor = false;
                    }
                });
        }

        // Carried into both Save and Export unconditionally (§Fase 9) —
        // rewriting the same values back when nothing was edited is a
        // harmless no-op, and this way the metadata editor never needs
        // its own separate "apply" plumbing.
        let current_metadata = DocumentMetadata {
            title: metadata_title.clone(),
            author: metadata_author.clone(),
            keywords: metadata_keywords.clone(),
        };

        if export_requested {
            actions.push(PdfViewerAction::ExportAs {
                path: path.clone(),
                annotations: staged_annotations.clone(),
                metadata: current_metadata.clone(),
            });
        }

        if show_save_confirm {
            egui::Window::new(&t_save)
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.label(&t_save_confirm);
                    ui.horizontal(|ui| {
                        if ui.button(&t_confirm_yes).clicked() {
                            actions.push(PdfViewerAction::SaveOver {
                                path: path.clone(),
                                annotations: staged_annotations.clone(),
                                metadata: current_metadata.clone(),
                            });
                            show_save_confirm = false;
                        }
                        if ui.button(&t_confirm_cancel).clicked() {
                            show_save_confirm = false;
                        }
                    });
                });
        }

        viewer.page_index = page_index;
        viewer.zoom_width = zoom_width;
        viewer.split_from = split_from;
        viewer.split_to = split_to;
        viewer.delete_confirm = delete_confirm;
        viewer.annotate_tool = annotate_tool;
        viewer.annotate_color = annotate_color;
        viewer.drag_start = drag_start;
        viewer.staged_annotations = staged_annotations;
        viewer.pending_annotation = pending_annotation;
        viewer.pending_annotation_text = pending_annotation_text;
        viewer.show_metadata_editor = show_metadata_editor;
        viewer.metadata_title = metadata_title;
        viewer.metadata_author = metadata_author;
        viewer.metadata_keywords = metadata_keywords;
        viewer.show_save_confirm = show_save_confirm;

        if back_requested {
            self.pdf_viewer = None;
        } else {
            self.pdf_viewer = Some(viewer);
        }

        for action in actions {
            self.apply_pdf_viewer_action(action);
        }
    }
}

/// Applies pending `annotations` + `metadata` on top of `source`, writing
/// the result to a fresh temp file and returning its path (§Fase 9) —
/// the common first step of both `PdfViewerAction::SaveOver` and
/// `ExportAs`, since both need the same baked-in content and only differ
/// in where it lands afterward. Metadata is always (re)written, even when
/// unedited, since it's a harmless idempotent rewrite of the same values
/// `open_pdf` loaded; annotating is skipped entirely when `annotations`
/// is empty (`pdf::annotator::add_annotations` rejects an empty batch).
fn bake_pdf_changes(
    source: &std::path::Path,
    annotations: &[Annotation],
    metadata: &DocumentMetadata,
) -> Result<PathBuf> {
    let stage = |suffix: &str| {
        std::env::temp_dir().join(format!("mnemonic-pdf-{}-{suffix}.pdf", Uuid::new_v4()))
    };

    let annotated_path = stage("annotated");
    let with_annotations: PathBuf = if annotations.is_empty() {
        source.to_path_buf()
    } else {
        pdf_annotator::add_annotations(source, annotations, &annotated_path)?;
        annotated_path.clone()
    };

    let staged_meta = stage("meta");
    pdf_editor::set_metadata(&with_annotations, metadata, &staged_meta)?;

    if with_annotations != source {
        let _ = std::fs::remove_file(&with_annotations); // best-effort cleanup of the intermediate stage
    }
    Ok(staged_meta)
}

/// Maps an `Annotation`'s PDF user-space `rect` (points, origin
/// bottom-left) to the on-screen rect it occupies over the rendered page
/// image at `img_rect` (§Fase 9) — the inverse of the drag-to-page-points
/// conversion `show_pdf_viewer` does when a new annotation is placed.
fn annotation_screen_rect(
    rect: (f32, f32, f32, f32),
    img_rect: egui::Rect,
    page_w: f32,
    page_h: f32,
) -> egui::Rect {
    let (x0, y0, x1, y1) = rect;
    let sx = img_rect.width() / page_w;
    let sy = img_rect.height() / page_h;
    let left = img_rect.min.x + x0 * sx;
    let right = img_rect.min.x + x1 * sx;
    // PDF y grows upward from the bottom; screen y grows downward from the top.
    let top = img_rect.min.y + (page_h - y1) * sy;
    let bottom = img_rect.min.y + (page_h - y0) * sy;
    egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom))
}

/// Default annotation size (pixels, in the rendered image's own screen
/// space) used when a drag was too short to count as "the user drew a
/// rectangle" (§Fase 9, see `MIN_ANNOTATION_DRAG_PX`) — so a single click
/// still places a usable annotation instead of a zero-size one.
fn default_annotation_size_px(kind: AnnotationKind) -> egui::Vec2 {
    match kind {
        AnnotationKind::StickyNote => egui::vec2(18.0, 18.0),
        AnnotationKind::TextInjection => egui::vec2(160.0, 22.0),
        AnnotationKind::Highlight | AnnotationKind::Underline => egui::vec2(80.0, 14.0),
    }
}

/// Draws one already-placed annotation as an overlay on the rendered page
/// (§Fase 9) — used both for `staged_annotations` (not yet saved to any
/// file) and, while a drag is in progress, the live rectangle being drawn.
/// `pdfium-render` will actually bake and render these into the bitmap
/// itself once saved (see `pdf::annotator`'s doc comment), so this
/// overlay only exists to show *pending*, not-yet-written changes.
fn draw_annotation_overlay(
    painter: &egui::Painter,
    screen_rect: egui::Rect,
    kind: AnnotationKind,
    color: (f32, f32, f32),
    contents: &str,
) {
    let color32 = egui::Color32::from_rgb(
        (color.0 * 255.0) as u8,
        (color.1 * 255.0) as u8,
        (color.2 * 255.0) as u8,
    );
    match kind {
        AnnotationKind::Highlight => {
            painter.rect_filled(
                screen_rect,
                egui::CornerRadius::ZERO,
                color32.gamma_multiply(0.35),
            );
        }
        AnnotationKind::Underline => {
            let y = screen_rect.bottom();
            painter.line_segment(
                [
                    egui::pos2(screen_rect.left(), y),
                    egui::pos2(screen_rect.right(), y),
                ],
                (2.0, color32),
            );
        }
        AnnotationKind::StickyNote => {
            painter.rect_filled(screen_rect, 2u8, color32);
            painter.rect_stroke(
                screen_rect,
                2u8,
                (1.0, egui::Color32::BLACK),
                egui::StrokeKind::Middle,
            );
        }
        AnnotationKind::TextInjection => {
            painter.rect_stroke(
                screen_rect,
                egui::CornerRadius::ZERO,
                (1.0, color32),
                egui::StrokeKind::Middle,
            );
            if !contents.is_empty() {
                painter.text(
                    screen_rect.left_top(),
                    egui::Align2::LEFT_TOP,
                    contents,
                    egui::FontId::proportional(12.0),
                    egui::Color32::BLACK,
                );
            }
        }
    }
}

/// Pre-translated labels shared by every rendered card, to avoid an
/// `i18n` lookup per card per frame.
struct CardStrings {
    pin: String,
    unpin: String,
    archive: String,
    unarchive: String,
    trash: String,
    restore: String,
    delete_permanent: String,
}

/// What the user asked for while looking at the grid. `app.rs::apply_grid_action`
/// applies these against the vault/disk after `show_grid`'s `egui` closures
/// have all returned — this function never touches disk itself.
enum GridAction {
    Open(PathBuf),
    TogglePin(Uuid),
    SetColor(Uuid, Option<String>),
    ToggleArchived(Uuid),
    Trash(Uuid),
    Restore(Uuid),
    DeletePermanently(Uuid),
    BatchArchive(Vec<Uuid>),
    BatchTrash(Vec<Uuid>),
    RenameTag(String, String),
    DeleteTag(String),
}

/// Renders one note card: title, snippet, checklist progress, tag chips,
/// and an always-visible action row (§3.1.2's per-card toolbar is
/// hover-only in the spec; always-visible is a simpler, equally
/// functional substitute in immediate-mode `egui`).
#[allow(clippy::too_many_arguments)]
fn render_note_card(
    ui: &mut egui::Ui,
    note: &Note,
    card: &CardStrings,
    selection_mode: bool,
    is_selected: bool,
    in_trash_view: bool,
    actions: &mut Vec<GridAction>,
    selected: &mut HashSet<Uuid>,
    confirm_delete: &mut Option<Uuid>,
) {
    let id = note.frontmatter.id;
    let fill = theme::color_for(note.frontmatter.color.as_deref());
    let mut frame = egui::Frame::group(ui.style());
    if let Some(color) = fill {
        frame = frame.fill(color);
    }

    frame.show(ui, |ui| {
        ui.set_width(220.0);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                if selection_mode {
                    let mut checked = is_selected;
                    if ui.checkbox(&mut checked, "").changed() {
                        if checked {
                            selected.insert(id);
                        } else {
                            selected.remove(&id);
                        }
                    }
                }
                if note.frontmatter.pinned {
                    ui.label("📌");
                }
                ui.strong(note.frontmatter.title.as_str());
            });

            ui.label(query::snippet(&note.body, 140));

            if let Some((done, total)) = note.checklist_progress() {
                ui.label(format!("✅ {done}/{total}"));
            }

            if !note.frontmatter.tags.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for tag in &note.frontmatter.tags {
                        ui.colored_label(theme::tag_color(tag), format!("#{tag}"));
                    }
                });
            }

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("📂").on_hover_text("Buka").clicked() {
                    actions.push(GridAction::Open(note.path.clone()));
                }

                if in_trash_view {
                    if ui.button(&card.restore).clicked() {
                        actions.push(GridAction::Restore(id));
                    }
                    if ui.button(&card.delete_permanent).clicked() {
                        *confirm_delete = Some(id);
                    }
                } else {
                    let pin_label = if note.frontmatter.pinned {
                        &card.unpin
                    } else {
                        &card.pin
                    };
                    if ui.button(pin_label).clicked() {
                        actions.push(GridAction::TogglePin(id));
                    }

                    ui.menu_button("🎨", |ui| {
                        for (name, color) in theme::PALETTE {
                            if ui.add(egui::Button::new("  ").fill(*color)).clicked() {
                                actions.push(GridAction::SetColor(id, Some((*name).to_string())));
                            }
                        }
                        if ui.button("✕").clicked() {
                            actions.push(GridAction::SetColor(id, None));
                        }
                    });

                    let archive_label = if note.frontmatter.archived {
                        &card.unarchive
                    } else {
                        &card.archive
                    };
                    if ui.button(archive_label).clicked() {
                        actions.push(GridAction::ToggleArchived(id));
                    }
                    if ui.button(&card.trash).clicked() {
                        actions.push(GridAction::Trash(id));
                    }
                }
            });
        });
    });
}

fn locales_dir() -> PathBuf {
    // Fase 1: locales/ ships next to the project root / executable.
    PathBuf::from("locales")
}

/// Liquid-Glass version of the note card. Full-width within its masonry
/// column (width is set by the parent `allocate_ui`), variable height.
#[allow(clippy::too_many_arguments)]
fn render_note_card_glass(
    ui: &mut egui::Ui,
    note: &Note,
    card: &CardStrings,
    selection_mode: bool,
    is_selected: bool,
    in_trash_view: bool,
    actions: &mut Vec<GridAction>,
    selected: &mut HashSet<Uuid>,
    confirm_delete: &mut Option<Uuid>,
) {
    let id = note.frontmatter.id;
    let is_canvas = note.is_canvas();

    // Build the card frame: use note colour if set, else default card frame.
    let card_fill =
        theme::color_for(note.frontmatter.color.as_deref()).unwrap_or(theme::BG_CARD_DARK);
    let border_color = theme::color_solid_for(note.frontmatter.color.as_deref())
        .map(|c| egui::Color32::from_rgba_premultiplied(c.r(), c.g(), c.b(), 100))
        .unwrap_or(theme::BORDER_SUBTLE);

    let frame = egui::Frame {
        inner_margin: egui::Margin::same(12),
        outer_margin: egui::Margin::ZERO,
        corner_radius: egui::CornerRadius::same(theme::ROUNDING_MD),
        shadow: egui::Shadow {
            offset: [0, 4],
            blur: 10,
            spread: 0,
            color: egui::Color32::from_black_alpha(60),
        },
        fill: card_fill,
        stroke: egui::Stroke::new(1.0, border_color),
    };

    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical(|ui| {
            // ── Header row ──
            ui.horizontal(|ui| {
                if selection_mode {
                    let mut checked = is_selected;
                    if ui.checkbox(&mut checked, "").changed() {
                        if checked {
                            selected.insert(id);
                        } else {
                            selected.remove(&id);
                        }
                    }
                }
                if note.frontmatter.pinned {
                    ui.label(
                        egui::RichText::new(egui_icons::icons::ICON_KEEP.codepoint)
                            .size(13.0)
                            .color(theme::ACCENT_ORANGE),
                    );
                }
                // Note type badge
                if is_canvas {
                    ui.label(
                        egui::RichText::new(egui_icons::icons::ICON_DRAW.codepoint)
                            .size(13.0)
                            .color(theme::ACCENT_BLUE),
                    );
                    ui.label(
                        egui::RichText::new("Kanvas")
                            .size(11.0)
                            .color(theme::ACCENT_BLUE),
                    );
                } else {
                    ui.label(
                        egui::RichText::new(egui_icons::icons::ICON_DESCRIPTION.codepoint)
                            .size(13.0)
                            .color(theme::TEXT_MUTED),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Quick open button
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(egui_icons::icons::ICON_OPEN_IN_NEW.codepoint)
                                    .color(theme::TEXT_SECONDARY)
                                    .size(13.0),
                            )
                            .frame(false),
                        )
                        .on_hover_text("Buka")
                        .clicked()
                    {
                        actions.push(GridAction::Open(note.path.clone()));
                    }
                });
            });

            ui.add_space(4.0);

            // ── Title ──
            let title_color = theme::TEXT_PRIMARY;
            let title_resp = ui.add(
                egui::Label::new(
                    egui::RichText::new(note.frontmatter.title.as_str())
                        .size(14.0)
                        .strong()
                        .color(title_color),
                )
                .sense(egui::Sense::click()),
            );
            if title_resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                actions.push(GridAction::Open(note.path.clone()));
            }

            // ── Body snippet ──
            if is_canvas {
                let canvas_doc = CanvasDocument::from_markdown_body(&note.frontmatter.title, &note.body);
                let summary = canvas_doc.summary_text();
                let text_content = canvas_doc.extract_searchable_text();
                let snippet = query::snippet(&text_content, 120);

                ui.add_space(3.0);
                ui.label(
                    egui::RichText::new(format!("🎨 {summary}"))
                        .size(11.5)
                        .color(theme::ACCENT_BLUE),
                );

                if !snippet.is_empty() {
                    ui.add_space(3.0);
                    ui.label(
                        egui::RichText::new(&snippet)
                            .size(12.0)
                            .color(theme::TEXT_SECONDARY),
                    );
                }
            } else {
                let snippet = query::snippet(&note.body, 120);
                if !snippet.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(&snippet)
                            .size(12.0)
                            .color(theme::TEXT_SECONDARY),
                    );
                }
            }

            // ── Checklist progress ──
            if let Some((done, total)) = note.checklist_progress() {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(format!(
                        "{} {done}/{total}",
                        egui_icons::icons::ICON_CHECK_CIRCLE.codepoint
                    ))
                    .size(11.0)
                    .color(theme::ACCENT_GREEN),
                );
            }

            // ── Tags ──
            if !note.frontmatter.tags.is_empty() {
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 3.0);
                    for tag in &note.frontmatter.tags {
                        let color = theme::tag_color(tag);
                        ui::tag_chip_frame(color).show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(format!("#{tag}"))
                                    .size(10.5)
                                    .color(color),
                            );
                        });
                    }
                });
            }

            // ── Action row ──
            ui.add_space(8.0);
            ui.add(egui::Separator::default().spacing(0.0));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);

                if in_trash_view {
                    let restore_btn = egui::Button::new(
                        egui::RichText::new(&card.restore)
                            .size(11.0)
                            .color(theme::ACCENT_BLUE),
                    )
                    .frame(false);
                    if ui.add(restore_btn).clicked() {
                        actions.push(GridAction::Restore(id));
                    }
                    let del_btn = egui::Button::new(
                        egui::RichText::new(&card.delete_permanent)
                            .size(11.0)
                            .color(theme::GLASS_ERROR),
                    )
                    .frame(false);
                    if ui.add(del_btn).clicked() {
                        *confirm_delete = Some(id);
                    }
                } else {
                    // Pin toggle
                    let pin_icon = if note.frontmatter.pinned {
                        egui_icons::icons::ICON_KEEP.codepoint
                    } else {
                        egui_icons::icons::ICON_PUSH_PIN.codepoint
                    };
                    let pin_tip = if note.frontmatter.pinned {
                        &card.unpin
                    } else {
                        &card.pin
                    };
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(pin_icon)
                                    .size(12.0)
                                    .color(if note.frontmatter.pinned {
                                        theme::ACCENT_ORANGE
                                    } else {
                                        theme::TEXT_MUTED
                                    }),
                            )
                            .frame(false),
                        )
                        .on_hover_text(pin_tip.as_str())
                        .clicked()
                    {
                        actions.push(GridAction::TogglePin(id));
                    }

                    // Colour picker menu
                    ui.menu_button(
                        egui::RichText::new(egui_icons::icons::ICON_PALETTE.codepoint)
                            .size(12.0)
                            .color(theme::TEXT_SECONDARY),
                        |ui| {
                            ui.horizontal_wrapped(|ui| {
                                for (name, _) in theme::PALETTE_SOLID {
                                    let solid = theme::color_solid_for(Some(name)).unwrap();
                                    if ui
                                        .add(
                                            egui::Button::new("  ")
                                                .fill(solid)
                                                .corner_radius(egui::CornerRadius::same(4)),
                                        )
                                        .clicked()
                                    {
                                        actions.push(GridAction::SetColor(
                                            id,
                                            Some((*name).to_string()),
                                        ));
                                    }
                                }
                                if ui.button("✕").clicked() {
                                    actions.push(GridAction::SetColor(id, None));
                                }
                            });
                        },
                    );

                    // Archive toggle
                    let arch_label = if note.frontmatter.archived {
                        &card.unarchive
                    } else {
                        &card.archive
                    };
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(arch_label)
                                    .size(11.0)
                                    .color(theme::TEXT_MUTED),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        actions.push(GridAction::ToggleArchived(id));
                    }

                    // Trash
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(&card.trash)
                                    .size(11.0)
                                    .color(theme::TEXT_MUTED),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        actions.push(GridAction::Trash(id));
                    }
                }
            });
        });
    });
}

/// Renders a PDF document card in the unified masonry grid.
fn render_pdf_card_glass(
    ui: &mut egui::Ui,
    path: &std::path::Path,
    open_pdf: &mut Option<std::path::PathBuf>,
) {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Document.pdf".to_string());

    // File size (best-effort)
    let size_str = std::fs::metadata(path)
        .map(|m| {
            let bytes = m.len();
            if bytes >= 1_048_576 {
                format!("{:.1} MB", bytes as f64 / 1_048_576.0)
            } else {
                format!("{:.0} KB", bytes as f64 / 1024.0)
            }
        })
        .unwrap_or_default();

    let pdf_accent = egui::Color32::from_rgb(239, 68, 68); // red accent for PDF
    let card_fill = egui::Color32::from_rgba_premultiplied(45, 15, 15, 120);
    let border_color = egui::Color32::from_rgba_premultiplied(239, 68, 68, 80);

    let frame = egui::Frame {
        inner_margin: egui::Margin::same(12),
        outer_margin: egui::Margin::ZERO,
        corner_radius: egui::CornerRadius::same(theme::ROUNDING_MD),
        shadow: egui::Shadow {
            offset: [0, 4],
            blur: 10,
            spread: 0,
            color: egui::Color32::from_black_alpha(60),
        },
        fill: card_fill,
        stroke: egui::Stroke::new(1.0, border_color),
    };

    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical(|ui| {
            // PDF icon + badge row
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(egui_icons::icons::ICON_PICTURE_AS_PDF.codepoint)
                        .size(20.0)
                        .color(pdf_accent),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // PDF badge
                    egui::Frame {
                        inner_margin: egui::Margin::symmetric(6, 2),
                        outer_margin: egui::Margin::ZERO,
                        corner_radius: egui::CornerRadius::same(4),
                        shadow: egui::Shadow::NONE,
                        fill: egui::Color32::from_rgba_premultiplied(239, 68, 68, 40),
                        stroke: egui::Stroke::new(
                            1.0,
                            egui::Color32::from_rgba_premultiplied(239, 68, 68, 100),
                        ),
                    }
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new("PDF")
                                .size(10.0)
                                .strong()
                                .color(pdf_accent),
                        );
                    });
                });
            });

            ui.add_space(6.0);

            // File name
            let display_name = if name.len() > 30 {
                format!("{}…", &name[..27])
            } else {
                name.clone()
            };
            ui.label(
                egui::RichText::new(&display_name)
                    .size(13.0)
                    .strong()
                    .color(theme::TEXT_PRIMARY),
            );

            // File size
            if !size_str.is_empty() {
                ui.label(
                    egui::RichText::new(&size_str)
                        .size(11.0)
                        .color(theme::TEXT_MUTED),
                );
            }

            // ── Action row ──
            ui.add_space(8.0);
            ui.add(egui::Separator::default().spacing(0.0));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let open_btn = egui::Button::new(
                    egui::RichText::new("Buka")
                        .size(11.5)
                        .color(theme::ACCENT_BLUE),
                )
                .frame(false);
                if ui.add(open_btn).clicked() {
                    *open_pdf = Some(path.to_path_buf());
                }
            });
        });
    });
}

impl eframe::App for MnemonicApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Poll the file watcher once per frame; debouncing happens inside
        // VaultWatcher itself (§6 mitigation for watcher event storms).
        let needs_rescan = self
            .watcher
            .as_mut()
            .map(|w| w.poll_rescan_needed())
            .unwrap_or(false);
        if needs_rescan {
            self.rescan_and_reindex();
            ui.ctx().request_repaint();
        }

        // Autosave poll (§3.2.4: idle debounce 500ms-1s). Scoped so the
        // `self.editor` borrow ends before `rescan_and_reindex`/        // Fase 7 background-worker polling: writes chunk/embedding
        // results into the SQLite cache, and dispatches search/chat
        // query-embedding + generation events to whichever request is
        // pending.
        self.poll_indexer_results();
        self.poll_search_and_chat(ui.ctx());

        // Apply Shapr3D / DUCAD theme every frame.
        ui::apply_theme(ui.ctx(), self.theme_mode);

        let vault_open = self.vault.is_some();
        let in_editor = self.editor.is_some();
        let in_pdf = self.pdf_viewer.is_some();

        // Global Command Palette keyboard shortcut (Cmd+K / Ctrl+K)
        if ui.input(|i| (i.modifiers.command || i.modifiers.ctrl) && i.key_pressed(egui::Key::K)) {
            self.command_palette.toggle();
        }

        // ─── Floating Top Bar (Shapr3D / DUCAD Style) ────────────────────────
        if !in_editor && !in_pdf {
            let screen_rect = ui.ctx().viewport_rect();
            let topbar_margin_x = 12.0;
            let topbar_margin_y = 8.0;
            let topbar_pos = screen_rect.min + egui::vec2(topbar_margin_x, topbar_margin_y);
            let topbar_width = (screen_rect.width() - (topbar_margin_x * 2.0)).max(200.0);

            egui::Area::new(egui::Id::new("mnemonic_floating_topbar_area"))
                .fixed_pos(topbar_pos)
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    ui.set_width(topbar_width);

                    let vault_name = self
                        .vault
                        .as_ref()
                        .and_then(|v| v.root.file_name())
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();

                    let active_tab = match self.view {
                        View::Notes => {
                            if matches!(self.doc_filter, DocFilter::WhiteboardsOnly) {
                                ui::TopBarNavTab::Canvas
                            } else {
                                ui::TopBarNavTab::Notes
                            }
                        }
                        View::Canvas => ui::TopBarNavTab::Canvas,
                        View::Search => ui::TopBarNavTab::Search,
                        View::Chat => ui::TopBarNavTab::Chat,
                    };

                    let note_count = self
                        .vault
                        .as_ref()
                        .map(|v| v.notes.iter().filter(|n| !n.frontmatter.trashed).count())
                        .unwrap_or(0);
                    let pdf_count = self.pdf_documents.len();

                    let mut topbar_state = ui::TopBarState {
                        vault_name,
                        vault_open,
                        active_tab,
                        sidebar_open: self.sidebar_open,
                        theme_mode: self.theme_mode,
                        active_locale: self.locales.active_locale().to_string(),
                        quick_capture_text: self.quick_capture_text.clone(),
                        icon_size: 16.0,
                        note_count,
                        pdf_count,
                    };

                    if let Some(top_event) = ui::TopBar::show(ui, &mut topbar_state) {
                        match top_event {
                            ui::TopBarEvent::SelectTab(tab) => {
                                match tab {
                                    ui::TopBarNavTab::Notes => {
                                        self.view = View::Notes;
                                        if matches!(self.doc_filter, DocFilter::WhiteboardsOnly) {
                                            self.doc_filter = DocFilter::All;
                                        }
                                    }
                                    ui::TopBarNavTab::Canvas => {
                                        self.view = View::Notes;
                                        self.doc_filter = DocFilter::WhiteboardsOnly;
                                    }
                                    ui::TopBarNavTab::Search => {
                                        self.view = View::Search;
                                    }
                                    ui::TopBarNavTab::Chat => {
                                        self.view = View::Chat;
                                    }
                                }
                            }
                            ui::TopBarEvent::ToggleSidebar => {
                                self.sidebar_open = !self.sidebar_open;
                            }
                            ui::TopBarEvent::OpenVaultPicker => {
                                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                                    match Vault::open(folder) {
                                        Ok(v) => self.activate_vault(v),
                                        Err(e) => self.report_error("error-context-open-vault", e),
                                    }
                                }
                            }
                            ui::TopBarEvent::CreateNote(title) => {
                                if let Some(vault) = self.vault.as_mut() {
                                    let note_title = if title.is_empty() {
                                        "Catatan Baru"
                                    } else {
                                        &title
                                    };
                                    match Note::create(&vault.root, note_title, "") {
                                        Ok(note) => {
                                            self.rescan_and_reindex();
                                            self.open_note(note);
                                        }
                                        Err(e) => self.report_error("error-context-create-note", e),
                                    }
                                }
                            }
                            ui::TopBarEvent::OpenCommandPalette => {
                                self.command_palette.open();
                            }
                            ui::TopBarEvent::ToggleTheme => {
                                self.theme_mode = self.theme_mode.toggled();
                            }
                            ui::TopBarEvent::SetLanguage(lang) => {
                                self.locales.set_active(&lang);
                            }
                            ui::TopBarEvent::ImportPdf => {
                                if let Some(file) =
                                    rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file()
                                {
                                    self.import_pdf(file);
                                }
                            }
                            ui::TopBarEvent::ManageLabels => {
                                self.show_label_manager = true;
                            }
                            ui::TopBarEvent::NewCanvas => {
                                if let Some(vault) = self.vault.as_mut() {
                                    match Note::create_canvas(&vault.root, "Kanvas Baru") {
                                        Ok(note) => {
                                            self.rescan_and_reindex();
                                            self.open_note(note);
                                        }
                                        Err(e) => self.report_error("error-context-create-note", e),
                                    }
                                }
                            }
                            _ => {}
                        }
                    }

                    self.quick_capture_text = topbar_state.quick_capture_text;
                });
        }

        // ─── Floating Sidebar Slide-Over Drawer ──────────────────────────────
        if vault_open && !in_editor && !in_pdf {
            let notes_for_sidebar: Vec<Note> = self
                .vault
                .as_ref()
                .map(|v| v.notes.clone())
                .unwrap_or_default();
            let all_tags = tags::all_tags(&notes_for_sidebar);
            let total_notes = notes_for_sidebar
                .iter()
                .filter(|n| !n.frontmatter.archived && !n.frontmatter.trashed)
                .count();
            let total_archived = notes_for_sidebar
                .iter()
                .filter(|n| n.frontmatter.archived && !n.frontmatter.trashed)
                .count();
            let total_trashed = notes_for_sidebar
                .iter()
                .filter(|n| n.frontmatter.trashed)
                .count();

            let sidebar_filter = match &self.doc_filter {
                DocFilter::All => ui::SidebarDocFilter::All,
                DocFilter::NotesOnly => ui::SidebarDocFilter::NotesOnly,
                DocFilter::WhiteboardsOnly => ui::SidebarDocFilter::WhiteboardsOnly,
                DocFilter::PdfsOnly => ui::SidebarDocFilter::PdfsOnly,
                DocFilter::Archived => ui::SidebarDocFilter::Archived,
                DocFilter::Trashed => ui::SidebarDocFilter::Trashed,
                DocFilter::Tag(t) => ui::SidebarDocFilter::Tag(t.clone()),
            };

            let sidebar_state = ui::SidebarState {
                is_open: self.sidebar_open,
                current_filter: sidebar_filter,
                all_tags,
                pdf_documents: self.pdf_documents.clone(),
                total_notes,
                total_pdfs: self.pdf_documents.len(),
                total_whiteboards: if self.standalone_canvas.is_some() {
                    1
                } else {
                    0
                },
                total_archived,
                total_trashed,
            };

            if let Some(side_event) = ui::SidebarDrawer::show(ui.ctx(), &sidebar_state) {
                match side_event {
                    ui::SidebarEvent::SelectFilter(f) => {
                        self.doc_filter = match f {
                            ui::SidebarDocFilter::All => DocFilter::All,
                            ui::SidebarDocFilter::NotesOnly => DocFilter::NotesOnly,
                            ui::SidebarDocFilter::WhiteboardsOnly => DocFilter::WhiteboardsOnly,
                            ui::SidebarDocFilter::PdfsOnly => DocFilter::PdfsOnly,
                            ui::SidebarDocFilter::Archived => DocFilter::Archived,
                            ui::SidebarDocFilter::Trashed => DocFilter::Trashed,
                            ui::SidebarDocFilter::Tag(t) => DocFilter::Tag(t),
                        };
                    }
                    ui::SidebarEvent::OpenPdf(path) => {
                        self.open_pdf(path);
                    }
                    ui::SidebarEvent::ImportPdf => {
                        if let Some(file) =
                            rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file()
                        {
                            self.import_pdf(file);
                        }
                    }
                    ui::SidebarEvent::ManageLabels => {
                        self.show_label_manager = true;
                    }
                    ui::SidebarEvent::CloseSidebar => {
                        self.sidebar_open = false;
                    }
                }
            }
        }

        // ─── Central Panel ───────────────────────────────────────────────────
        egui::CentralPanel::default().show(ui, |ui| {
            self.show_status_banner(ui);

            if self.vault.is_none() {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(
                        egui::RichText::new(egui_icons::icons::ICON_FOLDER_OPEN.codepoint)
                            .size(48.0)
                            .color(theme::ACCENT_BLUE),
                    );
                    ui.add_space(12.0);
                    ui.label(
                        egui::RichText::new(self.t("vault-select-prompt"))
                            .size(15.0)
                            .color(theme::TEXT_SECONDARY),
                    );
                    ui.add_space(16.0);
                    let pick_btn = egui::Button::new(
                        egui::RichText::new(format!(
                            "{}  {}",
                            egui_icons::icons::ICON_FOLDER.codepoint,
                            self.t("vault-pick-folder")
                        ))
                        .color(egui::Color32::WHITE)
                        .size(14.0),
                    )
                    .fill(theme::ACCENT_BLUE)
                    .corner_radius(egui::CornerRadius::same(theme::ROUNDING_MD));
                    if ui.add(pick_btn).clicked() {
                        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                            match crate::notes::Vault::open(folder) {
                                Ok(vault) => self.activate_vault(vault),
                                Err(e) => self.report_error("error-context-open-vault", e),
                            }
                        }
                    }
                });
                return;
            }

            if self.editor.is_some() {
                self.show_editor(ui);
                return;
            }

            if self.pdf_viewer.is_some() {
                self.show_pdf_viewer(ui);
                return;
            }

            match self.view {
                View::Notes => self.show_grid(ui),
                View::Canvas => self.show_standalone_canvas(ui),
                View::Search => self.show_search(ui),
                View::Chat => self.show_chat(ui),
            }
        });

        // ─── Floating Command Palette Modal (⌘K) ─────────────────────────────
        self.show_command_palette_modal(ui.ctx());
    }
}
