//! Vault-wide tag operations (§3.1.3 label manager): a tag isn't owned by
//! any single note, so listing, renaming, and deleting it means scanning
//! and mutating every note's frontmatter. Pure `Vec<Note>` mutation —
//! callers (`app.rs`) are responsible for calling `Note::save()` on the
//! changed indices afterward.

use super::Note;

/// Inline `#tag`s in a note body (Obsidian §Fase 1.2): a `#` at the start
/// of a line or after whitespace/punctuation, followed by letters, digits,
/// `_`, `-` or `/` (nested tags) with at least one non-digit character,
/// outside fenced code and inline code. Order of first appearance,
/// case-insensitively deduplicated, without the `#`.
pub fn inline_tags(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut in_fence = false;
    for line in body.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || t.starts_with('#') && t.chars().find(|c| *c != '#') == Some(' ') {
            // Fenced code, or a markdown heading.
            continue;
        }
        let mut in_code = false;
        let mut prev: Option<char> = None;
        let mut chars = line.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '`' {
                in_code = !in_code;
                prev = Some(c);
                continue;
            }
            if in_code || c != '#' {
                prev = Some(c);
                continue;
            }
            let boundary = prev.is_none_or(|p| p.is_whitespace() || "([{\"'".contains(p));
            if !boundary {
                prev = Some(c);
                continue;
            }
            let rest = &line[i + 1..];
            let len = rest
                .chars()
                .take_while(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-' | '/'))
                .map(char::len_utf8)
                .sum::<usize>();
            let tag = rest[..len].trim_matches('/');
            if !tag.is_empty() && tag.chars().any(|ch| !ch.is_ascii_digit()) {
                if !out.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
                    out.push(tag.to_string());
                }
                for _ in 0..rest[..len].chars().count() {
                    chars.next();
                }
            }
            prev = Some(c);
        }
    }
    out
}

/// `true` when `tag` is `filter` or nested under it (`projek/web` matches
/// `projek`), case-insensitively.
pub fn matches_tag(tag: &str, filter: &str) -> bool {
    let tag = tag.trim_matches('/');
    let filter = filter.trim_matches('/');
    tag.eq_ignore_ascii_case(filter)
        || (tag.len() > filter.len()
            && tag[..filter.len()].eq_ignore_ascii_case(filter)
            && tag[filter.len()..].starts_with('/'))
}

/// Distinct tags across non-trashed notes (frontmatter and inline), with
/// how many notes use each, sorted alphabetically (case-insensitive) —
/// backs the sidebar's "Berlabel" section and the label manager.
pub fn all_tags(notes: &[Note]) -> Vec<(String, usize)> {
    let mut counts: std::collections::HashMap<String, (String, usize)> = std::collections::HashMap::new();
    for note in notes {
        if note.frontmatter.trashed {
            continue;
        }
        for tag in &note.effective_tags() {
            let entry = counts
                .entry(tag.to_lowercase())
                .or_insert_with(|| (tag.clone(), 0));
            entry.1 += 1;
        }
    }
    let mut tags: Vec<(String, usize)> = counts.into_values().collect();
    tags.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    tags
}

/// Rename `old` to `new` on every note that has it (case-insensitive
/// match), de-duplicating if a note already also has `new`. Returns the
/// indices of notes that changed.
pub fn rename_tag(notes: &mut [Note], old: &str, new: &str) -> Vec<usize> {
    let old_lower = old.to_lowercase();
    let mut changed = Vec::new();
    for (i, note) in notes.iter_mut().enumerate() {
        if !note.frontmatter.tags.iter().any(|t| t.to_lowercase() == old_lower) {
            continue;
        }
        let mut tags: Vec<String> = note
            .frontmatter
            .tags
            .iter()
            .map(|t| {
                if t.to_lowercase() == old_lower {
                    new.to_string()
                } else {
                    t.clone()
                }
            })
            .collect();
        dedupe_case_insensitive(&mut tags);
        note.frontmatter.tags = tags;
        changed.push(i);
    }
    changed
}

/// Remove `tag` from every note that has it. Returns the indices of notes
/// that changed.
pub fn remove_tag(notes: &mut [Note], tag: &str) -> Vec<usize> {
    let tag_lower = tag.to_lowercase();
    let mut changed = Vec::new();
    for (i, note) in notes.iter_mut().enumerate() {
        let before = note.frontmatter.tags.len();
        note.frontmatter.tags.retain(|t| t.to_lowercase() != tag_lower);
        if note.frontmatter.tags.len() != before {
            changed.push(i);
        }
    }
    changed
}

fn dedupe_case_insensitive(tags: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    tags.retain(|t| seen.insert(t.to_lowercase()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_tags_finds_hashtags_outside_code_and_headings() {
        let body = "# Heading bukan tag
Catatan #projek/web dan #Rumah, lagi #projek/web
`#kode` bukan
```
#fence bukan
```
angka #2024 bukan, tapi #q1-2024 ya (#dalam-kurung)";
        assert_eq!(
            inline_tags(body),
            vec!["projek/web", "Rumah", "q1-2024", "dalam-kurung"]
        );
    }

    #[test]
    fn matches_tag_understands_nesting() {
        assert!(matches_tag("projek/web", "projek"));
        assert!(matches_tag("Projek", "projek"));
        assert!(!matches_tag("projekan", "projek"));
        assert!(!matches_tag("projek", "projek/web"));
    }

    use tempfile::tempdir;

    fn note_with_tags(dir: &std::path::Path, title: &str, tags: &[&str]) -> Note {
        let mut note = Note::create(dir, title, "").unwrap();
        note.frontmatter.tags = tags.iter().map(|t| t.to_string()).collect();
        note
    }

    #[test]
    fn all_tags_counts_and_sorts_excluding_trashed() {
        let dir = tempdir().unwrap();
        let mut a = note_with_tags(dir.path(), "A", &["rumah", "belanja"]);
        let b = note_with_tags(dir.path(), "B", &["Rumah"]);
        let mut trashed = note_with_tags(dir.path(), "C", &["belanja"]);
        trashed.frontmatter.trashed = true;
        a.frontmatter.trashed = false;

        let tags = all_tags(&[a, b, trashed]);
        assert_eq!(
            tags,
            vec![("belanja".to_string(), 1), ("rumah".to_string(), 2)]
        );
    }

    #[test]
    fn rename_tag_updates_matching_notes_and_dedupes() {
        let dir = tempdir().unwrap();
        let mut notes = vec![
            note_with_tags(dir.path(), "A", &["lama"]),
            note_with_tags(dir.path(), "B", &["lama", "baru"]),
            note_with_tags(dir.path(), "C", &["lain"]),
        ];

        let changed = rename_tag(&mut notes, "lama", "baru");

        assert_eq!(changed, vec![0, 1]);
        assert_eq!(notes[0].frontmatter.tags, vec!["baru".to_string()]);
        assert_eq!(notes[1].frontmatter.tags, vec!["baru".to_string()]); // deduped
        assert_eq!(notes[2].frontmatter.tags, vec!["lain".to_string()]);
    }

    #[test]
    fn remove_tag_only_touches_notes_that_have_it() {
        let dir = tempdir().unwrap();
        let mut notes = vec![
            note_with_tags(dir.path(), "A", &["hapus", "sisa"]),
            note_with_tags(dir.path(), "B", &["lain"]),
        ];

        let changed = remove_tag(&mut notes, "hapus");

        assert_eq!(changed, vec![0]);
        assert_eq!(notes[0].frontmatter.tags, vec!["sisa".to_string()]);
        assert_eq!(notes[1].frontmatter.tags, vec!["lain".to_string()]);
    }
}
