//! Vault-wide tag operations (§3.1.3 label manager): a tag isn't owned by
//! any single note, so listing, renaming, and deleting it means scanning
//! and mutating every note's frontmatter. Pure `Vec<Note>` mutation —
//! callers (`app.rs`) are responsible for calling `Note::save()` on the
//! changed indices afterward.

use super::Note;

/// Distinct tags across non-trashed notes, with how many notes use each,
/// sorted alphabetically (case-insensitive) — backs the sidebar's
/// "Berlabel" section and the label manager.
pub fn all_tags(notes: &[Note]) -> Vec<(String, usize)> {
    let mut counts: std::collections::HashMap<String, (String, usize)> = std::collections::HashMap::new();
    for note in notes {
        if note.frontmatter.trashed {
            continue;
        }
        for tag in &note.frontmatter.tags {
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
