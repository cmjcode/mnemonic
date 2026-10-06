//! Pure grid filtering/sorting/snippet helpers for the Notes Grid (§3.1.2,
//! §3.1.3) — kept free of `egui` so they stay unit-testable without a UI.
//! Callers: `app.rs`.

use super::Note;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GridFilter {
    All,
    Archived,
    Trashed,
    Tag(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    Modified,
    Created,
    Title,
    Color,
}

/// A search string with Obsidian's operators pulled apart (§Fase 1.6):
/// `tag:x`, `path:x`, `file:x`, `"exact phrase"`, `-excluded`, and a
/// bare `OR` between terms. Everything is matched case-insensitively.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedQuery {
    pub terms: Vec<String>,
    pub phrases: Vec<String>,
    pub excluded: Vec<String>,
    pub tags: Vec<String>,
    pub paths: Vec<String>,
    pub files: Vec<String>,
    /// `a OR b`: any term/phrase suffices instead of all.
    pub any: bool,
}

impl ParsedQuery {
    pub fn parse(raw: &str) -> ParsedQuery {
        let mut q = ParsedQuery::default();
        for token in tokenize_query(raw) {
            let lower = token.to_lowercase();
            if lower == "or" {
                q.any = true;
            } else if let Some(t) = lower.strip_prefix("tag:") {
                push_nonempty(&mut q.tags, t.trim_start_matches('#'));
            } else if let Some(p) = lower.strip_prefix("path:") {
                push_nonempty(&mut q.paths, p);
            } else if let Some(f) = lower.strip_prefix("file:") {
                push_nonempty(&mut q.files, f);
            } else if let Some(x) = lower.strip_prefix('-') {
                push_nonempty(&mut q.excluded, x.trim_matches('"'));
            } else if lower.len() >= 2 && lower.starts_with('"') && lower.ends_with('"') {
                push_nonempty(&mut q.phrases, lower.trim_matches('"'));
            } else {
                push_nonempty(&mut q.terms, &lower);
            }
        }
        q
    }

    /// The free-text part (terms + phrases) — what goes to FTS and the
    /// embedding model.
    pub fn text(&self) -> String {
        self.terms
            .iter()
            .cloned()
            .chain(self.phrases.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// `true` when any operator restricts which notes may match.
    pub fn has_filters(&self) -> bool {
        !(self.tags.is_empty() && self.paths.is_empty() && self.files.is_empty() && self.excluded.is_empty())
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.phrases.is_empty() && !self.has_filters()
    }

    /// Whether `note` satisfies the operators (`tag:`/`path:`/`file:`/
    /// `-x`) — the free-text part is *not* checked here.
    pub fn filters_match(&self, note: &Note) -> bool {
        let path = note.path.to_string_lossy().to_lowercase();
        let file = note
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        self.tags.iter().all(|t| note.has_tag(t))
            && self.paths.iter().all(|p| path.contains(p))
            && self.files.iter().all(|f| file.contains(f))
            && self.excluded.iter().all(|x| {
                !note.frontmatter.title.to_lowercase().contains(x) && !note.body.to_lowercase().contains(x)
            })
    }

    /// Full match: operators plus the free text against title/body.
    pub fn matches_note(&self, note: &Note) -> bool {
        if !self.filters_match(note) {
            return false;
        }
        let needles: Vec<&String> = self.terms.iter().chain(self.phrases.iter()).collect();
        if needles.is_empty() {
            return true;
        }
        let title = note.frontmatter.title.to_lowercase();
        let body = note.body.to_lowercase();
        let hit = |n: &&String| title.contains(n.as_str()) || body.contains(n.as_str());
        if self.any {
            needles.iter().any(hit)
        } else {
            needles.iter().all(hit)
        }
    }
}

fn push_nonempty(v: &mut Vec<String>, s: &str) {
    let s = s.trim();
    if !s.is_empty() && !v.iter().any(|x| x == s) {
        v.push(s.to_string());
    }
}

/// Splits on whitespace but keeps `"quoted phrases"` (and `tag:"a b"`,
/// `-"a b"`) together.
fn tokenize_query(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for c in raw.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                cur.push(c);
            }
            c if c.is_whitespace() && !in_quotes => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Notes visible for `filter` + `search` (Obsidian-style query: terms,
/// `"phrases"`, `-excluded`, `tag:`, `path:`, `file:`, `OR`).
pub fn filter_notes<'a>(notes: &'a [Note], filter: &GridFilter, search: &str) -> Vec<&'a Note> {
    let query = ParsedQuery::parse(search);
    notes
        .iter()
        .filter(|n| match filter {
            GridFilter::All => !n.frontmatter.trashed && !n.frontmatter.archived,
            GridFilter::Archived => !n.frontmatter.trashed && n.frontmatter.archived,
            GridFilter::Trashed => n.frontmatter.trashed,
            GridFilter::Tag(tag) => !n.frontmatter.trashed && n.has_tag(tag),
        })
        .filter(|n| query.matches_note(n))
        .collect()
}

/// Sort `notes` in place per `mode`, always keeping pinned notes first
/// (§3.1.2).
pub fn sort_notes(notes: &mut [&Note], mode: SortMode) {
    notes.sort_by(|a, b| {
        b.frontmatter.pinned.cmp(&a.frontmatter.pinned).then_with(|| match mode {
            SortMode::Modified => b.frontmatter.modified.cmp(&a.frontmatter.modified),
            SortMode::Created => b.frontmatter.created.cmp(&a.frontmatter.created),
            SortMode::Title => a
                .frontmatter
                .title
                .to_lowercase()
                .cmp(&b.frontmatter.title.to_lowercase()),
            SortMode::Color => color_key(a.frontmatter.color.as_deref())
                .cmp(color_key(b.frontmatter.color.as_deref())),
        })
    });
}

fn color_key(color: Option<&str>) -> &str {
    color.unwrap_or("")
}

/// A plain-text preview of `body` for the card snippet: light Markdown
/// decoration (headings, list/checklist markers, blockquote, emphasis
/// characters) is stripped and the result truncated to `max_chars`
/// characters.
pub fn snippet(body: &str, max_chars: usize) -> String {
    let mut plain = String::new();
    for line in body.lines() {
        let mut trimmed = line.trim_start();
        for prefix in ["#### ", "### ", "## ", "# ", "- [ ] ", "- [x] ", "- [X] ", "- ", "* ", "+ ", "> "] {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                trimmed = rest;
                break;
            }
        }
        if !plain.is_empty() && !trimmed.is_empty() {
            plain.push(' ');
        }
        plain.push_str(trimmed);
    }
    let cleaned: String = plain.chars().filter(|c| !matches!(c, '*' | '_' | '`')).collect();
    truncate_chars(cleaned.trim(), max_chars)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_query_splits_operators() {
        let q = ParsedQuery::parse(r#"nasi "sayur asem" -telur tag:#rumah path:Resep file:minggu OR goreng"#);
        assert_eq!(q.terms, vec!["nasi", "goreng"]);
        assert_eq!(q.phrases, vec!["sayur asem"]);
        assert_eq!(q.excluded, vec!["telur"]);
        assert_eq!(q.tags, vec!["rumah"]);
        assert_eq!(q.paths, vec!["resep"]);
        assert_eq!(q.files, vec!["minggu"]);
        assert!(q.any);
        assert_eq!(q.text(), "nasi goreng sayur asem");
        assert!(ParsedQuery::parse("   ").is_empty());
    }

    #[test]
    fn filter_notes_honours_query_operators() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("Resep");
        std::fs::create_dir_all(&sub).unwrap();
        let a = Note::create(&sub, "Nasi Goreng", "pakai telur #rumah/dapur").unwrap();
        let b = Note::create(dir.path(), "Nasi Uduk", "tanpa telur, pakai santan").unwrap();
        let notes = vec![a, b];
        let titles = |q: &str| -> Vec<String> {
            filter_notes(&notes, &GridFilter::All, q)
                .iter()
                .map(|n| n.frontmatter.title.clone())
                .collect()
        };
        assert_eq!(titles("nasi -santan"), vec!["Nasi Goreng"]);
        assert_eq!(titles("tag:rumah"), vec!["Nasi Goreng"]);
        assert_eq!(titles("path:resep"), vec!["Nasi Goreng"]);
        assert_eq!(titles("file:uduk"), vec!["Nasi Uduk"]);
        assert_eq!(titles("\"pakai santan\""), vec!["Nasi Uduk"]);
        assert_eq!(titles("santan OR dapur").len(), 2);
        assert_eq!(titles("santan dapur").len(), 0);
    }
    use tempfile::tempdir;

    fn note(dir: &std::path::Path, title: &str, body: &str) -> Note {
        Note::create(dir, title, body).unwrap()
    }

    #[test]
    fn filter_notes_all_excludes_archived_and_trashed() {
        let dir = tempdir().unwrap();
        let visible = note(dir.path(), "Terlihat", "");
        let mut archived = note(dir.path(), "Arsip", "");
        archived.frontmatter.archived = true;
        let mut trashed = note(dir.path(), "Sampah", "");
        trashed.frontmatter.trashed = true;

        let notes = vec![visible, archived, trashed];
        let result = filter_notes(&notes, &GridFilter::All, "");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].frontmatter.title, "Terlihat");
    }

    #[test]
    fn filter_notes_by_tag_is_case_insensitive() {
        let dir = tempdir().unwrap();
        let mut a = note(dir.path(), "A", "");
        a.frontmatter.tags = vec!["Rumah".to_string()];
        let b = note(dir.path(), "B", "");

        let notes = vec![a, b];
        let result = filter_notes(&notes, &GridFilter::Tag("rumah".to_string()), "");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].frontmatter.title, "A");
    }

    #[test]
    fn filter_notes_search_matches_title_or_body() {
        let dir = tempdir().unwrap();
        let a = note(dir.path(), "Belanja Mingguan", "beli susu");
        let b = note(dir.path(), "Catatan Lain", "isi lain");

        let notes = vec![a, b];
        let result = filter_notes(&notes, &GridFilter::All, "susu");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].frontmatter.title, "Belanja Mingguan");
    }

    #[test]
    fn sort_notes_keeps_pinned_first_regardless_of_mode() {
        let dir = tempdir().unwrap();
        let mut unpinned = note(dir.path(), "Z Unpinned", "");
        let mut pinned = note(dir.path(), "A Pinned", "");
        pinned.frontmatter.pinned = true;
        unpinned.frontmatter.pinned = false;

        let mut notes: Vec<&Note> = vec![&unpinned, &pinned];
        sort_notes(&mut notes, SortMode::Title);

        assert_eq!(notes[0].frontmatter.title, "A Pinned");
    }

    #[test]
    fn sort_notes_by_title_is_case_insensitive_alphabetical() {
        let dir = tempdir().unwrap();
        let b = note(dir.path(), "banana", "");
        let a = note(dir.path(), "Apple", "");

        let mut notes: Vec<&Note> = vec![&b, &a];
        sort_notes(&mut notes, SortMode::Title);

        assert_eq!(notes[0].frontmatter.title, "Apple");
        assert_eq!(notes[1].frontmatter.title, "banana");
    }

    #[test]
    fn snippet_strips_markdown_decoration_and_truncates() {
        let body = "# Judul\n- [ ] beli **susu**\nteks *biasa*";
        assert_eq!(snippet(body, 100), "Judul beli susu teks biasa");
    }

    #[test]
    fn snippet_truncates_long_text_with_ellipsis() {
        let body = "a".repeat(20);
        let result = snippet(&body, 10);
        assert_eq!(result.chars().count(), 10);
        assert!(result.ends_with('…'));
    }
}
