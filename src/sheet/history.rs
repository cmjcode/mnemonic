//! Editing session over a loaded `SheetFile` (§3.8.2): applies edits to
//! the active worksheet, records each as an invertible `Edit` for
//! undo/redo, and tracks the dirty flag against the last save. Pure, no
//! egui, so the grid editor's behaviour is unit-testable here.
//! Callers: `app::sheet`.

use super::{SheetFile, model::Sheet};

/// One reversible change to a worksheet.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    SetCell { row: usize, col: usize, old: String, new: String },
    InsertRow { at: usize, cells: Vec<String> },
    RemoveRow { at: usize, cells: Vec<String> },
    InsertColumn { at: usize, header: String, cells: Vec<String> },
    RemoveColumn { at: usize, header: String, cells: Vec<String> },
    RenameColumn { col: usize, old: String, new: String },
    /// Rows reordered so that `new[i] = old[perm[i]]`.
    Reorder { perm: Vec<usize> },
    /// Several edits undone/redone as one step (e.g. a pasted block).
    Batch(Vec<Edit>),
}

impl Edit {
    fn apply(&self, sheet: &mut Sheet) {
        match self {
            Edit::SetCell { row, col, new, .. } => {
                sheet.set_cell(*row, *col, new.clone());
            }
            Edit::InsertRow { at, cells } => {
                sheet.insert_row(*at, cells.clone());
            }
            Edit::RemoveRow { at, .. } => {
                sheet.remove_row(*at);
            }
            Edit::InsertColumn { at, header, cells } => {
                sheet.insert_column(*at, header.clone(), cells.clone());
            }
            Edit::RemoveColumn { at, .. } => {
                sheet.remove_column(*at);
            }
            Edit::RenameColumn { col, new, .. } => {
                if let Some(h) = sheet.headers.get_mut(*col) {
                    *h = new.clone();
                }
            }
            Edit::Reorder { perm } => sheet.apply_permutation(perm),
            Edit::Batch(edits) => edits.iter().for_each(|e| e.apply(sheet)),
        }
    }

    fn inverse(&self) -> Edit {
        match self.clone() {
            Edit::SetCell { row, col, old, new } => Edit::SetCell { row, col, old: new, new: old },
            Edit::InsertRow { at, cells } => Edit::RemoveRow { at, cells },
            Edit::RemoveRow { at, cells } => Edit::InsertRow { at, cells },
            Edit::InsertColumn { at, header, cells } => Edit::RemoveColumn { at, header, cells },
            Edit::RemoveColumn { at, header, cells } => Edit::InsertColumn { at, header, cells },
            Edit::RenameColumn { col, old, new } => Edit::RenameColumn { col, old: new, new: old },
            Edit::Reorder { perm } => {
                let mut inv = vec![0; perm.len()];
                for (new, &old) in perm.iter().enumerate() {
                    if let Some(slot) = inv.get_mut(old) {
                        *slot = new;
                    }
                }
                Edit::Reorder { perm: inv }
            }
            Edit::Batch(edits) => Edit::Batch(edits.iter().rev().map(Edit::inverse).collect()),
        }
    }
}

/// Undo depth; older edits are dropped.
const MAX_UNDO: usize = 500;

/// A sheet file open for viewing/editing.
#[derive(Debug, Clone)]
pub struct SheetDoc {
    pub file: SheetFile,
    /// Index into `file.sheets` of the worksheet on screen.
    pub active: usize,
    undo: Vec<(usize, Edit)>,
    redo: Vec<(usize, Edit)>,
    /// `undo.len()` at the last save, or `None` once that state can no
    /// longer be reached (only a new save makes the doc clean again).
    saved_at: Option<usize>,
}

impl SheetDoc {
    /// `file` must have at least one sheet (`sheet::load` guarantees it).
    pub fn new(mut file: SheetFile) -> SheetDoc {
        if file.sheets.is_empty() {
            file.sheets.push(Sheet::default());
        }
        SheetDoc {
            file,
            active: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            saved_at: Some(0),
        }
    }

    pub fn sheet(&self) -> &Sheet {
        &self.file.sheets[self.active.min(self.file.sheets.len() - 1)]
    }

    pub fn editable(&self) -> bool {
        self.file.editable()
    }

    pub fn is_dirty(&self) -> bool {
        self.saved_at != Some(self.undo.len())
    }

    pub fn mark_saved(&mut self) {
        self.saved_at = Some(self.undo.len());
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Applies `edit` to the active worksheet and records it. Ignored for
    /// read-only files and for no-op cell/header edits.
    pub fn apply(&mut self, edit: Edit) {
        if !self.editable() {
            return;
        }
        if let Edit::SetCell { old, new, .. } | Edit::RenameColumn { old, new, .. } = &edit
            && old == new
        {
            return;
        }
        if self.saved_at.is_some_and(|s| s > self.undo.len()) {
            // The saved state lives on the redo branch about to be dropped.
            self.saved_at = None;
        }
        let active = self.active.min(self.file.sheets.len() - 1);
        edit.apply(&mut self.file.sheets[active]);
        self.undo.push((active, edit));
        self.redo.clear();
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
            self.saved_at = self.saved_at.and_then(|s| s.checked_sub(1));
        }
    }

    /// Sets one cell, recording the old value.
    pub fn set_cell(&mut self, row: usize, col: usize, value: String) {
        let Some(old) = self.sheet().cell(row, col).map(str::to_string) else {
            return;
        };
        self.apply(Edit::SetCell { row, col, old, new: value });
    }

    pub fn insert_row(&mut self, at: usize) -> usize {
        let at = at.min(self.sheet().height());
        let cells = vec![String::new(); self.sheet().width()];
        self.apply(Edit::InsertRow { at, cells });
        at
    }

    pub fn remove_row(&mut self, at: usize) {
        if let Some(cells) = self.sheet().rows.get(at).cloned() {
            self.apply(Edit::RemoveRow { at, cells });
        }
    }

    pub fn insert_column(&mut self, at: usize, header: String) -> usize {
        let at = at.min(self.sheet().width());
        self.apply(Edit::InsertColumn { at, header, cells: Vec::new() });
        at
    }

    pub fn remove_column(&mut self, at: usize) {
        let sheet = self.sheet();
        if let Some(header) = sheet.headers.get(at).cloned() {
            let cells = sheet.rows.iter().map(|r| r[at].clone()).collect();
            self.apply(Edit::RemoveColumn { at, header, cells });
        }
    }

    pub fn rename_column(&mut self, col: usize, new: String) {
        if let Some(old) = self.sheet().headers.get(col).cloned() {
            self.apply(Edit::RenameColumn { col, old, new });
        }
    }

    pub fn sort_by_column(&mut self, col: usize, ascending: bool) {
        if !self.editable() {
            return;
        }
        let perm = self.sheet().sorted_order(col, ascending);
        if perm.iter().enumerate().any(|(i, &p)| i != p) {
            self.apply(Edit::Reorder { perm });
        }
    }

    /// Pastes a tab/newline-separated block (what Excel, Sheets and this
    /// editor put on the clipboard) with its top-left at `(row, col)`,
    /// appending rows as needed; cells past the last column are dropped.
    /// One undo step. Returns the number of cells written.
    pub fn paste_block(&mut self, row: usize, col: usize, text: &str) -> usize {
        let width = self.sheet().width();
        if !self.editable() || col >= width {
            return 0;
        }
        let text = text.strip_suffix('\n').unwrap_or(text);
        let text = text.strip_suffix('\r').unwrap_or(text);
        let mut edits = Vec::new();
        let mut height = self.sheet().height();
        let mut written = 0;
        for (dr, line) in text.split('\n').enumerate() {
            let r = row + dr;
            while r >= height {
                edits.push(Edit::InsertRow { at: height, cells: vec![String::new(); width] });
                height += 1;
            }
            for (dc, value) in line.trim_end_matches('\r').split('\t').enumerate() {
                let c = col + dc;
                if c >= width {
                    break;
                }
                let old = self.sheet().cell(r, c).unwrap_or_default().to_string();
                if old != value {
                    edits.push(Edit::SetCell { row: r, col: c, old, new: value.to_string() });
                    written += 1;
                }
            }
        }
        if !edits.is_empty() {
            self.apply(Edit::Batch(edits));
        }
        written
    }

    pub fn undo(&mut self) -> bool {
        let Some((sheet, edit)) = self.undo.pop() else {
            return false;
        };
        edit.inverse().apply(&mut self.file.sheets[sheet]);
        self.active = sheet;
        self.redo.push((sheet, edit));
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some((sheet, edit)) = self.redo.pop() else {
            return false;
        };
        edit.apply(&mut self.file.sheets[sheet]);
        self.active = sheet;
        self.undo.push((sheet, edit));
        true
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::sheet::{CsvFormat, SheetKind};

    fn doc(kind: SheetKind) -> SheetDoc {
        let sheet = Sheet::from_records(
            "t",
            vec![
                vec!["a".into(), "b".into()],
                vec!["2".into(), "x".into()],
                vec!["1".into(), "y".into()],
            ],
        );
        let path = PathBuf::from("t.csv");
        SheetDoc::new(SheetFile {
            csv_format: (kind == SheetKind::Csv).then(|| CsvFormat::for_path(&path)),
            path,
            kind,
            sheets: vec![sheet],
        })
    }

    #[test]
    fn every_edit_undoes_to_the_original() {
        let mut d = doc(SheetKind::Csv);
        let original = d.sheet().clone();
        d.set_cell(0, 1, "z".into());
        d.insert_row(1);
        d.insert_column(1, "mid".into());
        d.rename_column(0, "A".into());
        d.sort_by_column(0, true);
        d.remove_column(2);
        d.remove_row(0);
        assert!(d.is_dirty());
        let edited = d.sheet().clone();
        while d.undo() {}
        assert_eq!(d.sheet(), &original);
        assert!(!d.is_dirty());
        while d.redo() {}
        assert_eq!(d.sheet(), &edited);
    }

    #[test]
    fn dirty_tracks_the_saved_point() {
        let mut d = doc(SheetKind::Csv);
        d.set_cell(0, 0, "9".into());
        d.mark_saved();
        assert!(!d.is_dirty());
        d.undo();
        assert!(d.is_dirty());
        d.redo();
        assert!(!d.is_dirty());
        d.undo();
        d.set_cell(0, 0, "8".into()); // drops the redo branch holding the save
        assert!(d.is_dirty());
        d.undo();
        assert!(d.is_dirty(), "saved state is unreachable, stays dirty");
    }

    #[test]
    fn paste_block_grows_rows_and_undoes_in_one_step() {
        let mut d = doc(SheetKind::Csv);
        let original = d.sheet().clone();
        assert_eq!(d.paste_block(1, 1, "p\tdropped\nq\r\nr\n"), 3);
        assert_eq!(d.sheet().height(), 4);
        let col: Vec<&str> = d.sheet().rows.iter().map(|r| r[1].as_str()).collect();
        assert_eq!(col, vec!["x", "p", "q", "r"]);
        assert!(d.undo());
        assert_eq!(d.sheet(), &original);
        assert!(!d.can_undo());
    }

    #[test]
    fn no_op_and_read_only_edits_are_ignored() {
        let mut d = doc(SheetKind::Csv);
        d.set_cell(0, 0, "2".into());
        assert!(!d.can_undo());
        let mut ro = doc(SheetKind::Workbook);
        ro.set_cell(0, 0, "5".into());
        ro.sort_by_column(0, true);
        assert!(!ro.can_undo());
        assert_eq!(ro.sheet().rows[0][0], "2");
    }
}
