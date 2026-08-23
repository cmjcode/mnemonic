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

use egui_commonmark::CommonMarkCache;
use uuid::Uuid;

use crate::core::search::{self, SearchHit};
use crate::core::{DocumentChunk, IndexStore, IndexingWorker};
use crate::i18n::LocaleManager;
use crate::llm::{self, GenerationEvent, GenerationWorker};
use crate::markdown::editor::{
    char_index_to_byte_offset, slash_menu_triggered, slash_templates, wikilink_autocomplete_query,
};
use crate::markdown::{self, wikilink, EditorMode, MarkdownEditor, WikilinkIndex};
use crate::notes::query::{self, GridFilter, SortMode};
use crate::notes::{tags, trash, Note, Vault, VaultWatcher};
use crate::ui::theme;

/// How many top-ranked chunks to retrieve for the Search tab / Chat tab
/// respectively (§Fase 7). Search shows more candidates than chat's RAG
/// context window since a human is skimming results, while chat feeds
/// straight into a token-bounded LLM prompt.
const SEARCH_TOP_K: usize = 10;
const CHAT_TOP_K: usize = 5;

/// Which top-level tab is showing in the central panel (only relevant
/// while no note is open for editing — `show_editor` always takes over
/// regardless of `view`, same as before Fase 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Notes,
    Search,
    Chat,
}

/// One rendered bubble in the Chat tab's transcript.
struct ChatMessage {
    role: ChatRole,
    text: String,
    /// Source file paths the assistant grounded its reply on (§3.4 point
    /// 4's citations) — empty for user messages and for an assistant
    /// reply that found no relevant context.
    citations: Vec<String>,
}

#[derive(PartialEq, Eq)]
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

    fn assistant(text: String, citations: Vec<String>) -> ChatMessage {
        ChatMessage {
            role: ChatRole::Assistant,
            text,
            citations,
        }
    }
}

pub struct LontarApp {
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
    sort_mode: SortMode,
    search_text: String,
    selection_mode: bool,
    selected: HashSet<Uuid>,
    show_label_manager: bool,
    tag_rename: Option<(String, String)>,
    confirm_delete: Option<Uuid>,

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
}

impl LontarApp {
    pub fn new() -> LontarApp {
        let locales_dir = locales_dir();
        let locales = LocaleManager::load(&locales_dir);

        let mut app = LontarApp {
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
            sort_mode: SortMode::Modified,
            search_text: String::new(),
            selection_mode: false,
            selected: HashSet::new(),
            show_label_manager: false,
            tag_rename: None,
            confirm_delete: None,
            view: View::Notes,
            search_query: String::new(),
            search_pending_id: None,
            search_results: Vec::new(),
            search_status: String::new(),
            chat_input: String::new(),
            chat_messages: Vec::new(),
            chat_pending_embed_id: None,
            chat_pending_gen_id: None,
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
        let Some(index) = self.index.as_mut() else { return };
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
                            self.chat_messages.push(ChatMessage::assistant(msg, Vec::new()));
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
        let Some(index) = self.index.as_ref() else { return };
        let chunks = match index.all_chunks() {
            Ok(c) => c,
            Err(e) => {
                self.search_status = format!("{}: {e:#}", self.t("search-error"));
                return;
            }
        };
        let semantic = search::semantic_search(embedding, &chunks, SEARCH_TOP_K, llm::SIMILARITY_THRESHOLD);
        let notes: Vec<Note> = self.vault.as_ref().map(|v| v.notes.clone()).unwrap_or_default();
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

        let notes: Vec<Note> = self.vault.as_ref().map(|v| v.notes.clone()).unwrap_or_default();
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
        if text.is_empty() || self.chat_pending_embed_id.is_some() || self.chat_pending_gen_id.is_some() {
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
        let citations: Vec<String> = context
            .iter()
            .map(|c| match c.page_num {
                Some(p) => format!("{} (Halaman {p})", c.file_path.display()),
                None => c.file_path.display().to_string(),
            })
            .collect();
        let prompt = llm::build_rag_prompt(&context, &question);

        self.chat_messages.push(ChatMessage::assistant(String::new(), citations));

        if let Some(generator) = &self.generator {
            self.chat_pending_gen_id = Some(generator.submit(prompt, llm::DEFAULT_MAX_TOKENS));
        }
    }

    fn t(&self, key: &str) -> String {
        self.locales.t(key, &[])
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
            Err(e) => log::warn!("app: failed to auto-create note '{title}' from wikilink: {e}"),
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
        let t_word_count = self
            .locales
            .t("editor-word-count", &[("count", &editor.word_count().to_string())]);
        let t_reading_time = self.locales.t(
            "editor-reading-time",
            &[("minutes", &editor.reading_time_minutes().to_string())],
        );

        let outline = markdown::renderer::headings(&editor.note.body);
        let wikilink_index = self.vault.as_ref().map(|v| WikilinkIndex::build(&v.notes));
        let backlink_titles: Vec<String> = self
            .vault
            .as_ref()
            .map(|v| {
                wikilink::backlinks_for(&editor.note.frontmatter.title, editor.note.frontmatter.id, &v.notes)
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

        ui.horizontal(|ui| {
            if ui.button(t_back.as_str()).clicked() {
                close_requested = true;
            }
            ui.heading(editor.note.frontmatter.title.as_str());
        });

        ui.horizontal(|ui| {
            ui.selectable_value(&mut editor.mode, EditorMode::Source, t_source.as_str());
            ui.selectable_value(&mut editor.mode, EditorMode::LivePreview, t_live_preview.as_str());
            ui.selectable_value(&mut editor.mode, EditorMode::Reading, t_reading.as_str());
            ui.separator();
            if ui.button(t_undo.as_str()).clicked() {
                editor.undo();
            }
            if ui.button(t_redo.as_str()).clicked() {
                editor.redo();
            }
        });

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

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| match editor.mode {
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
                                        new_body.replace_range(byte - query.len()..byte, &format!("{title}]]"));
                                        editor.set_body(new_body);
                                    }
                                }
                            });
                        }
                    }
                }
                EditorMode::LivePreview | EditorMode::Reading => {
                    let outcome = markdown::renderer::render(ui, cache, &editor.note.body);
                    if let Some(new_body) = outcome.updated_body {
                        editor.set_body(new_body);
                    }
                    if let Some(title) = outcome.clicked_wikilink {
                        navigate_to = Some(title);
                    }
                }
            });
        });

        if let Some(slug) = scroll_to_slug {
            self.markdown_cache.scroll_to_id_target_mut().replace(slug);
        }

        if close_requested {
            if editor.is_dirty() {
                if let Err(e) = editor.autosave() {
                    log::warn!("app: failed to save note on close: {e}");
                }
            }
            self.rescan_and_reindex();
            self.reindex_note(&editor.note);
        } else if let Some(title) = navigate_to {
            if editor.is_dirty() {
                if let Err(e) = editor.autosave() {
                    log::warn!("app: failed to save note before navigating: {e}");
                }
            }
            self.rescan_and_reindex();
            self.reindex_note(&editor.note);
            self.navigate_wikilink(&title);
        } else {
            self.editor = Some(editor);
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
            GridAction::TogglePin(id) => self.mutate_note(id, |n| n.frontmatter.pinned = !n.frontmatter.pinned),
            GridAction::SetColor(id, color) => self.mutate_note(id, |n| n.frontmatter.color = color),
            GridAction::ToggleArchived(id) => {
                self.mutate_note(id, |n| n.frontmatter.archived = !n.frontmatter.archived)
            }
            GridAction::Trash(id) => self.move_note(id, |n, root| n.move_to_trash(root)),
            GridAction::Restore(id) => self.move_note(id, |n, root| n.restore_from_trash(root)),
            GridAction::DeletePermanently(id) => {
                let Some(vault) = self.vault.as_ref() else { return };
                let Some(note) = vault.notes.iter().find(|n| n.frontmatter.id == id).cloned() else {
                    return;
                };
                if let Err(e) = note.delete_permanently() {
                    log::warn!("app: failed to permanently delete note: {e}");
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
                let Some(vault) = self.vault.as_mut() else { return };
                for i in tags::rename_tag(&mut vault.notes, &old, &new) {
                    if let Err(e) = vault.notes[i].save() {
                        log::warn!("app: failed to save note after tag rename: {e}");
                    }
                }
                self.rescan_and_reindex();
            }
            GridAction::DeleteTag(tag) => {
                let Some(vault) = self.vault.as_mut() else { return };
                for i in tags::remove_tag(&mut vault.notes, &tag) {
                    if let Err(e) = vault.notes[i].save() {
                        log::warn!("app: failed to save note after tag delete: {e}");
                    }
                }
                self.rescan_and_reindex();
            }
        }
    }

    /// Loads the note `id`, applies `f` to its frontmatter, saves, and
    /// re-syncs the index. Used for the simple single-field toggles (pin,
    /// color, archive).
    fn mutate_note(&mut self, id: Uuid, f: impl FnOnce(&mut Note)) {
        let Some(vault) = self.vault.as_ref() else { return };
        let Some(mut note) = vault.notes.iter().find(|n| n.frontmatter.id == id).cloned() else {
            return;
        };
        f(&mut note);
        if let Err(e) = note.save() {
            log::warn!("app: failed to save note: {e}");
            return;
        }
        self.rescan_and_reindex();
        self.reindex_note(&note);
    }

    /// Loads the note `id` and applies a move operation (trash/restore)
    /// that needs the vault root, then re-syncs the index.
    fn move_note(&mut self, id: Uuid, f: impl FnOnce(Note, &std::path::Path) -> anyhow::Result<Note>) {
        let Some(vault) = self.vault.as_ref() else { return };
        let Some(note) = vault.notes.iter().find(|n| n.frontmatter.id == id).cloned() else {
            return;
        };
        let root = vault.root.clone();
        if let Err(e) = f(note, &root) {
            log::warn!("app: failed to move note: {e}");
            return;
        }
        self.rescan_and_reindex();
    }

    /// The Notes Grid (§3.1.2/§3.1.3): sidebar filters + tags, search,
    /// sort, the card grid, multi-select batch actions, and the label
    /// manager. Everything the `egui` closures below touch is a plain
    /// local (cloned app state, mutated locally, written back at the
    /// end) — same reasoning as `show_editor`'s doc comment. Note: this
    /// uses a flex-wrap layout rather than true variable-height masonry
    /// packing (out of reach for a reasonable effort in immediate-mode
    /// `egui`), and manual drag-to-reorder is deferred past this phase.
    fn show_grid(&mut self, ui: &mut egui::Ui) {
        let t_new = self.t("notes-new");
        let t_sidebar_all = self.t("sidebar-all");
        let t_sidebar_archived = self.t("sidebar-archived");
        let t_sidebar_trash = self.t("sidebar-trash");
        let t_sidebar_tags = self.t("sidebar-tags");
        let t_manage_tags = self.t("sidebar-manage-tags");
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
        let t_confirm_cancel = self.t("confirm-cancel");
        let t_tag_manager_title = self.t("tag-manager-title");
        let t_tag_rename = self.t("tag-rename");
        let t_tag_delete = self.t("tag-delete");
        let card = CardStrings {
            pin: self.t("notes-pin"),
            unpin: self.t("notes-unpin"),
            archive: self.t("card-archive"),
            unarchive: self.t("card-unarchive"),
            trash: self.t("card-trash"),
            restore: self.t("card-restore"),
            delete_permanent: self.t("card-delete-permanent"),
        };

        let notes: Vec<Note> = self.vault.as_ref().map(|v| v.notes.clone()).unwrap_or_default();
        let all_tags = tags::all_tags(&notes);

        let mut filter = self.grid_filter.clone();
        let mut sort_mode = self.sort_mode;
        let mut search_text = self.search_text.clone();
        let mut selection_mode = self.selection_mode;
        let mut selected = self.selected.clone();
        let mut show_label_manager = self.show_label_manager;
        let mut tag_rename = self.tag_rename.clone();
        let mut confirm_delete = self.confirm_delete;
        let mut actions: Vec<GridAction> = Vec::new();

        egui::Panel::left("grid_sidebar")
            .resizable(true)
            .default_size(180.0)
            .show(ui, |ui| {
                if ui.selectable_label(filter == GridFilter::All, &t_sidebar_all).clicked() {
                    filter = GridFilter::All;
                }
                if ui
                    .selectable_label(filter == GridFilter::Archived, &t_sidebar_archived)
                    .clicked()
                {
                    filter = GridFilter::Archived;
                }
                if ui
                    .selectable_label(filter == GridFilter::Trashed, &t_sidebar_trash)
                    .clicked()
                {
                    filter = GridFilter::Trashed;
                }

                ui.separator();
                ui.label(&t_sidebar_tags);
                for (tag, count) in &all_tags {
                    let is_selected = matches!(&filter, GridFilter::Tag(t) if t.eq_ignore_ascii_case(tag));
                    let mut label = egui::RichText::new(format!("#{tag} ({count})"));
                    label = label.color(theme::tag_color(tag));
                    if ui.selectable_label(is_selected, label).clicked() {
                        filter = GridFilter::Tag(tag.clone());
                    }
                }
                if ui.button(&t_manage_tags).clicked() {
                    show_label_manager = true;
                }
            });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.quick_capture_text);
                if ui.button(&t_new).clicked() && !self.quick_capture_text.is_empty() {
                    if let Some(vault) = self.vault.as_mut() {
                        match Note::create(&vault.root, &self.quick_capture_text, "") {
                            Ok(note) => {
                                self.quick_capture_text.clear();
                                actions.push(GridAction::Open(note.path.clone()));
                            }
                            Err(e) => log::warn!("app: failed to create note: {e}"),
                        }
                    }
                }
            });

            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut search_text).hint_text("🔍"));
                egui::ComboBox::from_id_salt("sort_mode")
                    .selected_text(match sort_mode {
                        SortMode::Modified => &t_sort_modified,
                        SortMode::Created => &t_sort_created,
                        SortMode::Title => &t_sort_title,
                        SortMode::Color => &t_sort_color,
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut sort_mode, SortMode::Modified, &t_sort_modified);
                        ui.selectable_value(&mut sort_mode, SortMode::Created, &t_sort_created);
                        ui.selectable_value(&mut sort_mode, SortMode::Title, &t_sort_title);
                        ui.selectable_value(&mut sort_mode, SortMode::Color, &t_sort_color);
                    });
                let selection_label = if selection_mode { &t_selection_off } else { &t_selection_on };
                if ui.button(selection_label).clicked() {
                    selection_mode = !selection_mode;
                    if !selection_mode {
                        selected.clear();
                    }
                }
            });

            if selection_mode && !selected.is_empty() {
                ui.horizontal(|ui| {
                    if ui.button(&t_selection_archive).clicked() {
                        actions.push(GridAction::BatchArchive(selected.iter().copied().collect()));
                        selected.clear();
                    }
                    if ui.button(&t_selection_trash).clicked() {
                        actions.push(GridAction::BatchTrash(selected.iter().copied().collect()));
                        selected.clear();
                    }
                });
            }

            ui.separator();

            let mut visible = query::filter_notes(&notes, &filter, &search_text);
            query::sort_notes(&mut visible, sort_mode);
            let in_trash_view = filter == GridFilter::Trashed;

            if notes.is_empty() {
                ui.label(&t_empty);
            } else if visible.is_empty() {
                ui.label(&t_empty_filtered);
            } else {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for note in &visible {
                            render_note_card(
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
                    });
                });
            }

            if !self.status.is_empty() {
                ui.separator();
                ui.colored_label(egui::Color32::RED, &self.status);
            }
        });

        if let Some(id) = confirm_delete {
            egui::Window::new(&t_confirm_title)
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.label(&t_confirm_body);
                    ui.horizontal(|ui| {
                        if ui.button(&t_confirm_yes).clicked() {
                            actions.push(GridAction::DeletePermanently(id));
                            confirm_delete = None;
                        }
                        if ui.button(&t_confirm_cancel).clicked() {
                            confirm_delete = None;
                        }
                    });
                });
        }

        if show_label_manager {
            egui::Window::new(&t_tag_manager_title)
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    for (tag, count) in &all_tags {
                        ui.horizontal(|ui| {
                            match &mut tag_rename {
                                Some((target, draft)) if target == tag => {
                                    ui.text_edit_singleline(draft);
                                    if ui.button(&t_tag_rename).clicked() {
                                        actions.push(GridAction::RenameTag(target.clone(), draft.clone()));
                                        tag_rename = None;
                                    }
                                    if ui.button(&t_confirm_cancel).clicked() {
                                        tag_rename = None;
                                    }
                                }
                                _ => {
                                    ui.colored_label(theme::tag_color(tag), format!("#{tag} ({count})"));
                                    if ui.button(&t_tag_rename).clicked() {
                                        tag_rename = Some((tag.clone(), tag.clone()));
                                    }
                                    if ui.button(&t_tag_delete).clicked() {
                                        actions.push(GridAction::DeleteTag(tag.clone()));
                                    }
                                }
                            }
                        });
                    }
                    ui.separator();
                    if ui.button(&t_confirm_cancel).clicked() {
                        show_label_manager = false;
                    }
                });
        }

        self.grid_filter = filter;
        self.sort_mode = sort_mode;
        self.search_text = search_text;
        self.selection_mode = selection_mode;
        self.selected = selected;
        self.show_label_manager = show_label_manager;
        self.tag_rename = tag_rename;
        self.confirm_delete = confirm_delete;

        for action in actions {
            self.apply_grid_action(action);
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

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.search_query)
                        .hint_text(t_placeholder.as_str())
                        .desired_width(320.0),
                );
                let enter_pressed = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
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
                                    open_note = Some(hit.chunk.doc_id);
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
                    let clicked = ui.add_enabled(can_send, egui::Button::new(&t_send)).clicked();
                    if can_send && (clicked || enter_pressed) {
                        self.send_chat_message();
                    }
                });
            });

            egui::ScrollArea::vertical().show(ui, |ui| {
                if self.chat_messages.is_empty() {
                    ui.label(&t_empty);
                }
                for msg in &self.chat_messages {
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
                                    ui.label(egui::RichText::new(citation).small());
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
                    let pin_label = if note.frontmatter.pinned { &card.unpin } else { &card.pin };
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

                    let archive_label = if note.frontmatter.archived { &card.unarchive } else { &card.archive };
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

impl eframe::App for LontarApp {
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
        // `self.editor` borrow ends before `rescan_and_reindex` needs
        // `&mut self` again.
        let mut needs_reindex: Option<Note> = None;
        if let Some(editor) = self.editor.as_mut() {
            if editor.should_autosave() {
                match editor.autosave() {
                    Ok(()) => needs_reindex = Some(editor.note.clone()),
                    Err(e) => log::warn!("app: autosave failed: {e}"),
                }
            }
        }
        if let Some(note) = needs_reindex {
            self.rescan_and_reindex();
            self.reindex_note(&note);
        }

        // Fase 7 background-worker polling: writes chunk/embedding
        // results into the SQLite cache, and dispatches search/chat
        // query-embedding + generation events to whichever request is
        // pending.
        self.poll_indexer_results();
        self.poll_search_and_chat(ui.ctx());

        let t_nav_notes = self.t("nav-notes");
        let t_nav_search = self.t("nav-search");
        let t_nav_chat = self.t("nav-chat");
        let show_tabs = self.vault.is_some() && self.editor.is_none();

        egui::Panel::top("top_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(self.t("app-title"));
                if show_tabs {
                    ui.separator();
                    ui.selectable_value(&mut self.view, View::Notes, &t_nav_notes);
                    ui.selectable_value(&mut self.view, View::Search, &t_nav_search);
                    ui.selectable_value(&mut self.view, View::Chat, &t_nav_chat);
                }
            });
        });

        egui::CentralPanel::default().show(ui, |ui| {
            if self.vault.is_none() {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label(self.t("vault-select-prompt"));
                    if ui.button(self.t("vault-pick-folder")).clicked() {
                        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                            match Vault::open(folder) {
                                Ok(vault) => self.activate_vault(vault),
                                Err(e) => {
                                    self.status = format!("Gagal membuka vault: {e}");
                                    log::warn!("app: failed to open vault: {e}");
                                }
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

            match self.view {
                View::Notes => self.show_grid(ui),
                View::Search => self.show_search(ui),
                View::Chat => self.show_chat(ui),
            }
        });
    }
}
