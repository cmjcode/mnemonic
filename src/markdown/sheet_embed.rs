//! `![[data.csv]]` in a note (§3.8.3): renders the first rows of a sheet
//! as a Markdown table that the normal renderer then draws, followed by
//! a `[[link]]` to open the whole sheet. The preview is cached per file
//! stamp (mtime + size), so the file is only re-read when it changes —
//! not on every frame. Numeric columns are right-aligned. Callers:
//! `markdown::renderer::transform_note_embeds`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use crate::sheet;

/// Data rows shown in an embedded preview.
pub const PREVIEW_ROWS: usize = 12;
/// Columns shown; wider sheets get a trailing `…` column.
const PREVIEW_COLUMNS: usize = 10;
/// Characters per cell in the preview.
const PREVIEW_CELL_CHARS: usize = 60;

type Stamp = (Option<SystemTime>, u64);

fn cache() -> &'static Mutex<HashMap<PathBuf, (Stamp, String)>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, (Stamp, String)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Markdown for embedding the sheet at `path`; `link` is the target text
/// used in the trailing "open" wikilink (what the user wrote).
pub fn preview_markdown(path: &Path, link: &str) -> String {
    let meta = std::fs::metadata(path).ok();
    let stamp: Stamp = (
        meta.as_ref().and_then(|m| m.modified().ok()),
        meta.as_ref().map(|m| m.len()).unwrap_or(0),
    );
    if let Ok(cache) = cache().lock()
        && let Some((cached, md)) = cache.get(path)
        && *cached == stamp
    {
        return md.clone();
    }
    let md = build_preview(path, link);
    if let Ok(mut cache) = cache().lock() {
        cache.insert(path.to_path_buf(), (stamp, md.clone()));
    }
    md
}

fn build_preview(path: &Path, link: &str) -> String {
    match sheet::load(path) {
        Ok(file) => match file.sheet(None) {
            Some((_, sh)) => table_markdown(sh, link),
            None => format!("[[{link}]]\n"),
        },
        Err(e) => {
            log::warn!("markdown: embedding sheet {} failed: {e:#}", path.display());
            format!("> [!warning] [[{link}]]\n> {e}\n")
        }
    }
}

/// The preview table for one sheet (pure, for tests).
pub fn table_markdown(sh: &sheet::Sheet, link: &str) -> String {
    if sh.width() == 0 {
        return format!("[[{link}]]\n");
    }
    let cols = sh.width().min(PREVIEW_COLUMNS);
    let more_cols = sh.width() > cols;
    let mut header: Vec<String> = sh.headers[..cols].iter().map(|h| cell(h)).collect();
    let mut rule: Vec<&str> = (0..cols)
        .map(|c| if sh.column_stats(c).is_numeric() { "---:" } else { "---" })
        .collect();
    if more_cols {
        header.push("…".into());
        rule.push("---");
    }
    let mut out = format!("| {} |\n| {} |\n", header.join(" | "), rule.join(" | "));
    for row in sh.rows.iter().take(PREVIEW_ROWS) {
        let mut cells: Vec<String> = row[..cols].iter().map(|c| cell(c)).collect();
        if more_cols {
            cells.push(String::new());
        }
        out.push_str(&format!("| {} |\n", cells.join(" | ")));
    }
    let shown = sh.height().min(PREVIEW_ROWS);
    out.push_str(&format!(
        "\n*[[{link}]] · {shown}/{} rows · {} columns*\n",
        sh.height(),
        sh.width()
    ));
    out
}

/// One table cell: single line, pipes escaped, length-capped.
fn cell(text: &str) -> String {
    let flat: String = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(PREVIEW_CELL_CHARS)
        .collect();
    let escaped = flat.replace('\\', "\\\\").replace('|', "\\|");
    if text.chars().count() > PREVIEW_CELL_CHARS {
        format!("{escaped}…")
    } else {
        escaped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records(rows: usize) -> sheet::Sheet {
        let mut r = vec![vec!["Item".to_string(), "Harga".to_string()]];
        for i in 0..rows {
            r.push(vec![format!("a|b {i}"), format!("{}", i * 10)]);
        }
        sheet::Sheet::from_records("x", r)
    }

    #[test]
    fn table_escapes_pipes_aligns_numbers_and_caps_rows() {
        let md = table_markdown(&records(20), "Kas.csv");
        let lines: Vec<&str> = md.lines().collect();
        assert_eq!(lines[0], "| Item | Harga |");
        assert_eq!(lines[1], "| --- | ---: |");
        assert_eq!(lines[2], "| a\\|b 0 | 0 |");
        assert_eq!(lines.len(), 2 + PREVIEW_ROWS + 2);
        assert!(md.ends_with("*[[Kas.csv]] · 12/20 rows · 2 columns*\n"));
    }

    #[test]
    fn preview_reads_file_and_caches_until_it_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k.csv");
        std::fs::write(&path, "A\n1\n").unwrap();
        assert!(preview_markdown(&path, "k.csv").contains("| 1 |"));
        std::fs::write(&path, "A\n1\n22\n").unwrap();
        assert!(preview_markdown(&path, "k.csv").contains("| 22 |"));
        let missing = preview_markdown(&dir.path().join("nope.csv"), "nope.csv");
        assert!(missing.starts_with("> [!warning] [[nope.csv]]"));
    }
}
