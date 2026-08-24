//! SQLite index cache (§3.1.1, §5): a rebuildable cache over the vault's
//! `.md` files (grid/search metadata) *and* over chunked+embedded document
//! text (`document_chunks`, the retrieval cache for §3.3/§3.4). Neither
//! table is the source of truth — notes live on disk, and chunks/vectors
//! are always regenerable from disk via `core::ingestion` +
//! `core::embedding`, so a missing/corrupt index file is just a "rebuild
//! from scratch" event, never data loss. Callers: `app.rs`,
//! `core::indexer` (writes `document_chunks` from its results).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, params};
use uuid::Uuid;

use super::embedding::{bytes_to_embedding, embedding_to_bytes};
use super::ingestion::DocumentChunk;
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
                );
                CREATE TABLE IF NOT EXISTS document_chunks (
                    id           INTEGER PRIMARY KEY AUTOINCREMENT,
                    doc_id       TEXT NOT NULL,
                    doc_type     TEXT NOT NULL,   -- 'note' | 'pdf'
                    file_path    TEXT NOT NULL,
                    page_num     INTEGER,          -- NULL for notes
                    char_offset  INTEGER NOT NULL,
                    text_content TEXT NOT NULL,
                    embedding    BLOB NOT NULL,     -- f32[] little-endian, see core::embedding
                    indexed_at   TEXT NOT NULL      -- RFC3339
                );
                CREATE INDEX IF NOT EXISTS idx_document_chunks_doc_id
                    ON document_chunks(doc_id);
                CREATE TABLE IF NOT EXISTS pdf_documents (
                    path        TEXT PRIMARY KEY,
                    imported_at TEXT NOT NULL     -- RFC3339
                );",
            )
            .context("creating index schema")?;
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

    /// Replaces every cached chunk belonging to `doc_id` with a freshly
    /// computed set, in one transaction (§3.3 point 2, §5). This is the
    /// (re)indexing write path for both notes and PDFs — a plain
    /// delete-then-insert rather than a diff, since chunk boundaries shift
    /// whenever the source text changes, so trying to patch individual
    /// rows in place would be no cheaper and much easier to get wrong.
    /// `chunks` pairs each `DocumentChunk` with its embedding vector, as
    /// produced by `core::indexer::IndexResult`.
    pub fn replace_chunks(
        &mut self,
        doc_id: Uuid,
        doc_type: &str,
        chunks: &[(DocumentChunk, Vec<f32>)],
    ) -> Result<()> {
        let tx = self
            .conn
            .transaction()
            .context("starting replace_chunks tx")?;
        tx.execute(
            "DELETE FROM document_chunks WHERE doc_id = ?1",
            params![doc_id.to_string()],
        )
        .context("clearing existing chunks for doc")?;

        let now = Utc::now().to_rfc3339();
        for (chunk, embedding) in chunks {
            tx.execute(
                "INSERT INTO document_chunks
                    (doc_id, doc_type, file_path, page_num, char_offset, text_content, embedding, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    doc_id.to_string(),
                    doc_type,
                    chunk.file_path.to_string_lossy(),
                    chunk.page_num.map(|p| p as i64),
                    chunk.char_offset as i64,
                    chunk.text_content,
                    embedding_to_bytes(embedding),
                    now,
                ],
            )
            .with_context(|| format!("inserting chunk for doc {doc_id}"))?;
        }
        tx.commit().context("committing replace_chunks tx")?;
        Ok(())
    }

    /// Drops all cached chunks for `doc_id` without replacing them — used
    /// when a note/PDF is deleted or trashed so stale chunks don't outlive
    /// their source and pollute future retrieval.
    pub fn delete_chunks_for_doc(&self, doc_id: Uuid) -> Result<()> {
        self.conn
            .execute(
                "DELETE FROM document_chunks WHERE doc_id = ?1",
                params![doc_id.to_string()],
            )
            .context("deleting chunks for doc")?;
        Ok(())
    }

    /// Total number of cached chunk rows across all documents.
    pub fn chunk_count(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM document_chunks", [], |row| row.get(0))
            .context("counting document_chunks rows")
    }

    /// Loads every cached chunk with its embedding — the retrieval-time
    /// read path for the future search/RAG pipeline (§3.3 point 3, §3.4).
    /// No filtering/paging yet: the whole cache is expected to comfortably
    /// fit in memory for a personal vault, matching `list_titles`' same
    /// load-it-all approach for the notes index.
    pub fn all_chunks(&self) -> Result<Vec<StoredChunk>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT doc_id, file_path, page_num, char_offset, text_content, embedding
                 FROM document_chunks",
            )
            .context("preparing all_chunks query")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                ))
            })
            .context("querying all_chunks")?;

        let mut out = Vec::new();
        for row in rows {
            let (doc_id, file_path, page_num, char_offset, text_content, embedding_blob) =
                row.context("reading document_chunks row")?;
            out.push(StoredChunk {
                chunk: DocumentChunk {
                    doc_id: Uuid::parse_str(&doc_id)
                        .with_context(|| format!("parsing stored doc_id '{doc_id}'"))?,
                    file_path: PathBuf::from(file_path),
                    page_num: page_num.map(|p| p as usize),
                    char_offset: char_offset as usize,
                    text_content,
                },
                embedding: bytes_to_embedding(&embedding_blob),
            });
        }
        Ok(out)
    }

    /// Registers `path` as an imported PDF (§Fase 8) — re-importing the
    /// same path just refreshes its `imported_at` timestamp rather than
    /// erroring, since `path` is the primary key.
    pub fn add_pdf_document(&self, path: &Path) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO pdf_documents (path, imported_at) VALUES (?1, ?2)
                 ON CONFLICT(path) DO UPDATE SET imported_at = excluded.imported_at",
                params![path.to_string_lossy(), Utc::now().to_rfc3339()],
            )
            .context("adding pdf document")?;
        Ok(())
    }

    /// All imported PDF paths, most recently imported first — backs the
    /// PDF library tab (§Fase 8).
    pub fn list_pdf_documents(&self) -> Result<Vec<PathBuf>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM pdf_documents ORDER BY imported_at DESC")
            .context("preparing list_pdf_documents query")?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .context("querying list_pdf_documents")?;
        let mut paths = Vec::new();
        for row in rows {
            paths.push(PathBuf::from(row.context("reading pdf_documents row")?));
        }
        Ok(paths)
    }

    /// Removes `path` from the imported-PDF list. Does not touch its
    /// cached `document_chunks` — callers that want those gone too should
    /// also call `delete_chunks_for_doc` with the PDF's `doc_id`
    /// (`core::ingestion::pdf_doc_id`), same as note deletion does.
    pub fn remove_pdf_document(&self, path: &Path) -> Result<()> {
        self.conn
            .execute("DELETE FROM pdf_documents WHERE path = ?1", params![path.to_string_lossy()])
            .context("removing pdf document")?;
        Ok(())
    }
}

/// One persisted chunk row together with its embedding vector, as read
/// back from `document_chunks` by `IndexStore::all_chunks`.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredChunk {
    pub chunk: DocumentChunk,
    pub embedding: Vec<f32>,
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

    fn sample_chunk(doc_id: Uuid, text: &str) -> (DocumentChunk, Vec<f32>) {
        (
            DocumentChunk {
                doc_id,
                file_path: PathBuf::from("catatan.md"),
                page_num: None,
                char_offset: 0,
                text_content: text.to_string(),
            },
            vec![0.1, 0.2, 0.3],
        )
    }

    #[test]
    fn replace_chunks_inserts_and_round_trips_embeddings() {
        let mut store = IndexStore::open_in_memory().unwrap();
        let doc_id = Uuid::new_v4();
        let chunks = vec![sample_chunk(doc_id, "halo dunia")];

        store.replace_chunks(doc_id, "note", &chunks).unwrap();

        assert_eq!(store.chunk_count().unwrap(), 1);
        let stored = store.all_chunks().unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].chunk.doc_id, doc_id);
        assert_eq!(stored[0].chunk.text_content, "halo dunia");
        assert_eq!(stored[0].embedding, vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn replace_chunks_drops_previous_chunks_for_the_same_doc() {
        let mut store = IndexStore::open_in_memory().unwrap();
        let doc_id = Uuid::new_v4();

        store
            .replace_chunks(doc_id, "note", &[sample_chunk(doc_id, "versi lama")])
            .unwrap();
        assert_eq!(store.chunk_count().unwrap(), 1);

        store
            .replace_chunks(doc_id, "note", &[sample_chunk(doc_id, "versi baru")])
            .unwrap();

        let stored = store.all_chunks().unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].chunk.text_content, "versi baru");
    }

    #[test]
    fn replace_chunks_does_not_touch_other_documents() {
        let mut store = IndexStore::open_in_memory().unwrap();
        let doc_a = Uuid::new_v4();
        let doc_b = Uuid::new_v4();

        store
            .replace_chunks(doc_a, "note", &[sample_chunk(doc_a, "punya A")])
            .unwrap();
        store
            .replace_chunks(doc_b, "pdf", &[sample_chunk(doc_b, "punya B")])
            .unwrap();
        assert_eq!(store.chunk_count().unwrap(), 2);

        store
            .replace_chunks(doc_a, "note", &[sample_chunk(doc_a, "A diedit")])
            .unwrap();

        assert_eq!(store.chunk_count().unwrap(), 2);
        let stored = store.all_chunks().unwrap();
        assert!(
            stored
                .iter()
                .any(|c| c.chunk.doc_id == doc_a && c.chunk.text_content == "A diedit")
        );
        assert!(
            stored
                .iter()
                .any(|c| c.chunk.doc_id == doc_b && c.chunk.text_content == "punya B")
        );
    }

    #[test]
    fn delete_chunks_for_doc_removes_only_that_doc() {
        let mut store = IndexStore::open_in_memory().unwrap();
        let doc_a = Uuid::new_v4();
        let doc_b = Uuid::new_v4();
        store
            .replace_chunks(doc_a, "note", &[sample_chunk(doc_a, "A")])
            .unwrap();
        store
            .replace_chunks(doc_b, "note", &[sample_chunk(doc_b, "B")])
            .unwrap();

        store.delete_chunks_for_doc(doc_a).unwrap();

        assert_eq!(store.chunk_count().unwrap(), 1);
        assert_eq!(store.all_chunks().unwrap()[0].chunk.doc_id, doc_b);
    }

    #[test]
    fn replace_chunks_with_empty_slice_clears_the_doc() {
        let mut store = IndexStore::open_in_memory().unwrap();
        let doc_id = Uuid::new_v4();
        store
            .replace_chunks(doc_id, "note", &[sample_chunk(doc_id, "isi")])
            .unwrap();
        assert_eq!(store.chunk_count().unwrap(), 1);

        store.replace_chunks(doc_id, "note", &[]).unwrap();
        assert_eq!(store.chunk_count().unwrap(), 0);
    }

    #[test]
    fn add_pdf_document_is_listed_most_recently_imported_first() {
        let store = IndexStore::open_in_memory().unwrap();
        store.add_pdf_document(Path::new("a.pdf")).unwrap();
        store.add_pdf_document(Path::new("b.pdf")).unwrap();

        let listed = store.list_pdf_documents().unwrap();
        assert_eq!(listed, vec![PathBuf::from("b.pdf"), PathBuf::from("a.pdf")]);
    }

    #[test]
    fn re_adding_the_same_pdf_path_does_not_duplicate_it() {
        let store = IndexStore::open_in_memory().unwrap();
        store.add_pdf_document(Path::new("a.pdf")).unwrap();
        store.add_pdf_document(Path::new("a.pdf")).unwrap();

        assert_eq!(store.list_pdf_documents().unwrap(), vec![PathBuf::from("a.pdf")]);
    }

    #[test]
    fn remove_pdf_document_drops_only_that_path() {
        let store = IndexStore::open_in_memory().unwrap();
        store.add_pdf_document(Path::new("a.pdf")).unwrap();
        store.add_pdf_document(Path::new("b.pdf")).unwrap();

        store.remove_pdf_document(Path::new("a.pdf")).unwrap();

        assert_eq!(store.list_pdf_documents().unwrap(), vec![PathBuf::from("b.pdf")]);
    }
}
