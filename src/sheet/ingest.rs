//! Sheet → indexable text (§3.8.4). A CSV row like `Kopi,3,12000` means
//! nothing to an embedding model on its own, so each row is rendered as
//! `Item: Kopi; Qty: 3; Harga: 12000`, and consecutive rows are grouped
//! into chunks of at most `CHUNK_CHARS` characters. Every chunk remembers
//! its worksheet and 1-based data-row range so citations can jump back to
//! the exact rows. Pure, no model dependency. Callers: `core::ingestion`
//! (`chunk_sheet`), `app::sheet` and `api::sheet` (size limits).

use super::{Sheet, SheetFile};

/// Target chunk size, roughly in line with `core::chunker`'s default for
/// prose (a few hundred tokens).
pub const CHUNK_CHARS: usize = 1200;
/// Rows beyond this are not indexed (large exports would flood the index
/// and the embedding worker); the sheet stays fully usable in the editor.
pub const MAX_INDEXED_ROWS: usize = 5_000;
/// Files larger than this are not indexed at all (parsing them would
/// hold the whole file in memory on the indexing thread).
pub const MAX_INDEXED_BYTES: u64 = 64 * 1024 * 1024;
/// Cells longer than this are cut in the rendered row text.
const MAX_CELL_CHARS: usize = 200;

/// One group of rows ready to embed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetChunk {
    /// Worksheet name (`None` for single-sheet files).
    pub sheet: Option<String>,
    /// 1-based data-row numbers (the header row is not counted); both 0
    /// for a header-only chunk.
    pub first_row: usize,
    pub last_row: usize,
    pub text: String,
}

/// `Header: value; …` for one row, skipping empty cells. Returns `None`
/// for rows with nothing in them.
pub fn row_text(sheet: &Sheet, row: usize) -> Option<String> {
    let cells = sheet.rows.get(row)?;
    let parts: Vec<String> = cells
        .iter()
        .zip(&sheet.headers)
        .filter(|(value, _)| !value.trim().is_empty())
        .map(|(value, header)| {
            let value: String = value
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(MAX_CELL_CHARS)
                .collect();
            format!("{}: {value}", header.trim())
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// Chunks every worksheet of `file`.
pub fn chunk_sheet_file(file: &SheetFile) -> Vec<SheetChunk> {
    let multi = file.sheets.len() > 1;
    file.sheets
        .iter()
        .flat_map(|sheet| chunk_sheet(sheet, multi.then(|| sheet.name.clone())))
        .collect()
}

/// Chunks one sheet. Every chunk starts with a `Columns: …` line so even
/// a chunk of sparse rows carries the table's shape.
pub fn chunk_sheet(sheet: &Sheet, sheet_label: Option<String>) -> Vec<SheetChunk> {
    let columns = format!("Columns: {}", sheet.headers.join(", "));
    let mut out = Vec::new();
    let mut current = String::new();
    let (mut first_row, mut last_row) = (0, 0);
    for i in 0..sheet.rows.len().min(MAX_INDEXED_ROWS) {
        let Some(line) = row_text(sheet, i) else { continue };
        if !current.is_empty() && current.len() + line.len() + 1 > CHUNK_CHARS {
            out.push(SheetChunk {
                sheet: sheet_label.clone(),
                first_row,
                last_row,
                text: std::mem::take(&mut current),
            });
        }
        if current.is_empty() {
            current.push_str(&columns);
            first_row = i + 1;
        }
        current.push('\n');
        current.push_str(&line);
        last_row = i + 1;
    }
    if !current.is_empty() {
        out.push(SheetChunk {
            sheet: sheet_label.clone(),
            first_row,
            last_row,
            text: current,
        });
    }
    // A header-only sheet is still worth finding by its columns.
    if out.is_empty() && !sheet.headers.is_empty() {
        out.push(SheetChunk {
            sheet: sheet_label,
            first_row: 0,
            last_row: 0,
            text: columns,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(rows: usize) -> Sheet {
        let mut records = vec![vec!["Item".to_string(), "Harga".to_string()]];
        for i in 0..rows {
            records.push(vec![format!("Barang {i}"), format!("{}", i * 1000)]);
        }
        Sheet::from_records("Belanja", records)
    }

    #[test]
    fn row_text_labels_values_and_skips_blanks() {
        let mut s = sheet(1);
        assert_eq!(row_text(&s, 0).unwrap(), "Item: Barang 0; Harga: 0");
        s.set_cell(0, 1, "  ".into());
        assert_eq!(row_text(&s, 0).unwrap(), "Item: Barang 0");
        s.set_cell(0, 0, String::new());
        assert_eq!(row_text(&s, 0), None);
    }

    #[test]
    fn chunks_cover_all_rows_in_order_within_budget() {
        let chunks = chunk_sheet(&sheet(200), None);
        assert!(chunks.len() > 1);
        assert_eq!(chunks[0].first_row, 1);
        assert_eq!(chunks.last().unwrap().last_row, 200);
        for pair in chunks.windows(2) {
            assert_eq!(pair[0].last_row + 1, pair[1].first_row);
        }
        for c in &chunks {
            assert!(c.text.starts_with("Columns: Item, Harga\n"));
            assert!(c.text.len() <= CHUNK_CHARS);
        }
    }

    #[test]
    fn header_only_sheet_still_indexes_columns() {
        let chunks = chunk_sheet(&sheet(0), Some("S".into()));
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "Columns: Item, Harga");
        assert_eq!(chunks[0].sheet.as_deref(), Some("S"));
    }
}
