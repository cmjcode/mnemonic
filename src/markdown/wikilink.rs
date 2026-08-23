//! Wikilink `[[Title]]` extraction & resolution, plus the backlink graph
//! (§3.2.2). Pure text-processing lives here so it stays unit-testable
//! without an `egui::Ui`; the actual link click handling lives in
//! `markdown::renderer`. Callers: `markdown::renderer`, `app.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::notes::Note;

/// Extract every wikilink target title referenced in `body`, in order of
/// appearance (duplicates included). `![[embed]]` image embeds are not
/// treated as note links. Occurrences inside fenced code blocks (``` or
/// ~~~) are ignored so code samples containing literal `[[...]]` text
/// aren't mistaken for links.
pub fn extract_wikilinks(body: &str) -> Vec<String> {
    let mut titles = Vec::new();
    let mut in_fence = false;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        find_wikilinks_in_line(line, |title, is_embed| {
            if !is_embed && !title.is_empty() {
                titles.push(title.to_string());
            }
        });
    }
    titles
}

/// Scans a single line for `[[...]]` occurrences, invoking `on_match` with
/// the target title (alias stripped) and whether it was an embed (`![[`).
pub(super) fn find_wikilinks_in_line(line: &str, mut on_match: impl FnMut(&str, bool)) {
    let mut rest = line;
    loop {
        let Some(start) = rest.find("[[") else { break };
        let is_embed = start > 0 && rest.as_bytes()[start - 1] == b'!';
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else { break };
        let inner = &after[..end];
        let title = inner.split('|').next().unwrap_or(inner).trim();
        on_match(title, is_embed);
        rest = &after[end + 2..];
    }
}

/// Maps note titles (case-insensitive) to the note that owns them, for
/// wikilink resolution and autocomplete (§3.2.2).
pub struct WikilinkIndex {
    by_title: HashMap<String, (String, PathBuf)>,
}

impl WikilinkIndex {
    /// Build the index from all (non-trashed) notes currently in the
    /// vault. Titles are assumed unique, matching Obsidian's own
    /// convention; the last note wins on a collision.
    pub fn build(notes: &[Note]) -> WikilinkIndex {
        let mut by_title = HashMap::new();
        for note in notes {
            if note.frontmatter.trashed || note.frontmatter.title.is_empty() {
                continue;
            }
            by_title.insert(
                note.frontmatter.title.to_lowercase(),
                (note.frontmatter.title.clone(), note.path.clone()),
            );
        }
        WikilinkIndex { by_title }
    }

    /// The path of the note titled `title`, if one exists.
    pub fn resolve(&self, title: &str) -> Option<&Path> {
        self.by_title.get(&title.to_lowercase()).map(|(_, p)| p.as_path())
    }

    pub fn contains(&self, title: &str) -> bool {
        self.by_title.contains_key(&title.to_lowercase())
    }

    /// Titles starting with `prefix` (case-insensitive), sorted
    /// alphabetically and capped at `limit` — backing the `[[` autocomplete
    /// popup.
    pub fn suggestions(&self, prefix: &str, limit: usize) -> Vec<String> {
        let needle = prefix.to_lowercase();
        let mut matches: Vec<&str> = self
            .by_title
            .values()
            .filter(|(title, _)| title.to_lowercase().starts_with(&needle))
            .map(|(title, _)| title.as_str())
            .collect();
        matches.sort_unstable();
        matches.into_iter().take(limit).map(String::from).collect()
    }
}

/// Notes (other than the one identified by `current_id`) whose body
/// contains a wikilink to `target_title` — the Backlinks panel (§3.2.2).
pub fn backlinks_for<'a>(target_title: &str, current_id: Uuid, notes: &'a [Note]) -> Vec<&'a Note> {
    let target = target_title.to_lowercase();
    notes
        .iter()
        .filter(|n| n.frontmatter.id != current_id && !n.frontmatter.trashed)
        .filter(|n| {
            extract_wikilinks(&n.body)
                .iter()
                .any(|t| t.to_lowercase() == target)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn extract_wikilinks_finds_plain_links() {
        let body = "Lihat [[Belanja Mingguan]] dan [[Resep Nasi Goreng]].";
        assert_eq!(
            extract_wikilinks(body),
            vec!["Belanja Mingguan", "Resep Nasi Goreng"]
        );
    }

    #[test]
    fn extract_wikilinks_strips_alias() {
        let body = "Baca [[Belanja Mingguan|daftar belanja]] dulu.";
        assert_eq!(extract_wikilinks(body), vec!["Belanja Mingguan"]);
    }

    #[test]
    fn extract_wikilinks_ignores_image_embeds() {
        let body = "![[foto.png]] tapi [[Catatan Lain]] tetap dihitung.";
        assert_eq!(extract_wikilinks(body), vec!["Catatan Lain"]);
    }

    #[test]
    fn extract_wikilinks_ignores_fenced_code_blocks() {
        let body = "```\n[[Bukan Link]]\n```\n[[Link Asli]]";
        assert_eq!(extract_wikilinks(body), vec!["Link Asli"]);
    }

    #[test]
    fn index_resolves_case_insensitively() {
        let dir = tempdir().unwrap();
        let note = Note::create(dir.path(), "Belanja Mingguan", "").unwrap();
        let index = WikilinkIndex::build(&[note.clone()]);

        assert!(index.contains("belanja mingguan"));
        assert_eq!(index.resolve("BELANJA MINGGUAN"), Some(note.path.as_path()));
    }

    #[test]
    fn index_suggestions_filter_sort_and_limit() {
        let dir = tempdir().unwrap();
        let notes = vec![
            Note::create(dir.path(), "Belanja Mingguan", "").unwrap(),
            Note::create(dir.path(), "Belanja Bulanan", "").unwrap(),
            Note::create(dir.path(), "Resep", "").unwrap(),
        ];
        let index = WikilinkIndex::build(&notes);

        assert_eq!(
            index.suggestions("bel", 1),
            vec!["Belanja Bulanan".to_string()]
        );
        assert_eq!(
            index.suggestions("bel", 10),
            vec!["Belanja Bulanan".to_string(), "Belanja Mingguan".to_string()]
        );
    }

    #[test]
    fn backlinks_for_finds_referencing_notes_and_excludes_self() {
        let dir = tempdir().unwrap();
        let target = Note::create(dir.path(), "Target", "").unwrap();
        let referrer = Note::create(dir.path(), "Referrer", "Lihat [[Target]].").unwrap();
        let unrelated = Note::create(dir.path(), "Unrelated", "Tidak menaut apa pun.").unwrap();
        let self_referencing =
            Note::create(dir.path(), "Target Duplikat", "[[Target]] (harus tetap muncul)").unwrap();

        let notes = vec![target.clone(), referrer.clone(), unrelated, self_referencing.clone()];
        let backlinks = backlinks_for("Target", target.frontmatter.id, &notes);

        let titles: Vec<&str> = backlinks.iter().map(|n| n.frontmatter.title.as_str()).collect();
        assert!(titles.contains(&"Referrer"));
        assert!(titles.contains(&"Target Duplikat"));
        assert_eq!(titles.len(), 2);
    }
}
