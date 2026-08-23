//! `Note` model: CRUD for a single note backed by a `.md` file on disk.
//! Callers: `notes::vault` (scan/create), `core::storage` (index rebuild).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;

use super::frontmatter::{self, NoteFrontmatter};

/// A note loaded from (or about to be written to) disk.
#[derive(Debug, Clone)]
pub struct Note {
    pub path: PathBuf,
    pub frontmatter: NoteFrontmatter,
    pub body: String,
}

impl Note {
    /// Load a note from an existing `.md` file. Never fails on malformed
    /// frontmatter — see `frontmatter::parse`.
    pub fn load(path: &Path) -> Result<Note> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading note file {}", path.display()))?;
        let (frontmatter, body) = frontmatter::parse(&raw);
        Ok(Note {
            path: path.to_path_buf(),
            frontmatter,
            body,
        })
    }

    /// Create a new note file in `dir` with the given title and body.
    pub fn create(dir: &Path, title: &str, body: &str) -> Result<Note> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating vault dir {}", dir.display()))?;

        let now = Utc::now();
        let mut frontmatter = NoteFrontmatter::default();
        frontmatter.title = title.to_string();
        frontmatter.created = now;
        frontmatter.modified = now;

        let file_name = format!("{}.md", frontmatter.id);
        let path = dir.join(file_name);

        let note = Note {
            path,
            frontmatter,
            body: body.to_string(),
        };
        note.save()?;
        Ok(note)
    }

    /// Write current frontmatter + body back to `self.path`, bumping
    /// `modified`. Writes are not yet atomic (temp-file + rename) — that
    /// hardening is deferred past Fase 1.
    pub fn save(&self) -> Result<()> {
        let mut fm = self.frontmatter.clone();
        fm.modified = Utc::now();
        let raw = frontmatter::serialize(&fm, &self.body)?;
        std::fs::write(&self.path, raw)
            .with_context(|| format!("writing note file {}", self.path.display()))?;
        Ok(())
    }

    /// Soft-delete: mark `trashed: true` and move the file into `.trash/`
    /// under the given vault root. Per §3.1.4, permanent deletion after a
    /// retention window is handled separately by `notes::trash`.
    pub fn move_to_trash(mut self, vault_root: &Path) -> Result<Note> {
        self.frontmatter.trashed = true;
        let trash_dir = vault_root.join(".trash");
        std::fs::create_dir_all(&trash_dir)
            .with_context(|| format!("creating trash dir {}", trash_dir.display()))?;

        let file_name = self
            .path
            .file_name()
            .context("note path has no file name")?;
        let new_path = trash_dir.join(file_name);

        self.save()?; // persist trashed:true at the old path first
        std::fs::rename(&self.path, &new_path)
            .with_context(|| format!("moving note to trash {}", new_path.display()))?;
        self.path = new_path;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn create_then_load_round_trips() {
        let dir = tempdir().unwrap();
        let created = Note::create(dir.path(), "Belanja Mingguan", "- [ ] Beli beras\n").unwrap();

        let loaded = Note::load(&created.path).unwrap();
        assert_eq!(loaded.frontmatter.title, "Belanja Mingguan");
        assert_eq!(loaded.body, "- [ ] Beli beras\n");
        assert_eq!(loaded.frontmatter.id, created.frontmatter.id);
    }

    #[test]
    fn save_bumps_modified_timestamp() {
        let dir = tempdir().unwrap();
        let mut note = Note::create(dir.path(), "Judul", "isi").unwrap();
        let first_modified = note.frontmatter.modified;

        std::thread::sleep(std::time::Duration::from_millis(5));
        note.body = "isi baru".to_string();
        note.save().unwrap();

        let reloaded = Note::load(&note.path).unwrap();
        assert!(reloaded.frontmatter.modified > first_modified);
        assert_eq!(reloaded.body, "isi baru");
    }

    #[test]
    fn move_to_trash_sets_flag_and_relocates_file() {
        let dir = tempdir().unwrap();
        let note = Note::create(dir.path(), "Hapus Saya", "isi").unwrap();
        let original_path = note.path.clone();

        let trashed = note.move_to_trash(dir.path()).unwrap();

        assert!(trashed.frontmatter.trashed);
        assert!(!original_path.exists());
        assert!(trashed.path.exists());
        assert_eq!(trashed.path.parent().unwrap().file_name().unwrap(), ".trash");
    }
}
