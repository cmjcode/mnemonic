//! Command palette (⌘K) commands: actions, navigation, settings, and
//! quick-open for notes and PDFs matching what the user typed.

use std::path::Path;

use egui_icons::icons::{
    ICON_AUTO_AWESOME, ICON_CALENDAR_TODAY, ICON_CREATE_NEW_FOLDER, ICON_DARK_MODE, ICON_DASHBOARD, ICON_DELETE,
    ICON_DESCRIPTION, ICON_DRAW, ICON_FOLDER_OPEN, ICON_HUB, ICON_INVENTORY_2, ICON_KEYBOARD,
    ICON_LABEL,
    ICON_LANGUAGE, ICON_LEFT_PANEL_CLOSE, ICON_NOTE_ADD, ICON_PICTURE_AS_PDF, ICON_SEARCH,
    ICON_UPLOAD_FILE,
};

use super::MnemonicApp;
use crate::ui::{self, PaletteCommand, SidebarDocFilter};

/// How many matching notes / PDFs the palette lists for quick-open.
const MAX_DOCUMENT_RESULTS: usize = 12;

impl MnemonicApp {
    fn palette_commands(&self) -> Vec<PaletteCommand> {
        let t = |key: &str| self.t(key);
        let actions = t("palette-cat-actions");
        let go = t("palette-cat-navigate");
        let docs = t("palette-cat-documents");
        let view = t("palette-cat-view");

        let mut cmds = vec![
            PaletteCommand::new(
                "new_note",
                &actions,
                ICON_NOTE_ADD.codepoint,
                t("notes-new"),
            )
            .with_hint(self.hotkey_label("new_note")),
            PaletteCommand::new(
                "new_canvas",
                &actions,
                ICON_DRAW.codepoint,
                t("sidebar-new-canvas"),
            ),
            PaletteCommand::new(
                "new_folder",
                &actions,
                ICON_CREATE_NEW_FOLDER.codepoint,
                t("sidebar-new-folder"),
            ),
            PaletteCommand::new(
                "import_pdf",
                &actions,
                ICON_UPLOAD_FILE.codepoint,
                t("pdf-import"),
            ),
            PaletteCommand::new(
                "search",
                &actions,
                ICON_SEARCH.codepoint,
                t("palette-search"),
            )
            .with_hint(self.hotkey_label("search")),
            PaletteCommand::new(
                "ask_ai",
                &actions,
                ICON_AUTO_AWESOME.codepoint,
                t("palette-ask-ai"),
            )
            .with_hint(self.hotkey_label("ai")),
            PaletteCommand::new(
                "open_graph",
                &go,
                ICON_HUB.codepoint,
                t("graph-title"),
            )
            .with_hint(self.hotkey_label("graph")),
            PaletteCommand::new(
                "daily_note",
                &actions,
                ICON_CALENDAR_TODAY.codepoint,
                t("palette-daily-note"),
            )
            .with_hint(self.hotkey_label("daily")),
            PaletteCommand::new(
                "migrate_filenames",
                &actions,
                ICON_DESCRIPTION.codepoint,
                t("palette-migrate-filenames"),
            ),
            PaletteCommand::new(
                "toggle_rerank",
                &view,
                ICON_AUTO_AWESOME.codepoint,
                if self.settings.rerank_search {
                    t("palette-rerank-off")
                } else {
                    t("palette-rerank-on")
                },
            ),
            PaletteCommand::new(
                "manage_tags",
                &actions,
                ICON_LABEL.codepoint,
                t("sidebar-manage-tags"),
            ),
            PaletteCommand::new("nav_all", &go, ICON_DASHBOARD.codepoint, t("sidebar-all")),
            PaletteCommand::new(
                "nav_notes",
                &go,
                ICON_DESCRIPTION.codepoint,
                t("sidebar-notes-only"),
            ),
            PaletteCommand::new(
                "nav_canvas",
                &go,
                ICON_DRAW.codepoint,
                t("sidebar-whiteboards-only"),
            ),
            PaletteCommand::new(
                "nav_pdf",
                &go,
                ICON_PICTURE_AS_PDF.codepoint,
                t("sidebar-pdfs-only"),
            ),
            PaletteCommand::new(
                "nav_archive",
                &go,
                ICON_INVENTORY_2.codepoint,
                t("sidebar-archived"),
            ),
            PaletteCommand::new("nav_trash", &go, ICON_DELETE.codepoint, t("sidebar-trash")),
            PaletteCommand::new(
                "toggle_sidebar",
                &view,
                ICON_LEFT_PANEL_CLOSE.codepoint,
                t("palette-toggle-sidebar"),
            )
            .with_hint(self.hotkey_label("sidebar")),
            PaletteCommand::new(
                "toggle_theme",
                &view,
                ICON_DARK_MODE.codepoint,
                t("palette-toggle-theme"),
            ),
            PaletteCommand::new(
                "toggle_language",
                &view,
                ICON_LANGUAGE.codepoint,
                t("palette-toggle-language"),
            ),
            PaletteCommand::new(
                "shortcuts",
                &view,
                ICON_KEYBOARD.codepoint,
                t("settings-shortcuts"),
            )
            .with_hint(self.hotkey_label("shortcuts")),
            PaletteCommand::new(
                "switch_vault",
                &view,
                ICON_FOLDER_OPEN.codepoint,
                t("settings-switch-vault"),
            ),
        ];

        // Quick-open: documents whose title matches the typed query.
        let query = self.command_palette.query().trim().to_lowercase();
        if query.is_empty() {
            return cmds;
        }
        if let Some(vault) = &self.vault {
            let notes = vault
                .notes
                .iter()
                .filter(|n| !n.frontmatter.trashed)
                .filter(|n| n.frontmatter.title.to_lowercase().contains(&query))
                .take(MAX_DOCUMENT_RESULTS);
            for note in notes {
                let icon = if note.is_canvas() {
                    ICON_DRAW.codepoint
                } else {
                    ICON_DESCRIPTION.codepoint
                };
                cmds.push(PaletteCommand::new(
                    format!("open:{}", note.path.display()),
                    &docs,
                    icon,
                    note.frontmatter.title.clone(),
                ));
            }
        }
        let pdfs = self
            .pdf_documents
            .iter()
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().to_lowercase().contains(&query))
            })
            .take(MAX_DOCUMENT_RESULTS);
        for pdf in pdfs {
            cmds.push(PaletteCommand::new(
                format!("open:{}", pdf.display()),
                &docs,
                ICON_PICTURE_AS_PDF.codepoint,
                pdf.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
            ));
        }
        // Templates (`<vault>/Templates/*.md`) insert into the open note.
        if self.editor.is_some()
            && let Some(root) = self.vault.as_ref().map(|v| v.root.clone())
        {
            let templates_cat = t("palette-cat-templates");
            for (name, path) in crate::notes::templates::list_templates(&root) {
                cmds.push(PaletteCommand::new(
                    format!("template:{}", path.display()),
                    &templates_cat,
                    ICON_DESCRIPTION.codepoint,
                    self.t_args("palette-insert-template", &[("name", &name)]),
                ));
            }
        }
        cmds
    }

    pub(super) fn show_command_palette(&mut self, ctx: &egui::Context) {
        if !self.command_palette.is_open() {
            return;
        }
        let commands = self.palette_commands();
        let Some(id) = self.command_palette.show(ctx, &self.locales, &commands) else {
            return;
        };

        if let Some(path) = id.strip_prefix("open:") {
            self.open_file_by_path(path.into());
            return;
        }
        if let Some(path) = id.strip_prefix("template:") {
            self.insert_template(Path::new(path));
            return;
        }
        let go = |app: &mut Self, filter: SidebarDocFilter| {
            app.close_document();
            app.close_graph_view();
            app.doc_filter = filter;
        };
        match id.as_str() {
            "new_note" => self.create_note(None, false),
            "new_canvas" => self.create_note(None, true),
            "new_folder" => {
                if let Some(root) = self.vault.as_ref().map(|v| v.root.clone()) {
                    self.handle_sidebar_event(ui::SidebarEvent::CreateFolder { parent_dir: root });
                }
            }
            "import_pdf" => self.import_pdf_dialog(),
            "search" => {
                self.close_document();
                self.focus_search = true;
            }
            "ask_ai" => {
                self.chat_sidebar_open = true;
                ctx.memory_mut(|m| m.request_focus(ui::ChatSidebarDrawer::input_id()));
            }
            "manage_tags" => self.show_label_manager = true,
            "migrate_filenames" => self.migrate_uuid_file_names(),
            "daily_note" => self.open_daily_note(),
            "open_graph" => self.open_graph_view(),
            "toggle_rerank" => {
                self.settings.rerank_search = !self.settings.rerank_search;
                self.persist_settings();
                let key = if self.settings.rerank_search {
                    "toast-rerank-on"
                } else {
                    "toast-rerank-off"
                };
                self.toast(ui::ToastKind::Info, key, &[]);
            }
            "nav_all" => go(self, SidebarDocFilter::All),
            "nav_notes" => go(self, SidebarDocFilter::NotesOnly),
            "nav_canvas" => go(self, SidebarDocFilter::WhiteboardsOnly),
            "nav_pdf" => go(self, SidebarDocFilter::PdfsOnly),
            "nav_archive" => go(self, SidebarDocFilter::Archived),
            "nav_trash" => go(self, SidebarDocFilter::Trashed),
            "toggle_sidebar" => {
                self.sidebar_open = !self.sidebar_open;
                self.persist_settings();
            }
            "toggle_theme" => {
                self.theme_mode = self.theme_mode.toggled();
                self.persist_settings();
            }
            "toggle_language" => {
                let next = if self.locales.active_locale() == "id-ID" {
                    "en-US"
                } else {
                    "id-ID"
                };
                self.locales.set_active(next);
                self.persist_settings();
            }
            "shortcuts" => self.show_shortcuts = true,
            "switch_vault" => self.pick_and_open_vault(),
            _ => {}
        }
    }
}
