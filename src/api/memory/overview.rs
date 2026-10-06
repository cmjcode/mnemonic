//! Orientation for agents (§3.10.1): the folder tree with note counts and
//! `vault_overview` — folders, top tags, recent notes, titles several
//! notes share, and the vault's own `AGENTS.md` — in one call, so an agent
//! learns the vault's structure and conventions before touching it.
//! Callers: `api::memory::tools`, `api::mcp`, `src/bin/mnemonic-cli`.

use std::collections::BTreeMap;

use super::types::*;
use crate::api::VaultService;
use crate::markdown::wikilink::title_key;
use crate::notes::vault::is_skipped_dir_name;

/// Files at the vault root read as the vault's instructions for agents,
/// first match wins.
pub const GUIDE_FILES: &[&str] = &["AGENTS.md", "agents.md"];
/// Longest guide text returned (chars).
const GUIDE_MAX_CHARS: usize = 8000;
/// Folders listed by the overview (the full tree stays in `list_folders`).
const OVERVIEW_MAX_FOLDERS: usize = 200;

impl VaultService {
    /// Every non-hidden folder of the vault (the root first, as `""`),
    /// sorted by path, with direct and subtree note counts.
    pub fn list_folders(&self) -> FolderList {
        let mut folders: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        folders.insert(String::new(), (0, 0));
        let walker = walkdir::WalkDir::new(self.root())
            .min_depth(1)
            .into_iter()
            .filter_entry(|e| !(e.file_type().is_dir() && is_skipped_dir_name(&e.file_name().to_string_lossy())));
        for entry in walker.flatten() {
            if entry.file_type().is_dir() {
                folders.entry(self.rel(entry.path())).or_default();
            }
        }
        for note in self.vault.notes.iter().filter(|n| !n.frontmatter.trashed) {
            let folder = self.summary(note).folder;
            folders.entry(folder.clone()).or_default().0 += 1;
            let mut prefix = folder.as_str();
            loop {
                folders.entry(prefix.to_string()).or_default().1 += 1;
                if prefix.is_empty() {
                    break;
                }
                prefix = prefix.rfind('/').map_or("", |i| &prefix[..i]);
            }
        }
        let root_name = self
            .root()
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        FolderList {
            folders: folders
                .into_iter()
                .map(|(path, (notes, notes_total))| FolderInfo {
                    name: path.rsplit('/').next().filter(|n| !n.is_empty()).map_or(root_name.clone(), str::to_string),
                    depth: if path.is_empty() { 0 } else { path.matches('/').count() + 1 },
                    path,
                    notes,
                    notes_total,
                })
                .collect(),
        }
    }

    /// Structure, vocabulary and conventions of the vault in one call.
    pub fn vault_overview(&self) -> VaultOverview {
        let live: Vec<_> = self.vault.notes.iter().filter(|n| !n.frontmatter.trashed).collect();
        let mut folders = self.list_folders().folders;
        let more_folders = folders.len().saturating_sub(OVERVIEW_MAX_FOLDERS);
        folders.truncate(OVERVIEW_MAX_FOLDERS);

        let mut tags: Vec<TagCount> = crate::notes::tags::all_tags(&self.vault.notes)
            .into_iter()
            .map(|(tag, count)| TagCount { tag, count })
            .collect();
        tags.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.tag.to_lowercase().cmp(&b.tag.to_lowercase())));
        tags.truncate(40);

        let mut by_modified = live.clone();
        by_modified.sort_by_key(|n| std::cmp::Reverse(n.frontmatter.modified));
        let recent = by_modified
            .iter()
            .take(10)
            .map(|n| RecentNote {
                note: self.note_ref(n),
                modified: n.frontmatter.modified,
            })
            .collect();

        let mut by_title: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
        for n in &live {
            let entry = by_title
                .entry(title_key(&n.frontmatter.title))
                .or_insert_with(|| (n.frontmatter.title.clone(), Vec::new()));
            entry.1.push(self.rel(&n.path));
        }
        let duplicate_titles = by_title
            .into_values()
            .filter(|(_, paths)| paths.len() > 1)
            .map(|(title, mut paths)| {
                paths.sort();
                DuplicateTitle { title, paths }
            })
            .collect();

        let mut tips = vec![
            "Refer to notes by vault-relative path (e.g. `Kuliah/Algoritma Graph.md`) when a title appears in duplicate_titles; bare titles of those are rejected as ambiguous.".to_string(),
            "read_note returns `outline` and `content_hash`; read one part with `section` (`Heading`, `Parent#Child`, `^anchor`) to save tokens.".to_string(),
            "Edit with append_note / patch_note instead of rewriting whole notes, and pass the last `content_hash` as `if_hash` so edits made meanwhile are never overwritten.".to_string(),
            "Use `recall` for task context within a token budget and `remember` to store a durable fact (it refuses duplicates).".to_string(),
            "Mark outdated knowledge with frontmatter `valid_until: YYYY-MM-DD` or `superseded_by: \"[[Newer Note]]\"`; recall ranks such notes last.".to_string(),
        ];
        if more_folders > 0 {
            tips.push(format!("{more_folders} more folders are not listed here; call list_folders for the full tree."));
        }

        VaultOverview {
            name: self
                .root()
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            notes: live.iter().filter(|n| !n.is_canvas()).count(),
            canvases: live.iter().filter(|n| n.is_canvas()).count(),
            sheets: crate::sheet::find_sheets(self.root()).len(),
            pdfs: self.index.list_pdf_documents().map(|p| p.len()).unwrap_or(0),
            folders,
            tags,
            recent,
            duplicate_titles,
            guide: self.agent_guide(),
            tips,
        }
    }

    /// The vault's `AGENTS.md` (frontmatter stripped), capped in length.
    fn agent_guide(&self) -> Option<AgentGuide> {
        GUIDE_FILES.iter().find_map(|name| {
            let path = self.root().join(name);
            let raw = std::fs::read_to_string(&path).ok()?;
            let body = self
                .vault
                .notes
                .iter()
                .find(|n| n.path == path)
                .map_or(raw, |n| n.body.clone());
            let truncated = body.chars().count() > GUIDE_MAX_CHARS;
            Some(AgentGuide {
                path: name.to_string(),
                text: body.chars().take(GUIDE_MAX_CHARS).collect(),
                truncated,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::api::VaultService;
    use crate::notes::Note;
    use tempfile::tempdir;

    #[test]
    fn folders_and_overview_describe_the_vault() {
        let dir = tempdir().unwrap();
        let kuliah = dir.path().join("Kuliah/Semester 1");
        std::fs::create_dir_all(&kuliah).unwrap();
        std::fs::create_dir_all(dir.path().join("Kosong")).unwrap();
        std::fs::create_dir_all(dir.path().join(".obsidian")).unwrap();
        Note::create(dir.path(), "ML", "#riset satu").unwrap();
        Note::create(&kuliah, "ML", "#riset dua").unwrap();
        Note::create(&dir.path().join("Kuliah"), "Graph", "").unwrap();
        Note::create(dir.path(), "AGENTS", "Simpan memori di folder Memory/.").unwrap();
        let svc = VaultService::open(dir.path()).unwrap();

        let folders = svc.list_folders().folders;
        let paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["", "Kosong", "Kuliah", "Kuliah/Semester 1"]);
        assert_eq!((folders[0].notes, folders[0].notes_total), (2, 4));
        assert_eq!((folders[2].notes, folders[2].notes_total, folders[2].depth), (1, 2, 1));
        assert_eq!(folders[3].name, "Semester 1");

        let o = svc.vault_overview();
        assert_eq!(o.notes, 4);
        assert_eq!(o.duplicate_titles.len(), 1);
        assert_eq!(o.duplicate_titles[0].paths, vec!["Kuliah/Semester 1/ML.md", "ML.md"]);
        assert_eq!(o.tags[0].tag, "riset");
        assert_eq!(o.tags[0].count, 2);
        let guide = o.guide.unwrap();
        assert_eq!(guide.path, "AGENTS.md");
        assert!(guide.text.contains("Memory/") && !guide.text.contains("title:"), "{}", guide.text);
    }
}
