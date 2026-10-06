//! Sheets for agents (§3.8.5): list the vault's CSV/XLSX files, read them
//! page by page with per-column aggregates, run deterministic filter +
//! sort + aggregate queries (reliable arithmetic instead of asking the
//! LLM to add numbers), and edit CSV/TSV files — set a cell, append rows,
//! create a sheet. Workbooks are read-only here exactly as in the app.
//! Writes are atomic (`sheet::csv_io::save_csv`); chunk indexing follows
//! on the next `reindex`, which also covers sheets (`reindex_sheets`).
//! Callers: `api::mcp`, `src/bin/mnemonic-cli.rs`.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde_json::Value;

use super::VaultService;
use super::index::KEYWORD_ONLY_HASH_PREFIX;
use super::types::*;
use crate::core::ingestion;
use crate::sheet::model::{Sheet, parse_number};
use crate::sheet::{self, CsvFormat, SheetFile, SheetKind, csv_io};

/// Upper bound on rows returned by one read/query call.
const MAX_ROWS_PER_CALL: usize = 5_000;

/// Rejects absolute paths, `..` and hidden folders in a vault-relative path.
fn check_relative(rel: &str) -> Result<()> {
    for component in Path::new(rel).components() {
        match component {
            Component::Normal(part) if !part.to_string_lossy().starts_with('.') => {}
            Component::CurDir => {}
            _ => bail!("`{rel}` must be a vault-relative path outside hidden folders"),
        }
    }
    Ok(())
}

impl VaultService {
    /// Absolute path of a sheet: a vault-relative path, or a file name /
    /// stem that matches exactly one sheet in the vault.
    pub fn resolve_sheet(&self, reference: &str) -> Result<PathBuf> {
        let reference = reference.trim().trim_start_matches("./");
        if reference.is_empty() {
            bail!("empty sheet reference");
        }
        check_relative(reference)?;
        let direct = self.root().join(reference);
        if direct.is_file() && sheet::is_sheet_path(&direct) {
            return Ok(direct);
        }
        let wanted = reference.to_lowercase();
        let mut matches: Vec<PathBuf> = sheet::find_sheets(self.root())
            .into_iter()
            .filter(|p| {
                let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase());
                let stem = p.file_stem().map(|n| n.to_string_lossy().to_lowercase());
                name.as_deref() == Some(&wanted) || stem.as_deref() == Some(&wanted)
            })
            .collect();
        match matches.len() {
            1 => Ok(matches.remove(0)),
            0 => bail!("sheet not found: {reference}"),
            _ => {
                let names: Vec<String> = matches.iter().map(|p| self.rel(p)).collect();
                bail!("`{reference}` is ambiguous: {}", names.join(", "))
            }
        }
    }

    fn load_sheet_file(&self, reference: &str) -> Result<SheetFile> {
        let path = self.resolve_sheet(reference)?;
        sheet::load(&path).with_context(|| format!("reading sheet {}", self.rel(&path)))
    }

    pub fn list_sheets(&self) -> Vec<SheetSummary> {
        sheet::find_sheets(self.root())
            .into_iter()
            .map(|path| {
                let meta = std::fs::metadata(&path).ok();
                let kind = SheetKind::of(&path).unwrap_or(SheetKind::Csv);
                SheetSummary {
                    path: self.rel(&path),
                    kind: kind.as_str(),
                    editable: kind == SheetKind::Csv,
                    size_bytes: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                    modified: meta
                        .and_then(|m| m.modified().ok())
                        .map(DateTime::<Utc>::from),
                }
            })
            .collect()
    }

    pub fn read_sheet(&self, req: &SheetReadRequest) -> Result<SheetData> {
        let file = self.load_sheet_file(&req.r#ref)?;
        let sh = pick_sheet(&file, req.sheet.as_deref())?;
        let rows = (req.offset..sh.height())
            .take(req.limit.min(MAX_ROWS_PER_CALL))
            .map(|r| SheetRowOut {
                row: r + 1,
                cells: sh.rows[r].clone(),
            })
            .collect();
        let all: Vec<usize> = (0..sh.height()).collect();
        Ok(SheetData {
            path: self.rel(&file.path),
            kind: file.kind.as_str(),
            editable: file.editable(),
            sheet: sh.name.clone(),
            sheets: file.sheets.iter().map(|s| s.name.clone()).collect(),
            headers: sh.headers.clone(),
            total_rows: sh.height(),
            offset: req.offset,
            rows,
            columns: (0..sh.width()).map(|c| column_out(sh, c, &all)).collect(),
        })
    }

    pub fn query_sheet(&self, req: &SheetQueryRequest) -> Result<SheetQueryResult> {
        let file = self.load_sheet_file(&req.r#ref)?;
        let sh = pick_sheet(&file, req.sheet.as_deref())?;
        let mut conditions = Vec::new();
        for f in &req.filters {
            conditions.push((column(sh, &f.column)?, FilterOp::parse(&f.op)?, f.value.as_str()));
        }
        let mut matched: Vec<usize> = (0..sh.height())
            .filter(|&r| conditions.iter().all(|(c, op, v)| op.holds(&sh.rows[r][*c], v)))
            .collect();
        if let Some(sort) = &req.sort_by {
            let mut keep = vec![false; sh.height()];
            matched.iter().for_each(|&r| keep[r] = true);
            matched = sh
                .sorted_order(column(sh, sort)?, !req.descending)
                .into_iter()
                .filter(|&r| keep[r])
                .collect();
        }
        let projection: Vec<usize> = match &req.columns {
            Some(names) if !names.is_empty() => {
                names.iter().map(|n| column(sh, n)).collect::<Result<_>>()?
            }
            _ => (0..sh.width()).collect(),
        };
        let rows = matched
            .iter()
            .take(req.limit.min(MAX_ROWS_PER_CALL))
            .map(|&r| SheetRowOut {
                row: r + 1,
                cells: projection.iter().map(|&c| sh.rows[r][c].clone()).collect(),
            })
            .collect();
        Ok(SheetQueryResult {
            path: self.rel(&file.path),
            sheet: sh.name.clone(),
            headers: projection.iter().map(|&c| sh.headers[c].clone()).collect(),
            matched: matched.len(),
            rows,
            columns: projection
                .iter()
                .map(|&c| column_out(sh, c, &matched))
                .collect(),
        })
    }

    pub fn set_sheet_cell(&mut self, req: &SheetSetCellRequest) -> Result<SheetWriteResult> {
        let (path, mut sh, format) = self.editable_sheet(&req.r#ref)?;
        let col = column(&sh, &req.column)?;
        if req.row == 0 || req.row > sh.height() {
            bail!("row {} is out of range (1..={})", req.row, sh.height());
        }
        let old = sh.set_cell(req.row - 1, col, req.value.clone());
        let changed = usize::from(old.as_deref() != Some(req.value.as_str()));
        if changed > 0 {
            csv_io::save_csv(&path, &sh, &format)?;
        }
        Ok(self.write_result(&path, &sh, changed))
    }

    pub fn append_sheet_rows(&mut self, req: &SheetAppendRequest) -> Result<SheetWriteResult> {
        let (path, mut sh, format) = self.editable_sheet(&req.r#ref)?;
        let mut changed = 0;
        for (i, value) in req.rows.iter().enumerate() {
            let cells = row_cells(&sh, value).with_context(|| format!("row {}", i + 1))?;
            changed += cells.iter().filter(|c| !c.is_empty()).count();
            let at = sh.height();
            sh.insert_row(at, cells);
        }
        if !req.rows.is_empty() {
            csv_io::save_csv(&path, &sh, &format)?;
        }
        Ok(self.write_result(&path, &sh, changed))
    }

    pub fn create_sheet(&mut self, req: &SheetCreateRequest) -> Result<SheetWriteResult> {
        let rel = req.path.trim().trim_start_matches("./");
        check_relative(rel)?;
        let path = self.root().join(rel);
        if SheetKind::of(&path) != Some(SheetKind::Csv) {
            bail!("new sheets must be .csv or .tsv files: {rel}");
        }
        if path.exists() {
            bail!("{rel} already exists");
        }
        if req.headers.iter().all(|h| h.trim().is_empty()) {
            bail!("a sheet needs at least one column header");
        }
        let mut records = vec![req.headers.clone()];
        records.extend(req.rows.iter().cloned());
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let sh = Sheet::from_records(name, records);
        csv_io::save_csv(&path, &sh, &CsvFormat::for_path(&path))?;
        Ok(self.write_result(&path, &sh, sh.height()))
    }

    /// Loads a CSV for writing; refuses workbooks and non-UTF-8 files.
    fn editable_sheet(&self, reference: &str) -> Result<(PathBuf, Sheet, CsvFormat)> {
        let file = self.load_sheet_file(reference)?;
        let rel = self.rel(&file.path);
        if file.kind == SheetKind::Workbook {
            bail!("{rel} is a workbook and read-only; convert it to CSV in the app to edit it");
        }
        let format = file.csv_format.context("CSV format missing")?;
        if format.lossy {
            bail!("{rel} is not UTF-8; refusing to rewrite it");
        }
        let sh = file.sheets.into_iter().next().context("empty sheet file")?;
        Ok((file.path, sh, format))
    }

    fn write_result(&self, path: &Path, sh: &Sheet, changed: usize) -> SheetWriteResult {
        SheetWriteResult {
            path: self.rel(path),
            rows: sh.height(),
            columns: sh.width(),
            changed,
        }
    }

    /// Chunks + stores every vault sheet whose file stamp changed (§3.8.4)
    /// and drops chunks of sheets that are gone. Called from `reindex`.
    pub(super) fn reindex_sheets(
        &mut self,
        semantic: bool,
        full: bool,
        report: &mut ReindexReport,
    ) -> Result<()> {
        let sheets = sheet::find_sheets(self.root());
        report.sheets_indexed = sheets.len();
        let live: std::collections::HashSet<_> =
            sheets.iter().map(|p| ingestion::sheet_doc_id(p)).collect();
        for id in self.index.indexed_doc_ids("sheet")? {
            if !live.contains(&id) {
                self.index.delete_chunks_for_doc(id)?;
                report.pruned += 1;
            }
        }
        for path in sheets {
            let too_big = std::fs::metadata(&path)
                .is_ok_and(|m| m.len() > sheet::ingest::MAX_INDEXED_BYTES);
            let Some(stamp) = ingestion::sheet_file_stamp(&path).filter(|_| !too_big) else {
                continue;
            };
            let id = ingestion::sheet_doc_id(&path);
            let stored = self.index.document_hash(id)?;
            let keyword_stamp = format!("{KEYWORD_ONLY_HASH_PREFIX}{stamp}");
            let up_to_date = stored.as_deref() == Some(stamp.as_str())
                || (!semantic && stored.as_deref() == Some(keyword_stamp.as_str()));
            if up_to_date && !full {
                continue;
            }
            match self.chunk_and_store_sheet(&path, semantic) {
                Ok(()) => {
                    let recorded = if semantic { stamp } else { keyword_stamp };
                    self.index.set_document_hash(id, &recorded)?;
                    report.sheets_chunked += 1;
                }
                Err(e) => {
                    let rel = self.rel(&path);
                    log::warn!("api: indexing {rel} failed: {e:#}");
                    report.failed.push(ReindexFailure {
                        path: rel,
                        error: format!("{e:#}"),
                    });
                }
            }
        }
        Ok(())
    }

    fn chunk_and_store_sheet(&mut self, path: &Path, semantic: bool) -> Result<()> {
        let chunks = ingestion::chunk_sheet(path)?;
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let inputs: Vec<String> = chunks
            .iter()
            .map(|c| ingestion::embedding_input(&title, None, &c.text_content))
            .collect();
        let vectors = self.vectors_for(&inputs, semantic)?;
        let pairs: Vec<_> = chunks.into_iter().zip(vectors).collect();
        self.index
            .replace_chunks(ingestion::sheet_doc_id(path), "sheet", &title, &pairs)
            .context("storing sheet chunks")
    }
}

fn pick_sheet<'a>(file: &'a SheetFile, name: Option<&str>) -> Result<&'a Sheet> {
    let names = || {
        file.sheets
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    file.sheet(name)
        .map(|(_, s)| s)
        .with_context(|| format!("worksheet `{}` not found (have: {})", name.unwrap_or(""), names()))
}

fn column(sh: &Sheet, name: &str) -> Result<usize> {
    sh.column_index(name)
        .with_context(|| format!("column `{name}` not found (have: {})", sh.headers.join(", ")))
}

fn column_out(sh: &Sheet, col: usize, rows: &[usize]) -> SheetColumnOut {
    let st = sh.column_stats_over(col, rows.iter().copied());
    let has = st.numeric > 0;
    SheetColumnOut {
        name: sh.headers[col].clone(),
        numeric: st.is_numeric(),
        filled: st.filled,
        count: st.numeric,
        sum: has.then_some(st.sum),
        avg: st.avg(),
        min: has.then_some(st.min),
        max: has.then_some(st.max),
    }
}

/// Cells for an appended row: a JSON array (positional) or an object keyed
/// by column name. Scalars are stringified; unknown columns are an error.
fn row_cells(sh: &Sheet, value: &Value) -> Result<Vec<String>> {
    let text = |v: &Value| match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let mut cells = vec![String::new(); sh.width()];
    match value {
        Value::Array(items) => {
            if items.len() > sh.width() {
                bail!("{} values for {} columns", items.len(), sh.width());
            }
            for (slot, item) in cells.iter_mut().zip(items) {
                *slot = text(item);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                cells[column(sh, key)?] = text(item);
            }
        }
        _ => bail!("a row must be an array of values or an object keyed by column"),
    }
    Ok(cells)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilterOp {
    Eq,
    Ne,
    Contains,
    NotContains,
    Gt,
    Gte,
    Lt,
    Lte,
    Empty,
    NotEmpty,
}

impl FilterOp {
    fn parse(op: &str) -> Result<FilterOp> {
        Ok(match op.trim().to_ascii_lowercase().as_str() {
            "eq" | "=" | "==" => FilterOp::Eq,
            "ne" | "!=" | "<>" => FilterOp::Ne,
            "contains" => FilterOp::Contains,
            "not_contains" => FilterOp::NotContains,
            "gt" | ">" => FilterOp::Gt,
            "gte" | ">=" => FilterOp::Gte,
            "lt" | "<" => FilterOp::Lt,
            "lte" | "<=" => FilterOp::Lte,
            "empty" => FilterOp::Empty,
            "not_empty" => FilterOp::NotEmpty,
            other => bail!(
                "unknown filter op `{other}` (eq, ne, contains, not_contains, gt, gte, lt, lte, empty, not_empty)"
            ),
        })
    }

    /// Numeric when both sides are numbers, else case-insensitive text.
    fn holds(self, cell: &str, value: &str) -> bool {
        use std::cmp::Ordering;
        let ordering = || match (parse_number(cell), parse_number(value)) {
            (Some(a), Some(b)) => a.partial_cmp(&b),
            _ => Some(cell.trim().to_lowercase().cmp(&value.trim().to_lowercase())),
        };
        let contains = || cell.to_lowercase().contains(&value.to_lowercase());
        match self {
            FilterOp::Eq => ordering() == Some(Ordering::Equal),
            FilterOp::Ne => ordering() != Some(Ordering::Equal),
            FilterOp::Contains => contains(),
            FilterOp::NotContains => !contains(),
            FilterOp::Gt => ordering() == Some(Ordering::Greater),
            FilterOp::Gte => matches!(ordering(), Some(Ordering::Greater | Ordering::Equal)),
            FilterOp::Lt => ordering() == Some(Ordering::Less),
            FilterOp::Lte => matches!(ordering(), Some(Ordering::Less | Ordering::Equal)),
            FilterOp::Empty => cell.trim().is_empty(),
            FilterOp::NotEmpty => !cell.trim().is_empty(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn service() -> (tempfile::TempDir, VaultService) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Data")).unwrap();
        std::fs::write(
            dir.path().join("Data/Budget.csv"),
            "Kategori;Bulan;Jumlah\nMakan;Jan;Rp 1.500.000\nTransport;Jan;300000\nMakan;Feb;1.200.000\n",
        )
        .unwrap();
        let svc = VaultService::open(dir.path()).unwrap();
        (dir, svc)
    }

    fn filter(column: &str, op: &str, value: &str) -> SheetFilter {
        SheetFilter {
            column: column.into(),
            op: op.into(),
            value: value.into(),
        }
    }

    fn keyword_reindex(svc: &mut VaultService) -> ReindexReport {
        svc.reindex(ReindexOptions {
            full: false,
            keyword_only: true,
        })
        .unwrap()
    }

    #[test]
    fn resolves_by_path_name_and_stem_and_rejects_escapes() {
        let (_d, svc) = service();
        assert!(svc.resolve_sheet("Data/Budget.csv").is_ok());
        assert!(svc.resolve_sheet("budget").is_ok());
        assert!(svc.resolve_sheet("Budget.CSV").is_ok());
        assert!(svc.resolve_sheet("../x.csv").is_err());
        assert!(svc.resolve_sheet(".trash/x.csv").is_err());
        assert!(svc.resolve_sheet("nope").is_err());
    }

    #[test]
    fn read_pages_and_reports_column_stats() {
        let (_d, svc) = service();
        let data = svc
            .read_sheet(&SheetReadRequest {
                r#ref: "Budget".into(),
                offset: 1,
                limit: 1,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(data.total_rows, 3);
        assert_eq!(data.rows.len(), 1);
        assert_eq!(data.rows[0].row, 2);
        assert_eq!(data.rows[0].cells[0], "Transport");
        let jumlah = &data.columns[2];
        assert!(jumlah.numeric);
        assert_eq!(jumlah.sum, Some(3_000_000.0));
    }

    #[test]
    fn query_filters_sorts_projects_and_aggregates_matches() {
        let (_d, svc) = service();
        let res = svc
            .query_sheet(&SheetQueryRequest {
                r#ref: "Budget".into(),
                filters: vec![filter("kategori", "eq", "makan")],
                columns: Some(vec!["Bulan".into(), "Jumlah".into()]),
                sort_by: Some("Jumlah".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(res.matched, 2);
        assert_eq!(res.headers, vec!["Bulan", "Jumlah"]);
        assert_eq!(res.rows[0].cells, vec!["Feb", "1.200.000"]);
        assert_eq!(res.rows[0].row, 3);
        assert_eq!(res.columns[1].sum, Some(2_700_000.0));

        let gt = SheetQueryRequest {
            r#ref: "Budget".into(),
            filters: vec![filter("Jumlah", ">", "1000000")],
            ..Default::default()
        };
        assert_eq!(svc.query_sheet(&gt).unwrap().matched, 2);
        let bad = SheetQueryRequest {
            r#ref: "Budget".into(),
            filters: vec![filter("Jumlah", "like", "")],
            ..Default::default()
        };
        assert!(svc.query_sheet(&bad).is_err());
    }

    #[test]
    fn writes_keep_delimiter_and_refuse_workbooks() {
        let (d, mut svc) = service();
        let res = svc
            .set_sheet_cell(&SheetSetCellRequest {
                r#ref: "Budget".into(),
                row: 2,
                column: "Jumlah".into(),
                value: "350000".into(),
            })
            .unwrap();
        assert_eq!(res.changed, 1);
        let res = svc
            .append_sheet_rows(&SheetAppendRequest {
                r#ref: "Budget".into(),
                rows: vec![json!(["Hiburan", "Feb", 50000]), json!({"Kategori": "Pulsa"})],
            })
            .unwrap();
        assert_eq!(res.rows, 5);
        let text = std::fs::read_to_string(d.path().join("Data/Budget.csv")).unwrap();
        assert!(text.contains("Transport;Jan;350000\n"));
        assert!(text.ends_with("Hiburan;Feb;50000\nPulsa;;\n"));
        let unknown = SheetAppendRequest {
            r#ref: "Budget".into(),
            rows: vec![json!({"Nope": 1})],
        };
        assert!(svc.append_sheet_rows(&unknown).is_err());

        let created = svc
            .create_sheet(&SheetCreateRequest {
                path: "Data/Baru.csv".into(),
                headers: vec!["A".into(), "B".into()],
                rows: vec![vec!["1".into(), "2".into()]],
            })
            .unwrap();
        assert_eq!((created.rows, created.columns), (1, 2));
        let again = SheetCreateRequest {
            path: "Data/Baru.csv".into(),
            headers: vec!["A".into()],
            rows: vec![],
        };
        assert!(svc.create_sheet(&again).is_err());

        let book = Sheet::from_records("S", vec![vec!["x".into()], vec!["1".into()]]);
        crate::sheet::xlsx_io::export_xlsx(&book, &d.path().join("Book.xlsx")).unwrap();
        let err = svc
            .set_sheet_cell(&SheetSetCellRequest {
                r#ref: "Book.xlsx".into(),
                row: 1,
                column: "x".into(),
                value: "2".into(),
            })
            .unwrap_err();
        assert!(format!("{err:#}").contains("read-only"));
    }

    #[test]
    fn keyword_reindex_covers_sheets_and_search_returns_rows() {
        let (d, mut svc) = service();
        let report = keyword_reindex(&mut svc);
        assert_eq!((report.sheets_indexed, report.sheets_chunked), (1, 1));
        assert_eq!(keyword_reindex(&mut svc).sheets_chunked, 0, "unchanged sheet is skipped");

        let res = svc
            .search(&SearchRequest {
                query: "Transport".into(),
                k: 5,
                semantic: false,
                ..Default::default()
            })
            .unwrap();
        let hit = res.hits.iter().find(|h| h.path == "Data/Budget.csv").unwrap();
        assert_eq!((hit.row, hit.page), (Some(1), None));

        std::fs::remove_file(d.path().join("Data/Budget.csv")).unwrap();
        assert!(keyword_reindex(&mut svc).pruned >= 1);
    }
}
