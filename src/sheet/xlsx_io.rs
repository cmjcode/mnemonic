//! Workbook import/export (§3.8.1). `read_workbook` loads every worksheet
//! of an XLSX/XLSM/XLSB/XLS/ODS file through `calamine` as read-only
//! `Sheet`s (cached formula results, dates as ISO text). Workbooks are
//! never written back — that would drop formulas, styles, charts and
//! macros — so the only writer is `export_xlsx`, which builds a *new*
//! file from a sheet (numeric cells as numbers). Callers: `sheet::load`,
//! `app::sheet`, `api::sheet`.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use calamine::{Data, Reader, open_workbook_auto};
use rust_xlsxwriter::Workbook;

use super::model::{Sheet, parse_number};
use crate::notes::note::write_atomic;

/// Excel's limits: 31 chars, none of `[]:*?/\`.
const MAX_SHEET_NAME: usize = 31;

/// All worksheets of the workbook at `path`, in tab order. The first row of
/// each is its header row; trailing empty rows are dropped.
pub fn read_workbook(path: &Path) -> Result<Vec<Sheet>> {
    let mut workbook = open_workbook_auto(path)
        .with_context(|| format!("opening workbook {}", path.display()))?;
    let mut sheets = Vec::new();
    for name in workbook.sheet_names() {
        let range = match workbook.worksheet_range(&name) {
            Ok(range) => range,
            Err(e) => {
                log::warn!("skipping worksheet `{name}` of {}: {e:?}", path.display());
                continue;
            }
        };
        let records: Vec<Vec<String>> = range
            .rows()
            .map(|row| row.iter().map(cell_to_string).collect())
            .collect();
        sheets.push(Sheet::from_records(name, trim_trailing_empty(records)));
    }
    if sheets.is_empty() {
        return Err(anyhow!("{} has no readable worksheets", path.display()));
    }
    Ok(sheets)
}

/// Drops fully empty rows at the end (common after deleted data).
fn trim_trailing_empty(mut records: Vec<Vec<String>>) -> Vec<Vec<String>> {
    while records
        .last()
        .is_some_and(|r| r.iter().all(|c| c.trim().is_empty()))
    {
        records.pop();
    }
    records
}

/// Text form of one cell: integers without `.0`, dates as `YYYY-MM-DD`
/// (plus ` HH:MM:SS` when there's a time part), errors as `#ERR …`.
pub fn cell_to_string(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => f.to_string(),
        Data::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Data::DateTime(dt) if dt.is_datetime() => match dt.as_datetime() {
            Some(dt) if dt.time() == chrono::NaiveTime::MIN => dt.format("%Y-%m-%d").to_string(),
            Some(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            None => cell.to_string(),
        },
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#ERR {e}"),
        other => other.to_string(),
    }
}

/// Excel-safe worksheet name derived from `name`.
fn worksheet_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if "[]:*?/\\".contains(c) { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_matches('\'');
    let truncated: String = cleaned.chars().take(MAX_SHEET_NAME).collect();
    if truncated.is_empty() {
        "Sheet1".to_string()
    } else {
        truncated
    }
}

/// Builds an XLSX workbook (one worksheet) from `sheet` and atomically
/// writes it to `path`. Unambiguous numbers are written as numbers so the
/// result is immediately usable in Excel; everything else stays text.
pub fn export_xlsx(sheet: &Sheet, path: &Path) -> Result<()> {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet
        .set_name(worksheet_name(&sheet.name))
        .context("naming worksheet")?;
    for (c, header) in sheet.headers.iter().enumerate() {
        worksheet
            .write_string(0, col_num(c)?, header)
            .context("writing header")?;
    }
    for (r, row) in sheet.rows.iter().enumerate() {
        let r = u32::try_from(r + 1).context("too many rows for XLSX")?;
        for (c, cell) in row.iter().enumerate() {
            if cell.is_empty() {
                continue;
            }
            let c = col_num(c)?;
            let written = match parse_number(cell) {
                Some(n) if looks_plain_number(cell) => worksheet.write_number(r, c, n),
                _ => worksheet.write_string(r, c, cell),
            };
            written.with_context(|| format!("writing cell {r}:{c}"))?;
        }
    }
    let bytes = workbook
        .save_to_buffer()
        .context("serializing XLSX workbook")?;
    write_atomic(path, &bytes).with_context(|| format!("writing {}", path.display()))
}

fn col_num(c: usize) -> Result<u16> {
    u16::try_from(c).context("too many columns for XLSX")
}

/// Only unambiguous numbers become numeric cells; values like `007`,
/// `Rp 25.000` or `12%` keep their text so nothing the user sees changes.
fn looks_plain_number(cell: &str) -> bool {
    let s = cell.trim();
    let digits = s.strip_prefix('-').unwrap_or(s);
    let leading_zero = digits.len() > 1 && digits.starts_with('0') && !digits.starts_with("0.");
    !leading_zero
        && s.chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | 'e' | 'E'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn export_then_read_round_trips_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.xlsx");
        let sheet = Sheet::from_records(
            "Data: Q3/2026",
            vec![
                s(&["Item", "Qty", "Kode"]),
                s(&["Kopi", "3", "007"]),
                s(&["Teh", "2.5", ""]),
            ],
        );
        export_xlsx(&sheet, &path).unwrap();
        let back = read_workbook(&path).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].name, "Data_ Q3_2026");
        assert_eq!(back[0].headers, sheet.headers);
        assert_eq!(back[0].rows, vec![s(&["Kopi", "3", "007"]), s(&["Teh", "2.5", ""])]);
    }

    #[test]
    fn cell_formatting() {
        assert_eq!(cell_to_string(&Data::Float(3.0)), "3");
        assert_eq!(cell_to_string(&Data::Float(0.25)), "0.25");
        assert_eq!(cell_to_string(&Data::Bool(true)), "TRUE");
        assert_eq!(cell_to_string(&Data::Empty), "");
    }

    #[test]
    fn worksheet_names_and_numeric_detection() {
        assert_eq!(worksheet_name(""), "Sheet1");
        assert_eq!(worksheet_name(&"x".repeat(40)).len(), 31);
        assert!(looks_plain_number("12.5") && !looks_plain_number("007"));
        assert!(!looks_plain_number("12%") && looks_plain_number("0.5"));
    }

    #[test]
    fn missing_workbook_is_an_error() {
        assert!(read_workbook(Path::new("/nonexistent/x.xlsx")).is_err());
    }
}
