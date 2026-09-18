//! Sheet viewer/editor (§3.8.2): opens CSV/TSV (editable) and workbooks
//! (read-only, one tab per worksheet), with a filter box, undo/redo,
//! Save, *Export as XLSX* and *Convert to CSV*. The grid itself lives in
//! `app::sheet_grid`; the pure editing session is `sheet::history`.
//!
//! Saves are atomic and never clobber an external edit: if the file's
//! mtime moved since it was loaded, the edits go to a
//! `<name> (conflict).csv` sibling instead. New files (export, convert,
//! import, new sheet) always get a fresh, non-colliding name.
//! Callers: `app` (file routing, top bar, shortcuts, close/quit).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use egui::{Margin, RichText};
use egui_icons::icons::{
    ICON_ADD_ROW_BELOW, ICON_FILE_DOWNLOAD, ICON_FILTER_LIST, ICON_LOCK, ICON_REDO, ICON_SAVE,
    ICON_TRANSFORM, ICON_UNDO, ICON_VIEW_COLUMN,
};

use super::MnemonicApp;
use crate::core::ingestion;
use crate::sheet::history::SheetDoc;
use crate::sheet::model::{default_header, format_number};
use crate::sheet::{self, ColumnStats, CsvFormat, SheetKind, csv_io, xlsx_io};
use crate::ui::{ToastKind, pal, theme, widgets};

/// A cell being edited in place.
pub(super) struct CellEditor {
    pub(super) row: usize,
    pub(super) col: usize,
    pub(super) buffer: String,
    /// Focus hasn't been requested yet (first frame).
    pub(super) fresh: bool,
}

pub(super) struct SheetViewerState {
    pub(super) path: PathBuf,
    pub(super) doc: SheetDoc,
    /// mtime when loaded / last saved, to detect external edits.
    loaded_mtime: Option<SystemTime>,
    /// Selected cell as (sheet row, column).
    pub(super) cursor: Option<(usize, usize)>,
    pub(super) editing: Option<CellEditor>,
    /// Column header being renamed: (column, buffer, focus requested).
    pub(super) renaming: Option<(usize, String, bool)>,
    pub(super) filter: String,
    /// Column + direction of the last sort (drawn as an arrow; for
    /// read-only sheets it also orders the view).
    pub(super) sort: Option<(usize, bool)>,
    /// Sheet rows on screen, in display order.
    pub(super) visible: Vec<usize>,
    view_stale: bool,
    /// Visible index to scroll into view next frame.
    pub(super) scroll_to: Option<usize>,
    /// Right-aligned (numeric) columns, recomputed with the view.
    pub(super) numeric_cols: Vec<bool>,
    /// Footer stats of one column over `visible`, until the view changes.
    footer_cache: Option<(usize, ColumnStats)>,
}

/// Toolbar requests that need the app (disk, toasts, other views).
enum SheetCommand {
    Save,
    ExportXlsx,
    ConvertToCsv,
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

pub(super) fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "sheet".into())
}

impl SheetViewerState {
    fn new(path: PathBuf, doc: SheetDoc) -> SheetViewerState {
        SheetViewerState {
            loaded_mtime: mtime(&path),
            path,
            doc,
            cursor: None,
            editing: None,
            renaming: None,
            filter: String::new(),
            sort: None,
            visible: Vec::new(),
            view_stale: true,
            scroll_to: None,
            numeric_cols: Vec::new(),
            footer_cache: None,
        }
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.doc.is_dirty()
    }

    /// A cell or header text field is active (Esc/shortcuts go to it).
    pub(super) fn has_dialog_open(&self) -> bool {
        self.editing.is_some() || self.renaming.is_some()
    }

    /// Call after anything that changes rows, columns or the filter.
    pub(super) fn invalidate(&mut self) {
        self.view_stale = true;
        self.footer_cache = None;
    }

    /// `column_stats_over` the visible rows, cached per column.
    fn footer_stats(&mut self, col: usize) -> ColumnStats {
        if let Some((c, stats)) = self.footer_cache
            && c == col
        {
            return stats;
        }
        let stats = self
            .doc
            .sheet()
            .column_stats_over(col, self.visible.iter().copied());
        self.footer_cache = Some((col, stats));
        stats
    }

    /// Recomputes `visible` (filter, then view sort for read-only files)
    /// and the numeric-column flags when stale.
    pub(super) fn refresh_view(&mut self) {
        if !self.view_stale {
            return;
        }
        self.view_stale = false;
        let sheet = self.doc.sheet();
        let matches = sheet.filter_rows(&self.filter);
        self.visible = match self.sort {
            Some((col, asc)) if !self.doc.editable() => {
                let mut keep = vec![false; sheet.height()];
                matches.iter().for_each(|&i| keep[i] = true);
                sheet
                    .sorted_order(col, asc)
                    .into_iter()
                    .filter(|&i| keep[i])
                    .collect()
            }
            _ => matches,
        };
        self.numeric_cols = (0..sheet.width())
            .map(|c| sheet.column_stats(c).is_numeric())
            .collect();
        if let Some((row, col)) = self.cursor
            && (row >= sheet.height() || col >= sheet.width())
        {
            self.cursor = None;
        }
    }

    /// Position of sheet row `row` in the current view.
    pub(super) fn visible_index(&self, row: usize) -> Option<usize> {
        self.visible.iter().position(|&r| r == row)
    }

    /// Moves the cursor by visible rows / columns, clamped.
    pub(super) fn move_cursor(&mut self, drow: isize, dcol: isize) {
        let width = self.doc.sheet().width();
        if self.visible.is_empty() || width == 0 {
            return;
        }
        let Some((vi, col)) = self
            .cursor
            .and_then(|(r, c)| Some((self.visible_index(r)?, c)))
        else {
            self.cursor = Some((self.visible[0], 0));
            self.scroll_to = Some(0);
            return;
        };
        let vi = (vi as isize + drow).clamp(0, self.visible.len() as isize - 1) as usize;
        let col = (col as isize + dcol).clamp(0, width as isize - 1) as usize;
        self.cursor = Some((self.visible[vi], col));
        self.scroll_to = Some(vi);
    }

    pub(super) fn begin_edit(&mut self, row: usize, col: usize, initial: Option<String>) {
        if !self.doc.editable() {
            return;
        }
        let buffer = initial
            .unwrap_or_else(|| self.doc.sheet().cell(row, col).unwrap_or("").to_string());
        self.cursor = Some((row, col));
        self.editing = Some(CellEditor {
            row,
            col,
            buffer,
            fresh: true,
        });
    }

    /// Writes the edited cell back, then moves the cursor.
    pub(super) fn commit_edit(&mut self, drow: isize, dcol: isize) {
        if let Some(edit) = self.editing.take() {
            self.doc.set_cell(edit.row, edit.col, edit.buffer);
            self.invalidate();
            self.refresh_view();
            self.cursor = Some((edit.row, edit.col));
            if drow != 0 || dcol != 0 {
                self.move_cursor(drow, dcol);
            }
        }
    }

    pub(super) fn commit_rename(&mut self) {
        if let Some((col, name, _)) = self.renaming.take() {
            let name = name.trim();
            if !name.is_empty() {
                self.doc.rename_column(col, name.to_string());
                self.invalidate();
            }
        }
    }

    pub(super) fn toggle_sort(&mut self, col: usize) {
        let ascending = !matches!(self.sort, Some((c, true)) if c == col);
        self.set_sort(col, ascending);
    }

    pub(super) fn set_sort(&mut self, col: usize, ascending: bool) {
        self.sort = Some((col, ascending));
        self.doc.sort_by_column(col, ascending);
        self.invalidate();
    }

    pub(super) fn undo(&mut self) {
        self.editing = None;
        if self.doc.undo() {
            self.invalidate();
        }
    }

    pub(super) fn redo(&mut self) {
        self.editing = None;
        if self.doc.redo() {
            self.invalidate();
        }
    }

    /// Inserts an empty row below the cursor (or at the end) and selects it.
    pub(super) fn add_row(&mut self) {
        let at = self
            .cursor
            .map(|(r, _)| r + 1)
            .unwrap_or(self.doc.sheet().height());
        self.insert_row_at(at);
    }

    pub(super) fn insert_row_at(&mut self, at: usize) {
        let at = self.doc.insert_row(at);
        // A new blank row must stay visible even under a filter.
        self.filter.clear();
        self.invalidate();
        self.refresh_view();
        let col = self.cursor.map(|(_, c)| c).unwrap_or(0);
        self.cursor = Some((at, col));
        self.scroll_to = self.visible_index(at);
    }

    /// Inserts a column right of the cursor (or at the end) and starts
    /// renaming it.
    pub(super) fn add_column(&mut self) {
        let at = self
            .cursor
            .map(|(_, c)| c + 1)
            .unwrap_or(self.doc.sheet().width());
        self.insert_column_at(at);
    }

    pub(super) fn insert_column_at(&mut self, at: usize) {
        let name = default_header(self.doc.sheet().width());
        let at = self.doc.insert_column(at, name.clone());
        self.invalidate();
        self.renaming = Some((at, name, false));
    }
}

impl MnemonicApp {
    /// Loads and shows a sheet file.
    pub(super) fn open_sheet(&mut self, path: PathBuf) {
        match sheet::load(&path) {
            Ok(file) => {
                self.editor = None;
                self.pdf_viewer = None;
                self.sheet_viewer = Some(SheetViewerState::new(path, SheetDoc::new(file)));
            }
            Err(e) => self.report_error("error-context-open-sheet", format!("{e:#}")),
        }
    }

    /// Opens a sheet with its 1-based data row `row` selected (citations).
    pub(super) fn open_sheet_at_row(&mut self, path: PathBuf, row: usize) {
        self.open_sheet(path);
        if let Some(viewer) = self.sheet_viewer.as_mut() {
            viewer.refresh_view();
            let row = row.saturating_sub(1);
            if row < viewer.doc.sheet().height() {
                viewer.cursor = Some((row, 0));
                viewer.scroll_to = viewer.visible_index(row);
            }
        }
    }

    /// Saves the open sheet if it has unsaved edits. `false` when the
    /// save failed and the edits exist only in memory.
    pub(super) fn save_sheet_now(&mut self) -> bool {
        let Some(viewer) = self.sheet_viewer.as_mut() else {
            return true;
        };
        viewer.commit_edit(0, 0);
        viewer.commit_rename();
        if !viewer.doc.is_dirty() || !viewer.doc.editable() {
            return true;
        }
        let format = viewer
            .doc
            .file
            .csv_format
            .unwrap_or_else(|| CsvFormat::for_path(&viewer.path));
        let changed_on_disk =
            viewer.loaded_mtime.is_some() && mtime(&viewer.path) != viewer.loaded_mtime;
        let target = if changed_on_disk {
            let ext = viewer
                .path
                .extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_else(|| "csv".into());
            let base = format!("{} (conflict)", file_stem(&viewer.path));
            sheet::unique_sibling(&viewer.path, &base, &ext)
        } else {
            viewer.path.clone()
        };
        if let Err(e) = csv_io::save_csv(&target, viewer.doc.sheet(), &format) {
            self.report_error("error-context-save-sheet", format!("{e:#}"));
            return false;
        }
        viewer.doc.mark_saved();
        if changed_on_disk {
            // Keep working on the copy that holds the user's edits.
            viewer.path = target.clone();
            viewer.doc.file.path = target.clone();
        }
        viewer.loaded_mtime = mtime(&target);
        if changed_on_disk {
            let name = file_name(&target);
            self.toast(ToastKind::Error, "toast-sheet-conflict", &[("name", &name)]);
            self.refresh_derived();
        }
        self.submit_sheet_for_indexing(target);
        true
    }

    /// Writes the active worksheet to a new `.xlsx` next to the file.
    fn export_sheet_xlsx(&mut self) {
        let Some(viewer) = self.sheet_viewer.as_ref() else {
            return;
        };
        let target = sheet::unique_sibling(&viewer.path, &file_stem(&viewer.path), "xlsx");
        match xlsx_io::export_xlsx(viewer.doc.sheet(), &target) {
            Ok(()) => {
                let name = file_name(&target);
                self.toast(ToastKind::Success, "toast-sheet-exported", &[("name", &name)]);
                self.refresh_derived();
            }
            Err(e) => self.report_error("error-context-export-sheet", format!("{e:#}")),
        }
    }

    /// Writes the active worksheet of a workbook to a new, editable CSV
    /// and opens it.
    fn convert_sheet_to_csv(&mut self) {
        let Some(viewer) = self.sheet_viewer.as_ref() else {
            return;
        };
        let sheet = viewer.doc.sheet();
        let single = viewer.doc.file.sheets.len() == 1;
        let target = sheet::csv_path_for(&viewer.path, &sheet.name, single);
        match csv_io::save_csv(&target, sheet, &CsvFormat::for_path(&target)) {
            Ok(()) => {
                let name = file_name(&target);
                self.toast(ToastKind::Success, "toast-sheet-converted", &[("name", &name)]);
                self.refresh_derived();
                self.submit_sheet_for_indexing(target.clone());
                self.open_sheet(target);
            }
            Err(e) => self.report_error("error-context-export-sheet", format!("{e:#}")),
        }
    }

    /// Creates `<untitled>.csv` with three blank columns in `parent_dir`
    /// (vault root by default) and opens it.
    pub(super) fn create_sheet(&mut self, parent_dir: Option<PathBuf>) {
        let Some(root) = self.vault.as_ref().map(|v| v.root.clone()) else {
            return;
        };
        self.close_document();
        if self.editor.is_some() || self.sheet_viewer.is_some() {
            return;
        }
        let dir = parent_dir.unwrap_or(root);
        let base = self.t("sheet-untitled");
        let target = sheet::unique_sibling(&dir.join(".probe"), &base, "csv");
        let headers: Vec<String> = (0..3).map(default_header).collect();
        let blank = sheet::Sheet::from_records(base, vec![headers, vec![String::new(); 3]]);
        match csv_io::save_csv(&target, &blank, &CsvFormat::for_path(&target)) {
            Ok(()) => {
                self.refresh_derived();
                self.open_sheet(target);
            }
            Err(e) => self.report_error("error-context-save-sheet", format!("{e:#}")),
        }
    }

    /// Copies picked CSV/XLSX/… files into the vault root (never moving or
    /// touching the originals) and opens the last one.
    pub(super) fn import_sheet_dialog(&mut self) {
        let Some(root) = self.vault.as_ref().map(|v| v.root.clone()) else {
            return;
        };
        let mut exts: Vec<&str> = sheet::CSV_EXTENSIONS.to_vec();
        exts.extend(sheet::WORKBOOK_EXTENSIONS);
        let Some(files) = rfd::FileDialog::new()
            .add_filter(self.t("sheet-import-filter"), &exts)
            .pick_files()
        else {
            return;
        };
        let mut last = None;
        for file in files {
            let target = if file.starts_with(&root) {
                file
            } else {
                let ext = file
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_else(|| "csv".into());
                let target = sheet::unique_sibling(&root.join(".probe"), &file_stem(&file), &ext);
                if let Err(e) = std::fs::copy(&file, &target) {
                    self.report_error("error-context-import-sheet", e);
                    continue;
                }
                target
            };
            self.submit_sheet_for_indexing(target.clone());
            last = Some(target);
        }
        self.refresh_derived();
        if let Some(path) = last {
            self.close_document();
            if self.editor.is_none() && self.sheet_viewer.is_none() {
                self.open_sheet(path);
            }
        }
    }

    // ─── Indexing (§3.8.4) ───────────────────────────────────────────────

    /// Queues one sheet for chunking + embedding (skipped when too large).
    pub(super) fn submit_sheet_for_indexing(&mut self, path: PathBuf) {
        let too_big = std::fs::metadata(&path)
            .is_ok_and(|m| m.len() > sheet::ingest::MAX_INDEXED_BYTES);
        if too_big {
            log::info!("app: not indexing large sheet {}", path.display());
            return;
        }
        if let Some(indexer) = &self.indexer {
            indexer.submit_sheet(path);
            self.index_jobs_pending += 1;
        }
    }

    /// Queues every vault sheet whose file changed since it was last
    /// indexed, and drops index entries of sheets that no longer exist
    /// (deleted, moved or renamed outside the app).
    pub(super) fn reindex_changed_sheets(&mut self) {
        let Some(root) = self.vault.as_ref().map(|v| v.root.clone()) else {
            return;
        };
        let sheets = sheet::find_sheets(&root);
        let mut stale = Vec::new();
        if let Some(index) = self.index.as_ref() {
            let live: std::collections::HashSet<_> =
                sheets.iter().map(|p| ingestion::sheet_doc_id(p)).collect();
            for id in index.indexed_doc_ids("sheet").unwrap_or_default() {
                if !live.contains(&id)
                    && let Err(e) = index.delete_chunks_for_doc(id)
                {
                    log::warn!("app: failed to drop index of removed sheet: {e:#}");
                }
            }
            for path in sheets {
                let stamp = ingestion::sheet_file_stamp(&path);
                let indexed = index
                    .document_hash(ingestion::sheet_doc_id(&path))
                    .ok()
                    .flatten();
                if stamp.is_some() && stamp != indexed {
                    stale.push(path);
                }
            }
        }
        for path in stale {
            self.submit_sheet_for_indexing(path);
        }
    }

    /// Keeps the open sheet pointing at its file after a move/rename of
    /// the file or a folder containing it.
    pub(super) fn relocate_open_sheet(&mut self, old_prefix: &Path, new_prefix: &Path) {
        if let Some(viewer) = self.sheet_viewer.as_mut()
            && let Ok(rel) = viewer.path.strip_prefix(old_prefix)
        {
            let new = if rel.as_os_str().is_empty() {
                new_prefix.to_path_buf()
            } else {
                new_prefix.join(rel)
            };
            viewer.path = new.clone();
            viewer.doc.file.path = new;
            viewer.loaded_mtime = mtime(&viewer.path);
        }
    }

    /// Jump-to-source for a search hit or chat citation (`page_num` is
    /// 1-based): sheets open at the row, PDFs at the page, anything else
    /// as a note.
    pub(super) fn open_chunk_source(&mut self, path: PathBuf, page_num: Option<usize>) {
        match page_num {
            Some(row) if sheet::is_sheet_path(&path) => {
                self.close_document();
                if self.editor.is_none() && self.sheet_viewer.is_none() {
                    self.open_sheet_at_row(path, row);
                }
            }
            Some(page) => {
                self.close_document();
                if self.editor.is_none() && self.sheet_viewer.is_none() {
                    self.open_pdf_at_page(path, page.saturating_sub(1));
                }
            }
            None => self.open_file_by_path(path),
        }
    }

    pub(super) fn show_sheet_viewer(&mut self, ui: &mut egui::Ui) {
        let Some(mut viewer) = self.sheet_viewer.take() else {
            return;
        };
        viewer.refresh_view();
        let mut commands = Vec::new();
        self.sheet_toolbar(ui, &mut viewer, &mut commands);
        self.sheet_footer(ui, &mut viewer);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.sheet_grid(ui, &mut viewer));
        self.sheet_viewer = Some(viewer);
        for command in commands {
            match command {
                SheetCommand::Save => {
                    if self.save_sheet_now() {
                        self.toast(ToastKind::Success, "editor-saved", &[]);
                    }
                }
                SheetCommand::ExportXlsx => self.export_sheet_xlsx(),
                SheetCommand::ConvertToCsv => self.convert_sheet_to_csv(),
            }
        }
    }

    fn sheet_toolbar(
        &self,
        ui: &mut egui::Ui,
        viewer: &mut SheetViewerState,
        commands: &mut Vec<SheetCommand>,
    ) {
        let tr = &self.locales;
        let t = |key: &str| tr.t(key, &[]);
        let p = pal();
        let editable = viewer.doc.editable();
        egui::Panel::top("sheet_toolbar")
            .frame(theme::top_bar_frame().inner_margin(Margin::symmetric(12, 6)))
            .show_separator_line(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if !editable {
                        let tip = if viewer.doc.file.kind == SheetKind::Workbook {
                            t("sheet-read-only-workbook")
                        } else {
                            t("sheet-read-only-encoding")
                        };
                        ui.label(
                            RichText::new(format!("{} {}", ICON_LOCK.codepoint, t("sheet-read-only")))
                                .size(theme::TEXT_SM)
                                .color(p.warning),
                        )
                        .on_hover_text(tip);
                        ui.separator();
                    }

                    if viewer.doc.file.sheets.len() > 1 {
                        let names: Vec<String> =
                            viewer.doc.file.sheets.iter().map(|s| s.name.clone()).collect();
                        for (i, name) in names.iter().enumerate() {
                            if ui.selectable_label(viewer.doc.active == i, name).clicked()
                                && viewer.doc.active != i
                            {
                                viewer.doc.active = i;
                                viewer.cursor = None;
                                viewer.sort = None;
                                viewer.invalidate();
                            }
                        }
                        ui.separator();
                    }

                    ui.label(RichText::new(ICON_FILTER_LIST.codepoint).color(p.text_dim));
                    let filter = ui.add(
                        egui::TextEdit::singleline(&mut viewer.filter)
                            .hint_text(t("sheet-filter"))
                            .desired_width(180.0),
                    );
                    if filter.changed() {
                        viewer.invalidate();
                    }

                    if editable {
                        ui.separator();
                        let add_row = ICON_ADD_ROW_BELOW.codepoint;
                        if widgets::icon_button(ui, add_row, &t("sheet-add-row"), false).clicked() {
                            viewer.add_row();
                        }
                        let add_col = ICON_VIEW_COLUMN.codepoint;
                        if widgets::icon_button(ui, add_col, &t("sheet-add-column"), false).clicked()
                        {
                            viewer.add_column();
                        }
                        ui.add_enabled_ui(viewer.doc.can_undo(), |ui| {
                            let undo = ICON_UNDO.codepoint;
                            if widgets::icon_button(ui, undo, &t("editor-undo"), false).clicked() {
                                viewer.undo();
                            }
                        });
                        ui.add_enabled_ui(viewer.doc.can_redo(), |ui| {
                            let redo = ICON_REDO.codepoint;
                            if widgets::icon_button(ui, redo, &t("editor-redo"), false).clicked() {
                                viewer.redo();
                            }
                        });
                        ui.add_enabled_ui(viewer.is_dirty(), |ui| {
                            let save = ICON_SAVE.codepoint;
                            if widgets::icon_button(ui, save, &t("pdf-save"), false).clicked() {
                                commands.push(SheetCommand::Save);
                            }
                        });
                    }

                    ui.separator();
                    if viewer.doc.file.kind == SheetKind::Workbook {
                        let convert = ICON_TRANSFORM.codepoint;
                        if widgets::icon_button(ui, convert, &t("sheet-convert-csv"), false)
                            .clicked()
                        {
                            commands.push(SheetCommand::ConvertToCsv);
                        }
                    } else {
                        let export = ICON_FILE_DOWNLOAD.codepoint;
                        if widgets::icon_button(ui, export, &t("sheet-export-xlsx"), false)
                            .clicked()
                        {
                            commands.push(SheetCommand::ExportXlsx);
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let sheet = viewer.doc.sheet();
                        let mut size = tr.t(
                            "sheet-size",
                            &[
                                ("rows", &sheet.height().to_string()),
                                ("cols", &sheet.width().to_string()),
                            ],
                        );
                        if viewer.visible.len() != sheet.height() {
                            let shown = viewer.visible.len().to_string();
                            size.push_str(" · ");
                            size.push_str(&tr.t("sheet-showing", &[("shown", &shown)]));
                        }
                        if viewer.is_dirty() {
                            size.push_str(" · ");
                            size.push_str(&t("sheet-unsaved"));
                        }
                        ui.label(RichText::new(size).size(theme::TEXT_SM).color(p.text_dim));
                    });
                });
            });
    }

    /// Aggregates of the cursor's column over the visible rows (§3.8.2:
    /// COUNT / SUM / AVG / MIN / MAX — not a formula engine).
    fn sheet_footer(&self, ui: &mut egui::Ui, viewer: &mut SheetViewerState) {
        let Some((_, col)) = viewer.cursor else {
            return;
        };
        let Some(header) = viewer.doc.sheet().headers.get(col).cloned() else {
            return;
        };
        let stats = viewer.footer_stats(col);
        let header = header.as_str();
        let tr = &self.locales;
        let text = if stats.is_numeric() {
            tr.t(
                "sheet-stats",
                &[
                    ("col", header),
                    ("count", &stats.numeric.to_string()),
                    ("sum", &format_number(stats.sum)),
                    ("avg", &stats.avg().map(format_number).unwrap_or_default()),
                    ("min", &format_number(stats.min)),
                    ("max", &format_number(stats.max)),
                ],
            )
        } else {
            tr.t(
                "sheet-stats-text",
                &[("col", header), ("filled", &stats.filled.to_string())],
            )
        };
        egui::Panel::bottom("sheet_footer")
            .frame(theme::top_bar_frame().inner_margin(Margin::symmetric(12, 4)))
            .show_separator_line(true)
            .show(ui, |ui| {
                ui.label(RichText::new(text).size(theme::TEXT_SM).color(pal().text_dim));
            });
    }
}
