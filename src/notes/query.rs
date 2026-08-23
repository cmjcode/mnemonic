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

/// Notes visible for `filter` + `search` (case-insensitive substring match
/// against title or body).
pub fn filter_notes<'a>(notes: &'a [Note], filter: &GridFilter, search: &str) -> Vec<&'a Note> {
    let query = search.trim().to_lowercase();
    notes
        .iter()
        .filter(|n| match filter {
            GridFilter::All => !n.frontmatter.trashed && !n.frontmatter.archived,
            GridFilter::Archived => !n.frontmatter.trashed && n.frontmatter.archived,
            GridFilter::Trashed => n.frontmatter.trashed,
            GridFilter::Tag(tag) => {
                !n.frontmatter.trashed && n.frontmatter.tags.iter().any(|t| t.eq_ignore_ascii_case(tag))
            }
        })
        .filter(|n| {
            query.is_empty()
                || n.frontmatter.title.to_lowercase().contains(&query)
                || n.body.to_lowercase().contains(&query)
        })
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
