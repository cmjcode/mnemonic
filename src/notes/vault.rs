//! Vault selection & initial scan. A "vault" is a user-chosen folder on
//! disk containing `.md` note files (§3.1.1). Remembering which vault was
//! open last lives in `crate::settings`. Callers: `app.rs`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::note::Note;

/// An open vault: a root folder plus the notes currently loaded from it.
/// `notes` includes trashed notes (loaded from `.trash/`, flagged
/// `frontmatter.trashed`) so the Trash view can list and restore them;
/// every other consumer already filters on that flag.
pub struct Vault {
    pub root: PathBuf,
    pub notes: Vec<Note>,
}

impl Vault {
    /// Open (or create) a vault at `root` and perform an initial
    /// recursive scan for `.md` files.
    pub fn open(root: PathBuf) -> Result<Vault> {
        std::fs::create_dir_all(&root)
            .with_context(|| format!("creating vault root {}", root.display()))?;
        let notes = scan(&root)?;
        Ok(Vault { root, notes })
    }

    /// Re-scan the vault root from disk, refreshing `self.notes`.
    /// Used by the file watcher and manual refresh.
    pub fn rescan(&mut self) -> Result<()> {
        self.notes = scan(&self.root)?;
        Ok(())
    }
}

/// Folders never walked for notes: hidden folders (`.obsidian`, `.git`,
/// `.mnemonic`, `.trash`, …) and dependency trees.
pub fn is_skipped_dir_name(name: &str) -> bool {
    name.starts_with('.') || name == "node_modules"
}

/// Recursively find all `.md` files under `root` and load them as notes.
/// Hidden folders are skipped (an Obsidian vault's `.obsidian/`, a `.git/`
/// checkout, our own `.mnemonic/`), and `.trash/` is not walked
/// recursively (a trashed folder keeps its tree out of the main view),
/// but its top-level notes are loaded so the Trash view can show them
/// (§3.1.4). A single unreadable file is logged and skipped rather than
/// failing the whole scan.
fn scan(root: &Path) -> Result<Vec<Note>> {
    let mut notes = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || !(e.file_type().is_dir()
                    && e.file_name().to_str().is_some_and(is_skipped_dir_name))
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                log::warn!("vault scan: skipping unreadable entry: {e}");
                continue;
            }
        };
        if entry.file_type().is_file() && is_markdown(entry.path()) {
            load_into(&mut notes, entry.path(), false);
        }
    }

    if let Ok(entries) = std::fs::read_dir(root.join(".trash")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && is_markdown(&path) {
                load_into(&mut notes, &path, true);
            }
        }
    }
    Ok(notes)
}

fn is_markdown(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("md")
}

fn load_into(notes: &mut Vec<Note>, path: &Path, in_trash: bool) {
    match Note::load(path) {
        Ok(mut note) => {
            // A note sitting in `.trash/` is trashed regardless of what
            // its frontmatter says (e.g. dropped there by hand).
            if in_trash {
                note.frontmatter.trashed = true;
            }
            notes.push(note);
        }
        Err(e) => log::warn!("vault scan: failed to load {}: {e}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn scan_finds_markdown_files_and_flags_trashed_ones() {
        let dir = tempdir().unwrap();
        Note::create(dir.path(), "Note A", "isi a").unwrap();
        Note::create(dir.path(), "Note B", "isi b").unwrap();
        std::fs::write(dir.path().join("not_a_note.txt"), "abaikan").unwrap();

        let trash_dir = dir.path().join(".trash");
        std::fs::create_dir_all(&trash_dir).unwrap();
        Note::create(&trash_dir, "Trashed", "isi").unwrap();
        // A whole trashed folder is not listed note-by-note.
        let trashed_folder = trash_dir.join("Old Folder");
        std::fs::create_dir_all(&trashed_folder).unwrap();
        Note::create(&trashed_folder, "Nested", "isi").unwrap();

        // Hidden folders (Obsidian config, git) and node_modules are ignored.
        for hidden in [".obsidian", ".git", "node_modules"] {
            let d = dir.path().join(hidden);
            std::fs::create_dir_all(&d).unwrap();
            Note::create(&d, "Hidden", "isi").unwrap();
        }

        let notes = scan(dir.path()).unwrap();
        assert_eq!(notes.len(), 3);
        let trashed: Vec<_> = notes.iter().filter(|n| n.frontmatter.trashed).collect();
        assert_eq!(trashed.len(), 1);
        assert_eq!(trashed[0].frontmatter.title, "Trashed");
    }
}
