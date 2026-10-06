//! In-memory sheet model (§3.8.2): a header row plus string cells, and the
//! edits the grid editor and the agent interface apply to it. Cells stay
//! strings so a load → save round trip never reformats what the user
//! typed; numbers are only parsed on demand (sorting, footer aggregates,
//! XLSX export). Pure, no egui. Callers: `sheet::csv_io`, `sheet::xlsx_io`,
//! `sheet::ingest`, `app::sheet`, `api::sheet`.

use std::cmp::Ordering;

/// One table: `headers.len()` columns, every row padded to that width.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sheet {
    /// Worksheet name (XLSX tab), or the file stem for CSV.
    pub name: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// Footer statistics over the numeric cells of one column (§3.8.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnStats {
    /// Non-empty cells.
    pub filled: usize,
    /// Cells that parsed as numbers.
    pub numeric: usize,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
}

impl ColumnStats {
    pub fn avg(&self) -> Option<f64> {
        (self.numeric > 0).then(|| self.sum / self.numeric as f64)
    }

    /// A column counts as numeric when most of its filled cells are.
    pub fn is_numeric(&self) -> bool {
        self.numeric > 0 && self.numeric * 2 >= self.filled
    }
}

impl Sheet {
    /// Builds a sheet from raw records, treating the first as the header
    /// row and padding everything to the widest record so ragged CSVs
    /// still load. Blank header names become `Column N`.
    pub fn from_records(name: impl Into<String>, mut records: Vec<Vec<String>>) -> Sheet {
        let width = records.iter().map(Vec::len).max().unwrap_or(0);
        let mut headers = if records.is_empty() {
            Vec::new()
        } else {
            records.remove(0)
        };
        headers.resize(width, String::new());
        for (i, h) in headers.iter_mut().enumerate() {
            if h.trim().is_empty() {
                *h = default_header(i);
            }
        }
        for row in &mut records {
            row.resize(width, String::new());
        }
        Sheet {
            name: name.into(),
            headers,
            rows: records,
        }
    }

    pub fn width(&self) -> usize {
        self.headers.len()
    }

    pub fn height(&self) -> usize {
        self.rows.len()
    }

    pub fn cell(&self, row: usize, col: usize) -> Option<&str> {
        self.rows.get(row)?.get(col).map(String::as_str)
    }

    /// Index of the column named `name` (exact first, then
    /// case-insensitive), or a 0-based numeric index.
    pub fn column_index(&self, name: &str) -> Option<usize> {
        let name = name.trim();
        self.headers
            .iter()
            .position(|h| h == name)
            .or_else(|| self.headers.iter().position(|h| h.eq_ignore_ascii_case(name)))
            .or_else(|| name.parse::<usize>().ok().filter(|i| *i < self.width()))
    }

    /// Replaces a cell, returning the previous value (`None` when out of
    /// range, in which case nothing changes).
    pub fn set_cell(&mut self, row: usize, col: usize, value: String) -> Option<String> {
        let slot = self.rows.get_mut(row)?.get_mut(col)?;
        Some(std::mem::replace(slot, value))
    }

    /// Inserts a row at `at` (clamped), padded or truncated to the sheet
    /// width. Returns the index it landed at.
    pub fn insert_row(&mut self, at: usize, mut row: Vec<String>) -> usize {
        row.resize(self.width(), String::new());
        let at = at.min(self.rows.len());
        self.rows.insert(at, row);
        at
    }

    pub fn remove_row(&mut self, at: usize) -> Option<Vec<String>> {
        (at < self.rows.len()).then(|| self.rows.remove(at))
    }

    /// Inserts a column at `at` (clamped). `cells` fills existing rows in
    /// order; missing values are empty.
    pub fn insert_column(&mut self, at: usize, header: String, cells: Vec<String>) -> usize {
        let at = at.min(self.width());
        self.headers.insert(at, header);
        let mut cells = cells.into_iter();
        for row in &mut self.rows {
            row.insert(at, cells.next().unwrap_or_default());
        }
        at
    }

    /// Removes column `at`, returning its header and cells.
    pub fn remove_column(&mut self, at: usize) -> Option<(String, Vec<String>)> {
        if at >= self.width() {
            return None;
        }
        let header = self.headers.remove(at);
        let cells = self.rows.iter_mut().map(|r| r.remove(at)).collect();
        Some((header, cells))
    }

    /// Stable sort of all rows by column `col` (see `sorted_order`).
    /// Returns the permutation applied (`new[i] = old[perm[i]]`) so the
    /// editor can undo it.
    pub fn sort_by_column(&mut self, col: usize, ascending: bool) -> Vec<usize> {
        let perm = self.sorted_order(col, ascending);
        self.apply_permutation(&perm);
        perm
    }

    /// Row indices in sorted order by column `col`, without moving any
    /// data (read-only sheets sort only their view). Numeric when the
    /// column is numeric (non-numbers last), otherwise case-insensitive
    /// text (blanks last). Stable.
    pub fn sorted_order(&self, col: usize, ascending: bool) -> Vec<usize> {
        let mut perm: Vec<usize> = (0..self.rows.len()).collect();
        if col >= self.width() {
            return perm;
        }
        let numeric = self.column_stats(col).is_numeric();
        let rows = &self.rows;
        perm.sort_by(|&a, &b| {
            let (x, y) = (rows[a][col].as_str(), rows[b][col].as_str());
            let ord = if numeric {
                match (parse_number(x), parse_number(y)) {
                    (Some(p), Some(q)) => p.partial_cmp(&q).unwrap_or(Ordering::Equal),
                    (Some(_), None) => return Ordering::Less,
                    (None, Some(_)) => return Ordering::Greater,
                    (None, None) => compare_text(x, y),
                }
            } else {
                match (x.trim().is_empty(), y.trim().is_empty()) {
                    (false, true) => return Ordering::Less,
                    (true, false) => return Ordering::Greater,
                    _ => compare_text(x, y),
                }
            };
            if ascending { ord } else { ord.reverse() }
        });
        perm
    }

    /// Reorders rows so that `new[i] = old[perm[i]]`.
    pub fn apply_permutation(&mut self, perm: &[usize]) {
        let mut old: Vec<Option<Vec<String>>> =
            std::mem::take(&mut self.rows).into_iter().map(Some).collect();
        self.rows = perm.iter().filter_map(|&i| old.get_mut(i)?.take()).collect();
        // Rows not named by `perm` (shouldn't happen) are kept, not lost.
        self.rows.extend(old.into_iter().flatten());
    }

    /// Reverses `apply_permutation(perm)`.
    pub fn undo_permutation(&mut self, perm: &[usize]) {
        let mut inverse = vec![0; perm.len()];
        for (new, &old) in perm.iter().enumerate() {
            if let Some(slot) = inverse.get_mut(old) {
                *slot = new;
            }
        }
        self.apply_permutation(&inverse);
    }

    /// Indices of rows with any cell containing `needle`
    /// (case-insensitive). An empty needle matches every row.
    pub fn filter_rows(&self, needle: &str) -> Vec<usize> {
        let needle = needle.trim().to_lowercase();
        (0..self.rows.len())
            .filter(|&i| {
                needle.is_empty()
                    || self.rows[i]
                        .iter()
                        .any(|c| c.to_lowercase().contains(&needle))
            })
            .collect()
    }

    pub fn column_stats(&self, col: usize) -> ColumnStats {
        self.column_stats_over(col, 0..self.rows.len())
    }

    /// Stats of column `col` over the given row indices (e.g. the rows a
    /// filter left visible).
    pub fn column_stats_over(
        &self,
        col: usize,
        rows: impl IntoIterator<Item = usize>,
    ) -> ColumnStats {
        let mut stats = ColumnStats {
            filled: 0,
            numeric: 0,
            sum: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        };
        for r in rows {
            let Some(cell) = self.cell(r, col) else { continue };
            if cell.trim().is_empty() {
                continue;
            }
            stats.filled += 1;
            if let Some(n) = parse_number(cell) {
                stats.numeric += 1;
                stats.sum += n;
                stats.min = stats.min.min(n);
                stats.max = stats.max.max(n);
            }
        }
        stats
    }
}

pub fn default_header(i: usize) -> String {
    format!("Column {}", i + 1)
}

fn compare_text(a: &str, b: &str) -> Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| a.cmp(b))
}

/// Parses a cell as a number, accepting both `1,234.5` and the Indonesian
/// `1.234,5` grouping, a leading currency marker (`Rp`, `$`, `€`, `£`) and
/// a trailing `%` (kept as the literal number: `12%` → 12). After `Rp` a
/// lone dot is always grouping (`Rp 25.000` → 25000). Returns `None` for
/// anything else, including empty cells and dates.
pub fn parse_number(cell: &str) -> Option<f64> {
    let mut s = cell.trim();
    let mut rupiah = false;
    for prefix in ["Rp.", "Rp", "rp", "IDR", "$", "€", "£"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            rupiah = prefix.eq_ignore_ascii_case("rp") || prefix == "Rp." || prefix == "IDR";
            s = rest.trim_start();
            break;
        }
    }
    s = s.strip_suffix('%').unwrap_or(s).trim_end();
    if s.is_empty() || !s.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let (sign, digits) = match s.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", s.strip_prefix('+').unwrap_or(s)),
    };
    if !digits
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | ' ' | '_' | 'e' | 'E' | '-' | '+'))
    {
        return None;
    }
    let digits: String = digits.chars().filter(|c| !matches!(c, ' ' | '_')).collect();
    let is_grouping = |groups: &[&str]| {
        groups.len() > 1 && !groups[0].is_empty() && groups[1..].iter().all(|g| g.len() == 3)
    };
    let normalized = match (digits.rfind('.'), digits.rfind(',')) {
        // Both present: whichever comes last is the decimal separator.
        (Some(dot), Some(comma)) if comma > dot => digits.replace('.', "").replace(',', "."),
        (Some(_), Some(_)) => digits.replace(',', ""),
        (None, Some(_)) => {
            let groups: Vec<&str> = digits.split(',').collect();
            if is_grouping(&groups) && (groups.len() > 2 || groups[0] != "0") && !rupiah {
                digits.replace(',', "")
            } else if groups.len() == 2 {
                digits.replace(',', ".")
            } else {
                return None;
            }
        }
        (Some(_), None) => {
            let groups: Vec<&str> = digits.split('.').collect();
            if groups.len() > 2 || (rupiah && is_grouping(&groups)) {
                // `1.234.567` / `Rp 25.000`: dots can only be grouping.
                if !is_grouping(&groups) {
                    return None;
                }
                digits.replace('.', "")
            } else {
                digits
            }
        }
        (None, None) => digits,
    };
    format!("{sign}{normalized}")
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
}

/// Formats a number for display in the footer / exports without noisy
/// trailing zeros (`3` not `3.0`, `2.5` not `2.500000`).
pub fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        let s = format!("{n:.6}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn sample() -> Sheet {
        Sheet::from_records(
            "t",
            vec![
                s(&["Nama", "Jumlah"]),
                s(&["b", "10"]),
                s(&["A", "2"]),
                s(&["c", ""]),
                s(&["d", "1.000,5"]),
            ],
        )
    }

    #[test]
    fn from_records_pads_ragged_rows_and_names_blank_headers() {
        let sheet = Sheet::from_records("x", vec![s(&["a"]), s(&["1", "2", "3"]), s(&[])]);
        assert_eq!(sheet.headers, s(&["a", "Column 2", "Column 3"]));
        assert_eq!(sheet.rows, vec![s(&["1", "2", "3"]), s(&["", "", ""])]);
        assert_eq!(Sheet::from_records("e", vec![]).width(), 0);
    }

    #[test]
    fn parse_number_handles_both_groupings() {
        assert_eq!(parse_number("1,234.5"), Some(1234.5));
        assert_eq!(parse_number("1.234,5"), Some(1234.5));
        assert_eq!(parse_number("1.234.567"), Some(1_234_567.0));
        assert_eq!(parse_number("1,234"), Some(1234.0));
        assert_eq!(parse_number("0,500"), Some(0.5));
        assert_eq!(parse_number("1,5"), Some(1.5));
        assert_eq!(parse_number("1.5"), Some(1.5));
        assert_eq!(parse_number("Rp 25.000"), Some(25_000.0));
        assert_eq!(parse_number("Rp 25.000,50"), Some(25_000.5));
        assert_eq!(parse_number("$1,200"), Some(1200.0));
        assert_eq!(parse_number("-3"), Some(-3.0));
        assert_eq!(parse_number("12%"), Some(12.0));
        assert_eq!(parse_number("1e3"), Some(1000.0));
        assert_eq!(parse_number("abc"), None);
        assert_eq!(parse_number("2024-01-05"), None);
        assert_eq!(parse_number("1,23,4"), None);
        assert_eq!(parse_number(""), None);
    }

    #[test]
    fn stats_and_numeric_detection() {
        let sheet = sample();
        let st = sheet.column_stats(1);
        assert_eq!((st.filled, st.numeric), (3, 3));
        assert_eq!(st.sum, 1012.5);
        assert_eq!((st.min, st.max), (2.0, 1000.5));
        assert_eq!(st.avg(), Some(337.5));
        assert!(st.is_numeric());
        assert!(!sheet.column_stats(0).is_numeric());
        assert_eq!(sheet.column_stats_over(1, [0, 1]).sum, 12.0);
    }

    #[test]
    fn numeric_sort_puts_blanks_last_and_undo_restores() {
        let mut sheet = sample();
        let before = sheet.rows.clone();
        let perm = sheet.sort_by_column(1, true);
        let col: Vec<&str> = sheet.rows.iter().map(|r| r[1].as_str()).collect();
        assert_eq!(col, vec!["2", "10", "1.000,5", ""]);
        sheet.undo_permutation(&perm);
        assert_eq!(sheet.rows, before);

        sheet.sort_by_column(0, false);
        let col: Vec<&str> = sheet.rows.iter().map(|r| r[0].as_str()).collect();
        assert_eq!(col, vec!["d", "c", "b", "A"]);
    }

    #[test]
    fn row_and_column_edits() {
        let mut sheet = sample();
        assert_eq!(sheet.set_cell(0, 1, "11".into()), Some("10".into()));
        assert_eq!(sheet.set_cell(99, 0, "x".into()), None);
        sheet.insert_row(99, s(&["e"]));
        assert_eq!(sheet.rows.last().unwrap(), &s(&["e", ""]));
        sheet.insert_column(1, "Kota".into(), s(&["Bdg"]));
        assert_eq!(sheet.headers, s(&["Nama", "Kota", "Jumlah"]));
        assert_eq!(sheet.rows[0], s(&["b", "Bdg", "11"]));
        assert_eq!(sheet.rows[1][1], "");
        let (h, cells) = sheet.remove_column(1).unwrap();
        assert_eq!(h, "Kota");
        assert_eq!(cells.len(), 5);
        assert_eq!(sheet.column_index("jumlah"), Some(1));
        assert_eq!(sheet.column_index("0"), Some(0));
        assert_eq!(sheet.remove_row(0).unwrap()[0], "b");
    }

    #[test]
    fn filter_is_case_insensitive() {
        let sheet = sample();
        assert_eq!(sheet.filter_rows("a"), vec![1]);
        assert_eq!(sheet.filter_rows("").len(), 4);
    }

    #[test]
    fn format_number_trims() {
        assert_eq!(format_number(3.0), "3");
        assert_eq!(format_number(2.5), "2.5");
        assert_eq!(format_number(1.0 / 3.0), "0.333333");
    }
}
