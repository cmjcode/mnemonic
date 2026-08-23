//! SQLite index cache (§3.1.1): a rebuildable cache over the vault's
//! `.md` files, used for fast search/grid rendering. Never the source of
//! truth — always rebuildable from disk. Callers: `app.rs`.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::notes::Note;

/// Opens (creating if needed) the SQLite index cache file at
/// `vault_root/.lontar-index.sqlite3` and ensures its schema exists.
pub struct IndexStore {
    conn: Connection,
}

impl IndexStore {
    pub fn open(vault_root: &Path) -> Result<IndexStore> {
        let path = vault_root.join(".lontar-index.sqlite3");
        let conn = Connection::open(&path)
            .with_context(|| format!("opening index db {}", path.display()))?;
        let store = IndexStore { conn };
        store.ensure_schema()?;
        Ok(store)
    }

    /// In-memory index store, useful for tests.
    #[cfg(test)]
    pub fn open_in_memory() -> Result<IndexStore> {
        let conn = Connection::open_in_memory().context("opening in-memory index db")?;
        let store = IndexStore { conn };
        store.ensure_schema()?;
        Ok(store)
    }

    fn ensure_schema(&self) -> Result<()> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS notes_index (
                    id        TEXT PRIMARY KEY,
                    path      TEXT NOT NULL UNIQUE,
                    title     TEXT NOT NULL,
                    tags      TEXT NOT NULL,   -- comma-joined
                    pinned    INTEGER NOT NULL,
                    archived  INTEGER NOT NULL,
                    trashed   INTEGER NOT NULL,
                    modified  TEXT NOT NULL    -- RFC3339
                );",
            )
            .context("creating notes_index schema")?;
        Ok(())
    }

    /// Fully rebuild the index from the given (already-scanned) notes.
    /// This is the recovery path when the index file is missing/corrupt
    /// (§6 risk mitigation: index is always derivable from disk).
    pub fn rebuild(&mut self, notes: &[Note]) -> Result<()> {
        let tx = self.conn.transaction().context("starting rebuild tx")?;
        tx.execute("DELETE FROM notes_index", [])
            .context("clearing notes_index")?;
        for note in notes {
            tx.execute(
                "INSERT INTO notes_index (id, path, title, tags, pinned, archived, trashed, modified)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    note.frontmatter.id.to_string(),
                    note.path.to_string_lossy(),
                    note.frontmatter.title,
                    note.frontmatter.tags.join(","),
                    note.frontmatter.pinned,
                    note.frontmatter.archived,
                    note.frontmatter.trashed,
                    note.frontmatter.modified.to_rfc3339(),
                ],
            )
            .with_context(|| format!("indexing note {}", note.path.display()))?;
        }
        tx.commit().context("committing rebuild tx")?;
        Ok(())
    }

    /// Number of rows currently in the index — used by tests and by
    /// startup logic to decide whether a rebuild is warranted.
    pub fn count(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM notes_index", [], |row| row.get(0))
            .context("counting notes_index rows")
    }

    /// Titles of all indexed, non-trashed notes ordered by most recently
    /// modified first — enough for a minimal Fase 1 grid/list view.
    pub fn list_titles(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT title FROM notes_index WHERE trashed = 0 ORDER BY modified DESC")
            .context("preparing list_titles query")?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .context("querying list_titles")?;
        let mut titles = Vec::new();
        for row in rows {
            titles.push(row.context("reading list_titles row")?);
        }
        Ok(titles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rebuild_from_scratch_indexes_all_notes() {
        let dir = tempdir().unwrap();
        let note = Note::create(dir.path(), "Judul Satu", "isi").unwrap();

        let mut store = IndexStore::open_in_memory().unwrap();
        assert_eq!(store.count().unwrap(), 0);

        store.rebuild(&[note]).unwrap();
        assert_eq!(store.count().unwrap(), 1);
        assert_eq!(store.list_titles().unwrap(), vec!["Judul Satu".to_string()]);
    }

    #[test]
    fn rebuild_replaces_previous_contents() {
        let dir = tempdir().unwrap();
        let note_a = Note::create(dir.path(), "A", "isi").unwrap();
        let note_b = Note::create(dir.path(), "B", "isi").unwrap();

        let mut store = IndexStore::open_in_memory().unwrap();
        store.rebuild(&[note_a]).unwrap();
        assert_eq!(store.count().unwrap(), 1);

        store.rebuild(&[note_b]).unwrap();
        assert_eq!(store.count().unwrap(), 1);
        assert_eq!(store.list_titles().unwrap(), vec!["B".to_string()]);
    }

    #[test]
    fn missing_index_file_rebuilds_cleanly_on_reopen() {
        let dir = tempdir().unwrap();
        let note = Note::create(dir.path(), "Persisten", "isi").unwrap();

        {
            let mut store = IndexStore::open(dir.path()).unwrap();
            store.rebuild(&[note.clone()]).unwrap();
        }

        // Simulate a corrupt/missing index file being deleted externally.
        std::fs::remove_file(dir.path().join(".lontar-index.sqlite3")).unwrap();

        let mut store = IndexStore::open(dir.path()).unwrap();
        assert_eq!(store.count().unwrap(), 0); // fresh db, empty until rebuild
        store.rebuild(&[note]).unwrap();
        assert_eq!(store.count().unwrap(), 1);
    }
}
