//! Sheets — tabular data living in the vault (§3.8). CSV/TSV files are
//! editable and remain the source of truth; workbooks (XLSX/XLSM/XLSB/
//! XLS/ODS) open read-only and are only ever converted or exported, never
//! overwritten. `model` is the pure table + edits, `history` the undoable
//! editing session, `csv_io`/`xlsx_io` the file formats, `ingest` turns a
//! sheet into indexable text. Pure, no egui.
//! Callers: `app::sheet`, `app` (file routing), `api::sheet`,
//! `core::indexer`, `markdown::sheet_embed`, `ui::sidebar`.

pub mod csv_io;
pub mod history;
pub mod ingest;
pub mod model;
pub mod xlsx_io;

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

pub use csv_io::CsvFormat;
pub use model::{ColumnStats, Sheet};

/// Extensions of files opened as editable CSV sheets.
pub const CSV_EXTENSIONS: [&str; 3] = ["csv", "tsv", "tab"];
/// Extensions of read-only workbooks.
pub const WORKBOOK_EXTENSIONS: [&str; 5] = ["xlsx", "xlsm", "xlsb", "xls", "ods"];

/// What kind of sheet file a path is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetKind {
    /// `.csv` / `.tsv`: editable, saved back in place.
    Csv,
    /// `.xlsx` & co.: read-only.
    Workbook,
}

impl SheetKind {
    pub fn of(path: &Path) -> Option<SheetKind> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        if CSV_EXTENSIONS.contains(&ext.as_str()) {
            Some(SheetKind::Csv)
        } else if WORKBOOK_EXTENSIONS.contains(&ext.as_str()) {
            Some(SheetKind::Workbook)
        } else {
            None
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SheetKind::Csv => "csv",
            SheetKind::Workbook => "workbook",
        }
    }
}

pub fn is_sheet_path(path: &Path) -> bool {
    SheetKind::of(path).is_some()
}

/// A loaded sheet file: one sheet for CSV, one per worksheet otherwise.
#[derive(Debug, Clone)]
pub struct SheetFile {
    pub path: PathBuf,
    pub kind: SheetKind,
    pub sheets: Vec<Sheet>,
    /// `Some` for CSV files; how to write them back.
    pub csv_format: Option<CsvFormat>,
}

impl SheetFile {
    /// Whether this file may be saved in place.
    pub fn editable(&self) -> bool {
        self.kind == SheetKind::Csv && self.csv_format.is_some_and(|f| !f.lossy)
    }

    /// Worksheet by name (case-insensitive) or 0-based index; the first
    /// sheet when `name` is `None` or blank.
    pub fn sheet(&self, name: Option<&str>) -> Option<(usize, &Sheet)> {
        let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) else {
            return self.sheets.first().map(|s| (0, s));
        };
        self.sheets
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(name))
            .or_else(|| name.parse::<usize>().ok().filter(|i| *i < self.sheets.len()))
            .map(|i| (i, &self.sheets[i]))
    }
}

/// Loads any sheet file.
pub fn load(path: &Path) -> Result<SheetFile> {
    let kind =
        SheetKind::of(path).ok_or_else(|| anyhow!("{} is not a sheet file", path.display()))?;
    Ok(match kind {
        SheetKind::Csv => {
            let (sheet, format) = csv_io::load_csv(path)?;
            SheetFile {
                path: path.to_path_buf(),
                kind,
                sheets: vec![sheet],
                csv_format: Some(format),
            }
        }
        SheetKind::Workbook => SheetFile {
            path: path.to_path_buf(),
            kind,
            sheets: xlsx_io::read_workbook(path)?,
            csv_format: None,
        },
    })
}

/// Every sheet file under `root`, skipping dot-folders (`.mnemonic`,
/// `.trash`, `.git`, …) like the note scanner does. Sorted.
pub fn find_sheets(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && is_sheet_path(e.path()))
        .map(|e| e.into_path())
        .collect();
    out.sort();
    out
}

/// Where "Convert to CSV" puts worksheet `sheet_name` of `workbook`:
/// `<stem> - <sheet>.csv` next to it (`<stem>.csv` for single-sheet
/// workbooks), with ` 2`, ` 3`… appended until the name is free.
pub fn csv_path_for(workbook: &Path, sheet_name: &str, single_sheet: bool) -> PathBuf {
    let stem = workbook
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "sheet".into());
    let safe_sheet: String = sheet_name
        .chars()
        .map(|c| if "/\\:*?\"<>|".contains(c) { '_' } else { c })
        .collect();
    let base = if single_sheet {
        stem
    } else {
        format!("{stem} - {safe_sheet}")
    };
    unique_sibling(workbook, &base, "csv")
}

/// `<dir of near>/<base>.<ext>`, or `<base> N.<ext>` with the first free
/// N ≥ 2.
pub fn unique_sibling(near: &Path, base: &str, ext: &str) -> PathBuf {
    let dir = near.parent().unwrap_or(Path::new("."));
    let mut candidate = dir.join(format!("{base}.{ext}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{base} {n}.{ext}"));
        n += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_extension() {
        assert_eq!(SheetKind::of(Path::new("a/B.CSV")), Some(SheetKind::Csv));
        assert_eq!(SheetKind::of(Path::new("x.tsv")), Some(SheetKind::Csv));
        assert_eq!(SheetKind::of(Path::new("x.xlsx")), Some(SheetKind::Workbook));
        assert_eq!(SheetKind::of(Path::new("x.md")), None);
        assert!(!is_sheet_path(Path::new("csv")));
    }

    #[test]
    fn convert_paths_never_clobber() {
        let dir = tempfile::tempdir().unwrap();
        let wb = dir.path().join("Budget.xlsx");
        assert_eq!(csv_path_for(&wb, "Q1", false), dir.path().join("Budget - Q1.csv"));
        assert_eq!(csv_path_for(&wb, "Q1", true), dir.path().join("Budget.csv"));
        std::fs::write(dir.path().join("Budget.csv"), "a").unwrap();
        assert_eq!(csv_path_for(&wb, "x", true), dir.path().join("Budget 2.csv"));
        assert_eq!(csv_path_for(&wb, "a/b", false), dir.path().join("Budget - a_b.csv"));
    }

    #[test]
    fn find_sheets_skips_hidden_folders() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::create_dir_all(dir.path().join(".trash")).unwrap();
        for f in ["a.csv", "sub/b.XLSX", ".trash/c.csv", "note.md"] {
            std::fs::write(dir.path().join(f), "x").unwrap();
        }
        let found = find_sheets(dir.path());
        assert_eq!(found, vec![dir.path().join("a.csv"), dir.path().join("sub/b.XLSX")]);
    }

    #[test]
    fn load_csv_file_is_editable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.csv");
        std::fs::write(&path, "a,b\n1,2\n").unwrap();
        let file = load(&path).unwrap();
        assert!(file.editable());
        assert_eq!(file.sheet(None).unwrap().1.rows.len(), 1);
        assert_eq!(file.sheet(Some("D")).unwrap().0, 0);
        assert!(file.sheet(Some("nope")).is_none());
    }
}
