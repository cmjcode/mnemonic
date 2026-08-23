//! Top-level `eframe::App` state & layout. Fase 1 shipped a minimal vault
//! picker + flat note list; Fase 2 (§3.2, §5) adds the note editor: Source
//! / Live Preview / Reading modes, interactive checklists, wikilinks with
//! `[[` autocomplete, a backlinks panel, and a heading outline. Richer
//! Keep-style grid UI (colors, pinning, drag-reorder) lands in Fase 3.
//! Callers: `main.rs`.

use std::path::PathBuf;

use egui_commonmark::CommonMarkCache;

use crate::core::IndexStore;
use crate::i18n::LocaleManager;
use crate::markdown::editor::{
    char_index_to_byte_offset, slash_menu_triggered, slash_templates, wikilink_autocomplete_query,
};
use crate::markdown::{self, wikilink, EditorMode, MarkdownEditor, WikilinkIndex};
use crate::notes::{trash, Note, Vault, VaultWatcher};

pub struct LontarApp {
    locales: LocaleManager,
    vault: Option<Vault>,
    watcher: Option<VaultWatcher>,
    index: Option<IndexStore>,
    quick_capture_text: String,
    status: String,
    editor: Option<MarkdownEditor>,
    markdown_cache: CommonMarkCache,
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
            quick_capture_text: String::new(),
            status: String::new(),
            editor: None,
            markdown_cache: CommonMarkCache::default(),
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
        } else if let Some(title) = navigate_to {
            if editor.is_dirty() {
                if let Err(e) = editor.autosave() {
                    log::warn!("app: failed to save note before navigating: {e}");
                }
            }
            self.rescan_and_reindex();
            self.navigate_wikilink(&title);
        } else {
            self.editor = Some(editor);
        }
    }
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
        let mut needs_reindex = false;
        if let Some(editor) = self.editor.as_mut() {
            if editor.should_autosave() {
                match editor.autosave() {
                    Ok(()) => needs_reindex = true,
                    Err(e) => log::warn!("app: autosave failed: {e}"),
                }
            }
        }
        if needs_reindex {
            self.rescan_and_reindex();
        }

        egui::Panel::top("top_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(self.t("app-title"));
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

            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.quick_capture_text);
                if ui.button(self.t("notes-new")).clicked() && !self.quick_capture_text.is_empty()
                {
                    if let Some(vault) = self.vault.as_mut() {
                        match Note::create(&vault.root, &self.quick_capture_text, "") {
                            Ok(note) => {
                                self.quick_capture_text.clear();
                                self.rescan_and_reindex();
                                self.open_note(note);
                            }
                            Err(e) => log::warn!("app: failed to create note: {e}"),
                        }
                    }
                }
            });

            ui.separator();

            let mut notes: Vec<&Note> = self
                .vault
                .as_ref()
                .map(|v| v.notes.iter().filter(|n| !n.frontmatter.trashed).collect())
                .unwrap_or_default();
            notes.sort_by(|a, b| b.frontmatter.modified.cmp(&a.frontmatter.modified));

            let mut clicked_path: Option<PathBuf> = None;
            if notes.is_empty() {
                ui.label(self.t("vault-empty"));
            } else {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for note in &notes {
                        if ui.button(note.frontmatter.title.as_str()).clicked() {
                            clicked_path = Some(note.path.clone());
                        }
                    }
                });
            }

            if !self.status.is_empty() {
                ui.separator();
                ui.colored_label(egui::Color32::RED, &self.status);
            }

            if let Some(path) = clicked_path {
                let clicked_note = self
                    .vault
                    .as_ref()
                    .and_then(|v| v.notes.iter().find(|n| n.path == path).cloned());
                if let Some(note) = clicked_note {
                    self.open_note(note);
                }
            }
        });
    }
}
