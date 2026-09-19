//! App glue for reading themes and print/export (§3.2.5): loading the
//! theme registry (built-ins + plugin folders, re-read when a vault opens),
//! picking the theme for the open note (frontmatter `theme:` beats the app
//! setting), switching themes, and printing / exporting the open note in
//! its theme's colours. PDF rendering runs on a background thread and is
//! polled per frame like the other workers.
//! Callers: `app` (top bar, palette, hotkeys, frame loop), `app::editor`.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use super::MnemonicApp;
use crate::export::{self, ExportFormat, ExportSource};
use crate::notes::Note;
use crate::reading_theme::{self, ReadingTheme, ThemeRegistry};
use crate::ui::{ExportKind, ToastKind};

/// A PDF being rendered in the background: where it goes, and the result.
pub(super) struct PendingExport {
    out: PathBuf,
    done: Receiver<Result<(), String>>,
}

impl MnemonicApp {
    /// Re-reads built-in and plugin themes (the vault's own folder too).
    pub(super) fn reload_themes(&mut self) {
        let root = self.vault.as_ref().map(|v| v.root.clone());
        self.themes = ThemeRegistry::load(&reading_theme::theme_dirs(root.as_deref()));
        if !self.themes.problems.is_empty() {
            let count = self.themes.problems.len().to_string();
            self.toast(ToastKind::Error, "theme-load-problems", &[("count", &count)]);
        }
    }

    /// The theme a note is shown and exported in: its frontmatter
    /// `theme:` when that theme exists, else the app-wide choice.
    pub(super) fn reading_theme_for(&self, note: &Note) -> &ReadingTheme {
        match reading_theme::note_theme(&note.frontmatter.extra) {
            Some(id) if self.themes.contains(id) => self.themes.get(id),
            _ => self.themes.get(&self.settings.reading_theme),
        }
    }

    /// Makes `id` the app-wide reading theme and remembers it.
    pub(super) fn set_reading_theme(&mut self, id: &str) {
        self.settings.reading_theme = id.to_string();
        self.persist_settings();
    }

    /// `(id, name)` of every theme, and the open note's frontmatter
    /// theme name, for the top bar picker.
    pub(super) fn theme_menu_entries(&self) -> (Vec<(String, String)>, Option<String>) {
        let entries = self.themes.list().iter().map(|t| (t.id.clone(), t.name.clone())).collect();
        let note_theme = self
            .editor
            .as_ref()
            .and_then(|e| reading_theme::note_theme(&e.note.frontmatter.extra))
            .filter(|id| self.themes.contains(id))
            .map(|id| self.themes.get(id).name.clone());
        (entries, note_theme)
    }

    /// Creates (if needed) and shows the per-user theme plugin folder,
    /// with a short README so it's obvious what goes there.
    pub(super) fn open_theme_folder(&mut self) {
        let Some(dir) = reading_theme::theme_dirs(None).into_iter().next() else {
            return;
        };
        let result = std::fs::create_dir_all(&dir).and_then(|()| {
            let readme = dir.join("README.txt");
            if readme.exists() { Ok(()) } else { std::fs::write(readme, THEME_FOLDER_README) }
        });
        match result.map_err(anyhow::Error::from).and_then(|()| export::system::open_path(&dir)) {
            Ok(()) => {}
            Err(e) => self.report_error("theme-folder-failed", e),
        }
    }

    /// Prints or exports the open note in its reading theme.
    pub(super) fn export_open_note(&mut self, kind: ExportKind) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if editor.mode.shows_canvas() {
            editor.sync_canvas_to_body();
        }
        let note = editor.note.clone();
        let theme = self.reading_theme_for(&note).clone();
        let lang = self.locales.active_locale().split('-').next().unwrap_or("en").to_string();
        let vault = self.vault.as_ref();
        let resolve = |target: &str| super::editor::resolve_embed_target(vault, target);
        let title = note.frontmatter.title.clone();
        let src = ExportSource { title: &title, body: &note.body, theme: &theme, resolve_embed: &resolve, lang: &lang };

        let format = match kind {
            ExportKind::Print => {
                match export::print_note(&src) {
                    Ok(_) => self.toast(ToastKind::Info, "export-print-opened", &[]),
                    Err(e) => self.report_error("export-failed", e),
                }
                return;
            }
            ExportKind::Html => ExportFormat::Html,
            ExportKind::Pdf => ExportFormat::Pdf,
        };
        let ext = format.extension();
        let Some(out) = rfd::FileDialog::new()
            .add_filter(ext.to_uppercase(), &[ext])
            .set_file_name(format!("{}.{ext}", export::file_stem_for(&title)))
            .save_file()
        else {
            return;
        };
        match format {
            ExportFormat::Html => match export::export_note(&src, format, &out) {
                Ok(()) => self.export_done(&out),
                Err(e) => self.report_error("export-failed", e),
            },
            ExportFormat::Pdf => {
                // Build the page here (it needs the vault for embeds), render
                // it off the UI thread: a headless browser takes a moment.
                let Some(browser) = export::system::find_browser() else {
                    self.toast(ToastKind::Error, "export-no-browser", &[]);
                    return;
                };
                let html = export::html::note_document(&title, &note.body, &theme, &resolve, &export::HtmlOptions {
                    auto_print: false,
                    lang: lang.clone(),
                });
                let (tx, rx) = channel();
                let target = out.clone();
                let spawned = std::thread::Builder::new().name("pdf-export".into()).spawn(move || {
                    let page = std::env::temp_dir().join(format!("mnemonic-export-{}.html", uuid::Uuid::new_v4()));
                    let result = std::fs::write(&page, html)
                        .map_err(anyhow::Error::from)
                        .and_then(|()| export::system::html_to_pdf(&browser, &page, &target));
                    let _ = std::fs::remove_file(&page);
                    let _ = tx.send(result.map_err(|e| format!("{e:#}")));
                });
                match spawned {
                    Ok(_) => {
                        self.pending_exports.push(PendingExport { out, done: rx });
                        self.toast(ToastKind::Info, "export-pdf-started", &[]);
                    }
                    Err(e) => self.report_error("export-failed", e),
                }
            }
        }
    }

    /// Reports finished background PDF exports.
    pub(super) fn poll_exports(&mut self) {
        let mut finished = Vec::new();
        self.pending_exports.retain(|job| match job.done.try_recv() {
            Ok(result) => {
                finished.push((job.out.clone(), result));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                finished.push((job.out.clone(), Err("export worker stopped".into())));
                false
            }
        });
        for (out, result) in finished {
            match result {
                Ok(()) => self.export_done(&out),
                Err(e) => {
                    let msg = format!("{}: {e}", self.t("export-failed"));
                    self.toasts.push(ToastKind::Error, msg);
                }
            }
        }
    }

    fn export_done(&mut self, out: &std::path::Path) {
        let name = out.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.toast(ToastKind::Success, "export-done", &[("file", &name)]);
    }
}

const THEME_FOLDER_README: &str = "MNEMONIC reading themes\n\
=======================\n\n\
Put theme files here as <name>.toml (the file name is the theme id).\n\
A theme sets colours for [light], [dark] and optionally [print]; any\n\
colour left out comes from `base` (default: mnemonic). Example:\n\n\
    name = \"Kopi\"\n\
    base = \"sunset\"\n\n\
    [light]\n\
    h1 = \"#6f4e37\"\n\
    link = \"#a0522d\"\n\n\
All keys: background text muted strong link tag h1..h6 code_text code_bg\n\
quote_bar rule table_border table_header_bg highlight_bg checkbox\n\
callout_note callout_tip callout_important callout_warning\n\
callout_caution callout_quote\n\n\
Themes can also live in <vault>/.mnemonic/themes/. Pick one from the\n\
palette icon above a note, or per note with `theme: kopi` in its\n\
frontmatter. Use \"Reload themes\" after editing a file.\n";
