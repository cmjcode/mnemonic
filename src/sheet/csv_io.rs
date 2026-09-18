//! CSV/TSV read + write (§3.8.1). The file on disk stays the source of
//! truth, so this module works hard to write back what it read: the
//! detected delimiter, UTF-8 BOM and line ending are kept in a
//! `CsvFormat` and reused on save (quoting is normalized to "only when
//! needed"), and saves go through the same temp-file + rename as notes
//! (`notes::note::write_atomic`). A file that isn't valid UTF-8 loads
//! lossily and is flagged so it's never saved back over the original.
//! Callers: `sheet::load`, `app::sheet`, `api::sheet`.

use std::path::Path;

use anyhow::{Context, Result, bail};

use super::model::Sheet;
use crate::notes::note::write_atomic;

const UTF8_BOM: &str = "\u{feff}";
const DELIMITERS: [u8; 4] = [b',', b';', b'\t', b'|'];
/// Lines sampled when guessing the delimiter.
const SNIFF_LINES: usize = 50;

/// How a CSV file was written, reused verbatim when saving it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsvFormat {
    pub delimiter: u8,
    pub bom: bool,
    pub crlf: bool,
    /// The bytes weren't valid UTF-8 and were decoded lossily; saving
    /// would corrupt them, so `save_csv` refuses.
    pub lossy: bool,
}

impl CsvFormat {
    /// Defaults for a new file: comma (tab for `.tsv`), no BOM, LF.
    pub fn for_path(path: &Path) -> CsvFormat {
        CsvFormat {
            delimiter: if is_tsv(path) { b'\t' } else { b',' },
            bom: false,
            crlf: false,
            lossy: false,
        }
    }
}

fn is_tsv(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("tsv") || e.eq_ignore_ascii_case("tab"))
}

/// Reads and parses `path`. The sheet is named after the file stem.
pub fn load_csv(path: &Path) -> Result<(Sheet, CsvFormat)> {
    let bytes =
        std::fs::read(path).with_context(|| format!("reading sheet {}", path.display()))?;
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let (text, lossy) = match String::from_utf8(bytes) {
        Ok(text) => (text, false),
        Err(e) => {
            log::warn!("{} is not valid UTF-8; opening read-only", path.display());
            (String::from_utf8_lossy(e.as_bytes()).into_owned(), true)
        }
    };
    let (sheet, mut format) = parse_csv(&name, &text, is_tsv(path))
        .with_context(|| format!("parsing sheet {}", path.display()))?;
    format.lossy = lossy;
    Ok((sheet, format))
}

/// Parses CSV text, sniffing the delimiter (`prefer_tab` breaks ties for
/// `.tsv` files) and treating the first record as the header row.
pub fn parse_csv(name: &str, text: &str, prefer_tab: bool) -> Result<(Sheet, CsvFormat)> {
    let bom = text.starts_with(UTF8_BOM);
    let body = text.strip_prefix(UTF8_BOM).unwrap_or(text);
    let delimiter = sniff_delimiter(body, prefer_tab);
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(body.as_bytes());
    let mut records = Vec::new();
    for (i, record) in reader.records().enumerate() {
        let record = record.with_context(|| format!("record {}", i + 1))?;
        records.push(record.iter().map(str::to_string).collect::<Vec<_>>());
    }
    let format = CsvFormat {
        delimiter,
        bom,
        crlf: body.contains("\r\n"),
        lossy: false,
    };
    Ok((Sheet::from_records(name, records), format))
}

/// Picks the delimiter that yields the most records with the same field
/// count as the first one (then the most fields). Falls back to
/// tab/comma when no candidate splits the header.
fn sniff_delimiter(text: &str, prefer_tab: bool) -> u8 {
    let sample: String = text.lines().take(SNIFF_LINES).collect::<Vec<_>>().join("\n");
    let fallback = if prefer_tab { b'\t' } else { b',' };
    // (consistent records, fields in first record, preferred, delimiter)
    let mut best: Option<(usize, usize, bool, u8)> = None;
    for delim in DELIMITERS {
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(delim)
            .has_headers(false)
            .flexible(true)
            .from_reader(sample.as_bytes());
        let counts: Vec<usize> = reader.records().filter_map(|r| r.ok()).map(|r| r.len()).collect();
        let Some(&first) = counts.first() else { continue };
        if first < 2 {
            continue;
        }
        let consistent = counts.iter().filter(|&&c| c == first).count();
        let candidate = (consistent, first, delim == fallback, delim);
        if best.is_none_or(|b| candidate > b) {
            best = Some(candidate);
        }
    }
    best.map(|(.., d)| d).unwrap_or(fallback)
}

/// Serializes `sheet` (header row first) in `format`.
pub fn to_csv_bytes(sheet: &Sheet, format: &CsvFormat) -> Result<Vec<u8>> {
    let mut writer = csv::WriterBuilder::new()
        .delimiter(format.delimiter)
        .flexible(true)
        .terminator(if format.crlf {
            csv::Terminator::CRLF
        } else {
            csv::Terminator::Any(b'\n')
        })
        .from_writer(if format.bom {
            UTF8_BOM.as_bytes().to_vec()
        } else {
            Vec::new()
        });
    if !sheet.headers.is_empty() {
        writer.write_record(&sheet.headers).context("writing header row")?;
    }
    for (i, row) in sheet.rows.iter().enumerate() {
        writer
            .write_record(row)
            .with_context(|| format!("writing row {}", i + 1))?;
    }
    writer.into_inner().context("flushing CSV buffer")
}

/// Atomically writes `sheet` to `path`. Refuses files that were decoded
/// lossily, since the save would replace their original bytes.
pub fn save_csv(path: &Path, sheet: &Sheet, format: &CsvFormat) -> Result<()> {
    if format.lossy {
        bail!(
            "{} is not UTF-8; refusing to overwrite it (convert it to UTF-8 first)",
            path.display()
        );
    }
    let bytes = to_csv_bytes(sheet, format)?;
    write_atomic(path, &bytes).with_context(|| format!("saving sheet {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_semicolon_and_keeps_bom_and_crlf() {
        let text = "\u{feff}Nama;Harga\r\nKopi;\"12,5\"\r\nTeh;8\r\n";
        let (sheet, format) = parse_csv("x", text, false).unwrap();
        assert_eq!(format.delimiter, b';');
        assert!(format.bom && format.crlf);
        assert_eq!(sheet.headers, vec!["Nama", "Harga"]);
        assert_eq!(sheet.rows[0], vec!["Kopi", "12,5"]);
        let out = String::from_utf8(to_csv_bytes(&sheet, &format).unwrap()).unwrap();
        // Quoting is normalized to "only when needed".
        assert_eq!(out, "\u{feff}Nama;Harga\r\nKopi;12,5\r\nTeh;8\r\n");
    }

    #[test]
    fn comma_with_quoted_newlines_round_trips() {
        let text = "a,b\n\"multi\nline\",\"has, comma\"\n";
        let (sheet, format) = parse_csv("x", text, false).unwrap();
        assert_eq!(format.delimiter, b',');
        assert_eq!(sheet.rows[0], vec!["multi\nline", "has, comma"]);
        assert_eq!(String::from_utf8(to_csv_bytes(&sheet, &format).unwrap()).unwrap(), text);
    }

    #[test]
    fn tab_and_single_column_fallbacks() {
        let (_, f) = parse_csv("x", "a\tb\n1\t2\n", false).unwrap();
        assert_eq!(f.delimiter, b'\t');
        let (sheet, f) = parse_csv("x", "only\nvalue\n", true).unwrap();
        assert_eq!(f.delimiter, b'\t');
        assert_eq!(sheet.headers, vec!["only"]);
        let (sheet, _) = parse_csv("x", "", false).unwrap();
        assert_eq!((sheet.width(), sheet.height()), (0, 0));
    }

    #[test]
    fn save_is_atomic_and_refuses_lossy_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.csv");
        std::fs::write(&path, b"a,b\n\xff,2\n").unwrap();
        let (mut sheet, format) = load_csv(&path).unwrap();
        assert!(format.lossy);
        assert!(save_csv(&path, &sheet, &format).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"a,b\n\xff,2\n");

        let format = CsvFormat::for_path(&path);
        sheet.set_cell(0, 0, "x".into());
        save_csv(&path, &sheet, &format).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a,b\nx,2\n");
        let leftovers = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(leftovers, 1);
    }
}
