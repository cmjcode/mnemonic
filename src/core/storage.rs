//! SQLite index cache (§3.1.1, §5): a rebuildable cache over the vault's
//! `.md` files (grid/search metadata + the wikilink table) *and* over
//! chunked+embedded document text (the retrieval cache for §3.3/§3.4).
//! Nothing here is the source of truth — notes live on disk, and chunks/
//! vectors are always regenerable via `core::ingestion` +
//! `core::embedding`, so a missing/corrupt/outdated index file is just a
//! "rebuild from scratch" event, never data loss.
//!
//! Retrieval runs inside SQLite: `vec_chunks` / `vec_docs` are
//! `sqlite-vec` `vec0` tables (brute-force KNN with cosine distance) and
//! `chunks_fts` is an FTS5 table (BM25) for the keyword half of hybrid
//! search. Callers: `app`, `core::indexer` (via `IndexResult`),
//! `graph::model`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Once;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

use super::embedding::{bytes_to_embedding, embedding_to_bytes};
use super::ingestion::DocumentChunk;
use crate::markdown::wikilink::{self, title_key};
use crate::notes::Note;

/// Bumped whenever the cache schema changes shape. An index file written
/// with another version has its derived tables dropped and rebuilt;
/// `pdf_documents` (the user's import list) and `graph_layout` survive.
const SCHEMA_VERSION: i64 = 2;

static REGISTER_SQLITE_VEC: Once = Once::new();

/// Registers the statically linked `sqlite-vec` extension with every
/// SQLite connection this process opens from now on.
fn register_sqlite_vec() {
    REGISTER_SQLITE_VEC.call_once(|| {
        // SAFETY: `sqlite3_vec_init` is sqlite-vec's C extension entry
        // point, which has exactly the signature `sqlite3_auto_extension`
        // expects (db, error-message out-pointer, API routines); the
        // transmute only erases the Rust-side declaration mismatch.
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                unsafe extern "C" fn(
                    *mut rusqlite::ffi::sqlite3,
                    *mut *mut std::os::raw::c_char,
                    *const rusqlite::ffi::sqlite3_api_routines,
                ) -> std::os::raw::c_int,
            >(
                sqlite_vec::sqlite3_vec_init as *const ()
            )));
        }
    });
}

/// Opens (creating if needed) the SQLite index cache file at
/// `vault_root/.mnemonic-index.sqlite3` and ensures its schema exists.
pub struct IndexStore {
    conn: Connection,
}

/// One persisted chunk row together with its embedding vector, as read
/// back from `document_chunks` by `IndexStore::all_chunks`.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredChunk {
    pub chunk: DocumentChunk,
    pub embedding: Vec<f32>,
}

/// Marks the start/end of a matched term inside `KeywordChunk::snippet`.
/// Control characters, so they can't collide with note text (which often
/// contains `[`/`]` from wikilinks and checklists).
pub const HIGHLIGHT_START: char = '\u{2}';
pub const HIGHLIGHT_END: char = '\u{3}';

/// A chunk matched by FTS5, with its BM25 rank (lower is better) and a
/// snippet with matched terms wrapped in `HIGHLIGHT_START`/`HIGHLIGHT_END`.
#[derive(Debug, Clone, PartialEq)]
pub struct KeywordChunk {
    pub chunk: DocumentChunk,
    pub bm25: f64,
    pub snippet: String,
}

/// A note linking to some target, for the backlinks panel.
#[derive(Debug, Clone, PartialEq)]
pub struct Backlink {
    pub src_id: Uuid,
    pub src_title: String,
    pub src_path: PathBuf,
    pub line: usize,
    pub context: String,
}

/// One wikilink edge for the graph: `src_id` links to whatever
/// `target_key` resolves to (a note title, a PDF name, or nothing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkEdge {
    pub src_id: Uuid,
    pub target: String,
    pub target_key: String,
}

/// Folder inside the vault holding MNEMONIC's own state (never scanned
/// for notes — see `notes::vault::is_skipped_dir_name`).
pub const STATE_DIR: &str = ".mnemonic";
/// Index file name inside `STATE_DIR`.
pub const INDEX_FILE: &str = "index.sqlite3";
/// Pre-Fase 2 location of the index, migrated on first open.
const LEGACY_INDEX_FILE: &str = ".mnemonic-index.sqlite3";

impl IndexStore {
    /// Where the index of the vault at `vault_root` lives.
    pub fn index_path(vault_root: &Path) -> PathBuf {
        vault_root.join(STATE_DIR).join(INDEX_FILE)
    }

    /// Opens (creating if needed) the vault's index. WAL journaling plus a
    /// busy timeout so the desktop app and the `mnemonic-cli`/MCP process
    /// can have the file open at the same time (§Fase 2).
    pub fn open(vault_root: &Path) -> Result<IndexStore> {
        register_sqlite_vec();
        let path = Self::index_path(vault_root);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("creating state dir {}", dir.display()))?;
        }
        let legacy = vault_root.join(LEGACY_INDEX_FILE);
        if !path.exists() && legacy.exists() {
            if let Err(e) = std::fs::rename(&legacy, &path) {
                log::warn!("storage: could not move legacy index into {STATE_DIR}: {e}");
            }
        }
        let conn = Connection::open(&path)
            .with_context(|| format!("opening index db {}", path.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .context("setting busy timeout")?;
        // WAL lets readers (the CLI) proceed while the app writes; NORMAL
        // sync is safe under WAL and much cheaper on every autosave-driven
        // reindex. A failure here is not fatal — just slower/lock-prone.
        if let Err(e) = conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;") {
            log::warn!("storage: enabling WAL failed: {e}");
        }
        let store = IndexStore { conn };
        store.ensure_schema()?;
        Ok(store)
    }

    /// In-memory index store, useful for tests.
    #[cfg(test)]
    pub fn open_in_memory() -> Result<IndexStore> {
        register_sqlite_vec();
        let conn = Connection::open_in_memory().context("opening in-memory index db")?;
        let store = IndexStore { conn };
        store.ensure_schema()?;
        Ok(store)
    }

    fn ensure_schema(&self) -> Result<()> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS index_meta (
                    key   TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS pdf_documents (
                    path        TEXT PRIMARY KEY,
                    imported_at TEXT NOT NULL     -- RFC3339
                );
                CREATE TABLE IF NOT EXISTS graph_layout (
                    node_key TEXT PRIMARY KEY,
                    x        REAL NOT NULL,
                    y        REAL NOT NULL
                );",
            )
            .context("creating index meta tables")?;

        if self.meta("schema_version")?.as_deref() != Some(&SCHEMA_VERSION.to_string()) {
            // Older cache layout (e.g. v1 kept embeddings as BLOBs): drop
            // everything derived and start clean — it is all rebuildable.
            self.conn
                .execute_batch(
                    "DROP TABLE IF EXISTS notes_index;
                     DROP TABLE IF EXISTS document_chunks;
                     DROP TABLE IF EXISTS chunks_fts;
                     DROP TABLE IF EXISTS documents;
                     DROP TABLE IF EXISTS links;
                     DROP TABLE IF EXISTS vec_chunks;
                     DROP TABLE IF EXISTS vec_docs;
                     DELETE FROM index_meta;",
                )
                .context("dropping outdated index tables")?;
            self.set_meta("schema_version", &SCHEMA_VERSION.to_string())?;
        }

        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS notes_index (
                    id        TEXT PRIMARY KEY,
                    path      TEXT NOT NULL UNIQUE,
                    title     TEXT NOT NULL,
                    title_key TEXT NOT NULL,   -- wikilink::title_key(title)
                    tags      TEXT NOT NULL,   -- comma-joined
                    pinned    INTEGER NOT NULL,
                    archived  INTEGER NOT NULL,
                    trashed   INTEGER NOT NULL,
                    modified  TEXT NOT NULL    -- RFC3339
                );
                CREATE INDEX IF NOT EXISTS idx_notes_index_title_key
                    ON notes_index(title_key);
                CREATE TABLE IF NOT EXISTS document_chunks (
                    id           INTEGER PRIMARY KEY AUTOINCREMENT,
                    doc_id       TEXT NOT NULL,
                    doc_type     TEXT NOT NULL,   -- 'note' | 'pdf'
                    file_path    TEXT NOT NULL,
                    page_num     INTEGER,          -- NULL for notes
                    char_offset  INTEGER NOT NULL,
                    text_content TEXT NOT NULL,
                    indexed_at   TEXT NOT NULL      -- RFC3339
                );
                CREATE INDEX IF NOT EXISTS idx_document_chunks_doc_id
                    ON document_chunks(doc_id);
                -- rowid = document_chunks.id
                CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
                    title, text_content,
                    tokenize = 'unicode61 remove_diacritics 2'
                );
                -- One row per embedded document; id keys vec_docs.
                CREATE TABLE IF NOT EXISTS documents (
                    id        INTEGER PRIMARY KEY AUTOINCREMENT,
                    doc_id    TEXT NOT NULL UNIQUE,
                    doc_type  TEXT NOT NULL,
                    file_path TEXT NOT NULL,
                    title     TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS links (
                    src_id     TEXT NOT NULL,
                    target     TEXT NOT NULL,
                    target_key TEXT NOT NULL,
                    heading    TEXT,
                    line       INTEGER NOT NULL,
                    context    TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_links_src ON links(src_id);
                CREATE INDEX IF NOT EXISTS idx_links_target ON links(target_key);
                -- Content fingerprint of the text that was chunked+embedded
                -- per document, so an unchanged note is never re-embedded
                -- (incremental indexing, §Fase 2).
                CREATE TABLE IF NOT EXISTS document_hashes (
                    doc_id       TEXT PRIMARY KEY,
                    content_hash TEXT NOT NULL,
                    indexed_at   TEXT NOT NULL
                );",
            )
            .context("creating index schema")?;
        Ok(())
    }

    /// The content hash recorded when `doc_id` was last chunked+embedded.
    pub fn document_hash(&self, doc_id: Uuid) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT content_hash FROM document_hashes WHERE doc_id = ?1",
                [doc_id.to_string()],
                |r| r.get(0),
            )
            .optional()
            .context("reading document hash")
    }

    /// Records `content_hash` for `doc_id` (call after `replace_chunks`).
    pub fn set_document_hash(&self, doc_id: Uuid, content_hash: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO document_hashes (doc_id, content_hash, indexed_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(doc_id) DO UPDATE SET
                    content_hash = excluded.content_hash, indexed_at = excluded.indexed_at",
                params![doc_id.to_string(), content_hash, Utc::now().to_rfc3339()],
            )
            .context("recording document hash")?;
        Ok(())
    }

    /// `true` when `doc_id` was never embedded, or its text changed since.
    pub fn needs_reindex(&self, doc_id: Uuid, content_hash: &str) -> Result<bool> {
        Ok(self.document_hash(doc_id)?.as_deref() != Some(content_hash))
    }

    /// Every doc id that has cached chunks of type `doc_type`.
    pub fn indexed_doc_ids(&self, doc_type: &str) -> Result<Vec<Uuid>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT doc_id FROM document_chunks WHERE doc_type = ?1")
            .context("preparing indexed_doc_ids query")?;
        let rows = stmt
            .query_map([doc_type], |r| r.get::<_, String>(0))
            .context("querying indexed doc ids")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(parse_uuid(&row.context("reading doc id")?)?);
        }
        Ok(out)
    }

    /// Drops cached chunks of notes that no longer exist in the vault
    /// (deleted or trashed outside the app), so retrieval never cites a
    /// note the user can't open. Returns how many were pruned.
    pub fn prune_notes_not_in(&self, live: &std::collections::HashSet<Uuid>) -> Result<usize> {
        let mut pruned = 0;
        for id in self.indexed_doc_ids("note")? {
            if !live.contains(&id) {
                self.delete_chunks_for_doc(id)?;
                pruned += 1;
            }
        }
        Ok(pruned)
    }

    fn meta(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM index_meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .with_context(|| format!("reading index meta '{key}'"))
    }

    fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO index_meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .with_context(|| format!("writing index meta '{key}'"))?;
        Ok(())
    }

    fn vector_dim(&self) -> Result<Option<usize>> {
        Ok(self.meta("embedding_dim")?.and_then(|d| d.parse().ok()))
    }

    /// Makes sure the vector tables match `model_id`/`dim`. When the cache
    /// was built by a different embedding model (or never built), all
    /// chunks and vectors are dropped and `true` is returned: the caller
    /// must resubmit every note and PDF for indexing.
    pub fn ensure_embedding_model(&mut self, model_id: &str, dim: usize) -> Result<bool> {
        let same = self.meta("embedding_model")?.as_deref() == Some(model_id)
            && self.vector_dim()? == Some(dim);
        if same {
            return Ok(false);
        }
        let tx = self.conn.transaction().context("starting model reset tx")?;
        tx.execute_batch(
            "DELETE FROM document_chunks;
             DELETE FROM chunks_fts;
             DELETE FROM documents;
             DROP TABLE IF EXISTS vec_chunks;
             DROP TABLE IF EXISTS vec_docs;",
        )
        .context("clearing chunks for embedding model change")?;
        create_vector_tables(&tx, dim)?;
        for (key, value) in [
            ("embedding_model", model_id.to_string()),
            ("embedding_dim", dim.to_string()),
        ] {
            tx.execute(
                "INSERT INTO index_meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .context("recording embedding model")?;
        }
        tx.commit().context("committing model reset tx")?;
        Ok(true)
    }

    // ─── Notes & links ───────────────────────────────────────────────────

    /// Fully rebuild the notes index and link table from the given
    /// (already-scanned) notes. This is the recovery path when the index
    /// file is missing/corrupt (§6 risk mitigation), and also runs on every
    /// vault rescan, which keeps links in sync with external edits.
    pub fn rebuild(&mut self, notes: &[Note]) -> Result<()> {
        let tx = self.conn.transaction().context("starting rebuild tx")?;
        tx.execute_batch("DELETE FROM notes_index; DELETE FROM links;")
            .context("clearing notes_index")?;
        for note in notes {
            insert_note(&tx, note)?;
        }
        tx.commit().context("committing rebuild tx")?;
        Ok(())
    }

    /// Re-indexes a single note's metadata and outgoing links — cheaper
    /// than `rebuild` after the editor saves one note.
    pub fn upsert_note(&mut self, note: &Note) -> Result<()> {
        let tx = self.conn.transaction().context("starting upsert_note tx")?;
        let id = note.frontmatter.id.to_string();
        tx.execute(
            "DELETE FROM notes_index WHERE id = ?1 OR path = ?2",
            params![id, note.path.to_string_lossy()],
        )
        .context("removing previous note row")?;
        tx.execute("DELETE FROM links WHERE src_id = ?1", [&id])
            .context("removing previous note links")?;
        insert_note(&tx, note)?;
        tx.commit().context("committing upsert_note tx")?;
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
    /// modified first.
    pub fn list_titles(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT title FROM notes_index WHERE trashed = 0 ORDER BY modified DESC")
            .context("preparing list_titles query")?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .context("querying list_titles")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("reading list_titles rows")
    }

    /// Non-trashed notes linking to `title` (other than `exclude_id`), one
    /// row per link occurrence, ordered by source title then line.
    pub fn backlinks(&self, title: &str, exclude_id: Uuid) -> Result<Vec<Backlink>> {
        self.backlinks_for_keys(&[title_key(title)], exclude_id)
    }

    /// Backlinks reaching a note through any of `keys` (its title, file
    /// stem and aliases — see `wikilink::link_keys_for`), each already
    /// normalized with `title_key`.
    pub fn backlinks_for_keys(&self, keys: &[String], exclude_id: Uuid) -> Result<Vec<Backlink>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (0..keys.len())
            .map(|i| format!("?{}", i + 2))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT n.id, n.title, n.path, l.line, l.context
             FROM links l JOIN notes_index n ON n.id = l.src_id
             WHERE l.target_key IN ({placeholders}) AND n.trashed = 0 AND n.id != ?1
             ORDER BY n.title COLLATE NOCASE, l.line"
        );
        let mut stmt = self.conn.prepare(&sql).context("preparing backlinks query")?;
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(exclude_id.to_string())];
        for k in keys {
            args.push(Box::new(k.clone()));
        }
        let rows = stmt
            .query_map(rusqlite::params_from_iter(args.iter().map(|a| a.as_ref())), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .context("querying backlinks")?;
        let mut out = Vec::new();
        for row in rows {
            let (id, src_title, path, line, context) = row.context("reading backlink row")?;
            out.push(Backlink {
                src_id: parse_uuid(&id)?,
                src_title,
                src_path: PathBuf::from(path),
                line: line as usize,
                context,
            });
        }
        Ok(out)
    }

    /// Every wikilink from a non-trashed note, for the graph view.
    pub fn link_edges(&self) -> Result<Vec<LinkEdge>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT l.src_id, l.target, l.target_key
                 FROM links l JOIN notes_index n ON n.id = l.src_id
                 WHERE n.trashed = 0
                 ORDER BY l.rowid",
            )
            .context("preparing link_edges query")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .context("querying link_edges")?;
        let mut out = Vec::new();
        for row in rows {
            let (src, target, target_key) = row.context("reading link edge")?;
            out.push(LinkEdge {
                src_id: parse_uuid(&src)?,
                target,
                target_key,
            });
        }
        Ok(out)
    }

    // ─── Chunks & vectors ────────────────────────────────────────────────

    /// Replaces every cached chunk belonging to `doc_id` with a freshly
    /// computed set, in one transaction (§3.3 point 2, §5): the chunk rows,
    /// their FTS entries, their `vec_chunks` vectors, and the document's
    /// mean vector in `vec_docs`. A plain delete-then-insert rather than a
    /// diff, since chunk boundaries shift whenever the source text changes.
    /// `title` is indexed alongside the text so keyword search can match
    /// on it. If no vector tables exist yet (no `ensure_embedding_model`
    /// call, e.g. in tests) they are created from the first vector's size.
    pub fn replace_chunks(
        &mut self,
        doc_id: Uuid,
        doc_type: &str,
        title: &str,
        chunks: &[(DocumentChunk, Vec<f32>)],
    ) -> Result<()> {
        let dim = match (self.vector_dim()?, chunks.first()) {
            (Some(dim), _) => Some(dim),
            (None, Some((_, v))) => Some(v.len()),
            (None, None) => None,
        };
        if let Some(dim) = dim
            && let Some((_, bad)) = chunks.iter().find(|(_, v)| v.len() != dim)
        {
            bail!(
                "embedding for doc {doc_id} has {} dimensions, index expects {dim}",
                bad.len()
            );
        }

        let tx = self
            .conn
            .transaction()
            .context("starting replace_chunks tx")?;
        if let Some(dim) = dim
            && !table_exists(&tx, "vec_chunks")?
        {
            create_vector_tables(&tx, dim)?;
            tx.execute(
                "INSERT INTO index_meta (key, value) VALUES ('embedding_dim', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [dim.to_string()],
            )
            .context("recording embedding dim")?;
        }
        delete_doc(&tx, doc_id)?;

        let now = Utc::now().to_rfc3339();
        let mut mean = vec![0f32; dim.unwrap_or(0)];
        for (chunk, embedding) in chunks {
            tx.execute(
                "INSERT INTO document_chunks
                    (doc_id, doc_type, file_path, page_num, char_offset, text_content, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    doc_id.to_string(),
                    doc_type,
                    chunk.file_path.to_string_lossy(),
                    chunk.page_num.map(|p| p as i64),
                    chunk.char_offset as i64,
                    chunk.text_content,
                    now,
                ],
            )
            .with_context(|| format!("inserting chunk for doc {doc_id}"))?;
            let chunk_id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO chunks_fts (rowid, title, text_content) VALUES (?1, ?2, ?3)",
                params![chunk_id, title, chunk.text_content],
            )
            .context("indexing chunk text")?;
            tx.execute(
                "INSERT INTO vec_chunks (chunk_id, embedding) VALUES (?1, ?2)",
                params![chunk_id, embedding_to_bytes(embedding)],
            )
            .context("storing chunk vector")?;
            for (m, x) in mean.iter_mut().zip(embedding) {
                *m += x;
            }
        }

        if let Some((first, _)) = chunks.first() {
            tx.execute(
                "INSERT INTO documents (doc_id, doc_type, file_path, title) VALUES (?1, ?2, ?3, ?4)",
                params![
                    doc_id.to_string(),
                    doc_type,
                    first.file_path.to_string_lossy(),
                    title
                ],
            )
            .context("registering embedded document")?;
            let doc_row = tx.last_insert_rowid();
            let norm = mean.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 0.0 {
                mean.iter_mut().for_each(|x| *x /= norm);
                tx.execute(
                    "INSERT INTO vec_docs (doc_row, embedding) VALUES (?1, ?2)",
                    params![doc_row, embedding_to_bytes(&mean)],
                )
                .context("storing document vector")?;
            }
        }
        tx.commit().context("committing replace_chunks tx")?;
        Ok(())
    }

    /// Drops all cached chunks and vectors for `doc_id` without replacing
    /// them — used when a note/PDF is deleted or trashed so stale chunks
    /// don't outlive their source and pollute future retrieval.
    pub fn delete_chunks_for_doc(&self, doc_id: Uuid) -> Result<()> {
        let tx = self
            .conn
            .unchecked_transaction()
            .context("starting delete_chunks tx")?;
        delete_doc(&tx, doc_id)?;
        tx.commit().context("committing delete_chunks tx")?;
        Ok(())
    }

    /// Total number of cached chunk rows across all documents.
    pub fn chunk_count(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM document_chunks", [], |row| row.get(0))
            .context("counting document_chunks rows")
    }

    /// Loads every cached chunk with its embedding. Retrieval uses
    /// `knn_chunks`/`keyword_chunks` instead; this full scan remains for
    /// diagnostics and tests.
    pub fn all_chunks(&self) -> Result<Vec<StoredChunk>> {
        if !table_exists(&self.conn, "vec_chunks")? {
            return Ok(Vec::new());
        }
        let mut stmt = self
            .conn
            .prepare(
                "SELECT c.doc_id, c.file_path, c.page_num, c.char_offset, c.text_content,
                        (SELECT v.embedding FROM vec_chunks v WHERE v.chunk_id = c.id)
                 FROM document_chunks c ORDER BY c.id",
            )
            .context("preparing all_chunks query")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((read_chunk(row)?, row.get::<_, Option<Vec<u8>>>(5)?))
            })
            .context("querying all_chunks")?;
        let mut out = Vec::new();
        for row in rows {
            let (chunk, blob) = row.context("reading document_chunks row")?;
            out.push(StoredChunk {
                chunk: chunk?,
                embedding: blob.map(|b| bytes_to_embedding(&b)).unwrap_or_default(),
            });
        }
        Ok(out)
    }

    /// The `k` chunks closest to `query` by cosine similarity (`1 −
    /// cosine distance`), best first. Empty when nothing is embedded yet.
    pub fn knn_chunks(&self, query: &[f32], k: usize) -> Result<Vec<(DocumentChunk, f32)>> {
        if k == 0 || query.is_empty() || !table_exists(&self.conn, "vec_chunks")? {
            return Ok(Vec::new());
        }
        let mut stmt = self
            .conn
            .prepare(
                "SELECT c.doc_id, c.file_path, c.page_num, c.char_offset, c.text_content, knn.distance
                 FROM (SELECT chunk_id, distance FROM vec_chunks
                       WHERE embedding MATCH ?1 AND k = ?2) knn
                 JOIN document_chunks c ON c.id = knn.chunk_id
                 ORDER BY knn.distance",
            )
            .context("preparing knn_chunks query")?;
        let rows = stmt
            .query_map(params![embedding_to_bytes(query), k as i64], |row| {
                Ok((read_chunk(row)?, row.get::<_, f64>(5)?))
            })
            .context("querying knn_chunks")?;
        let mut out = Vec::new();
        for row in rows {
            let (chunk, distance) = row.context("reading knn row")?;
            out.push((chunk?, 1.0 - distance as f32));
        }
        Ok(out)
    }

    /// Up to `k` chunks matching `text` via FTS5/BM25, best first. All
    /// query words must match (the last one as a prefix); if that finds
    /// nothing, any word may match.
    pub fn keyword_chunks(&self, text: &str, k: usize) -> Result<Vec<KeywordChunk>> {
        let parsed = crate::notes::query::ParsedQuery::parse(text);
        for any in [parsed.any, true] {
            let Some(query) = fts_query_parsed(&parsed, any) else {
                return Ok(Vec::new());
            };
            let hits = self.fts_search(&query, k)?;
            if !hits.is_empty() {
                return Ok(hits);
            }
        }
        Ok(Vec::new())
    }

    fn fts_search(&self, query: &str, k: usize) -> Result<Vec<KeywordChunk>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT c.doc_id, c.file_path, c.page_num, c.char_offset, c.text_content,
                        bm25(chunks_fts, 2.0, 1.0),
                        snippet(chunks_fts, 1, char(2), char(3), '…', 24)
                 FROM chunks_fts JOIN document_chunks c ON c.id = chunks_fts.rowid
                 WHERE chunks_fts MATCH ?1
                 ORDER BY bm25(chunks_fts, 2.0, 1.0)
                 LIMIT ?2",
            )
            .context("preparing keyword query")?;
        let rows = stmt
            .query_map(params![query, k as i64], |row| {
                Ok((
                    read_chunk(row)?,
                    row.get::<_, f64>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .context("querying chunks_fts")?;
        let mut out = Vec::new();
        for row in rows {
            let (chunk, bm25, snippet) = row.context("reading keyword row")?;
            out.push(KeywordChunk {
                chunk: chunk?,
                bm25,
                snippet,
            });
        }
        Ok(out)
    }

    /// Documents most similar to `doc_id` by mean-vector cosine
    /// similarity, best first, excluding `doc_id` itself.
    pub fn similar_documents(&self, doc_id: Uuid, k: usize) -> Result<Vec<(Uuid, f32)>> {
        if k == 0 || !table_exists(&self.conn, "vec_docs")? {
            return Ok(Vec::new());
        }
        let vector: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT (SELECT v.embedding FROM vec_docs v WHERE v.doc_row = d.id)
                 FROM documents d WHERE d.doc_id = ?1",
                [doc_id.to_string()],
                |r| r.get(0),
            )
            .optional()
            .context("loading document vector")?
            .flatten();
        let Some(vector) = vector else {
            return Ok(Vec::new());
        };
        let mut stmt = self
            .conn
            .prepare(
                "SELECT d.doc_id, knn.distance
                 FROM (SELECT doc_row, distance FROM vec_docs
                       WHERE embedding MATCH ?1 AND k = ?2) knn
                 JOIN documents d ON d.id = knn.doc_row
                 ORDER BY knn.distance",
            )
            .context("preparing similar_documents query")?;
        let rows = stmt
            .query_map(params![vector, (k + 1) as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
            })
            .context("querying similar_documents")?;
        let mut out = Vec::new();
        for row in rows {
            let (id, distance) = row.context("reading similar document row")?;
            let id = parse_uuid(&id)?;
            if id != doc_id {
                out.push((id, 1.0 - distance as f32));
            }
        }
        out.truncate(k);
        Ok(out)
    }

    // ─── PDFs ────────────────────────────────────────────────────────────

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
    /// cached chunks — callers that want those gone too should also call
    /// `delete_chunks_for_doc` with the PDF's `doc_id`
    /// (`core::ingestion::pdf_doc_id`), same as note deletion does.
    pub fn remove_pdf_document(&self, path: &Path) -> Result<()> {
        self.conn
            .execute(
                "DELETE FROM pdf_documents WHERE path = ?1",
                params![path.to_string_lossy()],
            )
            .context("removing pdf document")?;
        Ok(())
    }

    // ─── Graph layout ────────────────────────────────────────────────────

    /// Saved graph node positions, keyed by node key.
    pub fn load_graph_layout(&self) -> Result<HashMap<String, [f32; 2]>> {
        let mut stmt = self
            .conn
            .prepare("SELECT node_key, x, y FROM graph_layout")
            .context("preparing graph layout query")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, f64>(2)?,
                ))
            })
            .context("querying graph layout")?;
        let mut out = HashMap::new();
        for row in rows {
            let (key, x, y) = row.context("reading graph layout row")?;
            out.insert(key, [x as f32, y as f32]);
        }
        Ok(out)
    }

    /// Replaces the saved graph layout with `positions`.
    pub fn save_graph_layout(&mut self, positions: &[(String, [f32; 2])]) -> Result<()> {
        let tx = self.conn.transaction().context("starting layout tx")?;
        tx.execute("DELETE FROM graph_layout", [])
            .context("clearing graph layout")?;
        for (key, [x, y]) in positions {
            tx.execute(
                "INSERT INTO graph_layout (node_key, x, y) VALUES (?1, ?2, ?3)",
                params![key, *x as f64, *y as f64],
            )
            .context("saving graph node position")?;
        }
        tx.commit().context("committing layout tx")?;
        Ok(())
    }
}

fn insert_note(tx: &Transaction, note: &Note) -> Result<()> {
    let fm = &note.frontmatter;
    let id = fm.id.to_string();
    tx.execute(
        "INSERT OR REPLACE INTO notes_index
            (id, path, title, title_key, tags, pinned, archived, trashed, modified)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            note.path.to_string_lossy(),
            fm.title,
            title_key(&fm.title),
            note.effective_tags().join(","),
            fm.pinned,
            fm.archived,
            fm.trashed,
            fm.modified.to_rfc3339(),
        ],
    )
    .with_context(|| format!("indexing note {}", note.path.display()))?;
    // Canvas bodies are serialized diagrams, not prose with links.
    if note.is_canvas() {
        return Ok(());
    }
    for occ in wikilink::parse_wikilinks(&note.body) {
        tx.execute(
            "INSERT INTO links (src_id, target, target_key, heading, line, context)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                occ.link.target,
                title_key(&occ.link.target),
                occ.link.heading,
                occ.line as i64,
                occ.context,
            ],
        )
        .with_context(|| format!("indexing links of {}", note.path.display()))?;
    }
    Ok(())
}

fn create_vector_tables(conn: &Connection, dim: usize) -> Result<()> {
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS vec_chunks USING vec0(
             chunk_id INTEGER PRIMARY KEY,
             embedding float[{dim}] distance_metric=cosine
         );
         CREATE VIRTUAL TABLE IF NOT EXISTS vec_docs USING vec0(
             doc_row INTEGER PRIMARY KEY,
             embedding float[{dim}] distance_metric=cosine
         );"
    ))
    .context("creating vector tables")
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE name = ?1",
        [name],
        |_| Ok(()),
    )
    .optional()
    .map(|r| r.is_some())
    .with_context(|| format!("checking for table {name}"))
}

/// Removes every row derived from `doc_id` (chunks, FTS, vectors,
/// document entry). Vector tables may not exist yet.
fn delete_doc(conn: &Connection, doc_id: Uuid) -> Result<()> {
    let id = doc_id.to_string();
    conn.execute(
        "DELETE FROM chunks_fts WHERE rowid IN (SELECT id FROM document_chunks WHERE doc_id = ?1)",
        [&id],
    )
    .context("clearing chunk text index for doc")?;
    if table_exists(conn, "vec_chunks")? {
        conn.execute(
            "DELETE FROM vec_chunks WHERE chunk_id IN (SELECT id FROM document_chunks WHERE doc_id = ?1)",
            [&id],
        )
        .context("clearing chunk vectors for doc")?;
        conn.execute(
            "DELETE FROM vec_docs WHERE doc_row IN (SELECT id FROM documents WHERE doc_id = ?1)",
            [&id],
        )
        .context("clearing document vector")?;
    }
    conn.execute("DELETE FROM documents WHERE doc_id = ?1", [&id])
        .context("clearing document entry")?;
    conn.execute("DELETE FROM document_chunks WHERE doc_id = ?1", [&id])
        .context("clearing existing chunks for doc")?;
    conn.execute("DELETE FROM document_hashes WHERE doc_id = ?1", [&id])
        .context("clearing document hash")?;
    Ok(())
}

/// Reads columns 0..=4 (`doc_id, file_path, page_num, char_offset,
/// text_content`) into a chunk. The inner `Result` carries a malformed-uuid
/// error that rusqlite's row mapper can't express.
fn read_chunk(row: &rusqlite::Row) -> rusqlite::Result<Result<DocumentChunk>> {
    let doc_id: String = row.get(0)?;
    let file_path: String = row.get(1)?;
    let page_num: Option<i64> = row.get(2)?;
    let char_offset: i64 = row.get(3)?;
    let text_content: String = row.get(4)?;
    Ok(parse_uuid(&doc_id).map(|doc_id| DocumentChunk {
        doc_id,
        file_path: PathBuf::from(file_path),
        page_num: page_num.map(|p| p as usize),
        char_offset: char_offset as usize,
        text_content,
    }))
}

fn parse_uuid(s: &str) -> Result<Uuid> {
    Uuid::parse_str(s).with_context(|| format!("parsing stored id '{s}'"))
}

/// Words FTS5's `unicode61` tokenizer would see, used to build safe
/// queries (user text is never passed to MATCH verbatim).
fn fts_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Builds an FTS5 MATCH expression from free text: every word quoted, the
/// last one as a prefix (so results appear while typing), joined with
/// implicit AND or with `OR` when `any`. `None` if there are no words.
#[cfg(test)]
fn fts_query(text: &str, any: bool) -> Option<String> {
    fts_query_parsed(&crate::notes::query::ParsedQuery::parse(text), any)
}

/// FTS5 MATCH expression for a parsed query: every term quoted (the last
/// one prefix-matched, so typing feels live), phrases kept whole, `-x`
/// terms excluded with `NOT`. `None` when there is nothing positive to
/// search for.
fn fts_query_parsed(parsed: &crate::notes::query::ParsedQuery, any: bool) -> Option<String> {
    let words = fts_tokens(&parsed.terms.join(" "));
    let last = words.len().checked_sub(1);
    let mut positive: Vec<String> = words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            if Some(i) == last {
                format!("\"{w}\"*")
            } else {
                format!("\"{w}\"")
            }
        })
        .collect();
    for phrase in &parsed.phrases {
        let tokens = fts_tokens(phrase);
        if !tokens.is_empty() {
            positive.push(format!("\"{}\"", tokens.join(" ")));
        }
    }
    if positive.is_empty() {
        return None;
    }
    let joined = positive.join(if any { " OR " } else { " " });
    let mut query = if any && positive.len() > 1 { format!("({joined})") } else { joined };
    for ex in &parsed.excluded {
        let tokens = fts_tokens(ex);
        if !tokens.is_empty() {
            query = format!("{query} NOT \"{}\"", tokens.join(" "));
        }
    }
    Some(query)
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
        std::fs::remove_file(IndexStore::index_path(dir.path())).unwrap();

        let mut store = IndexStore::open(dir.path()).unwrap();
        assert_eq!(store.count().unwrap(), 0); // fresh db, empty until rebuild
        store.rebuild(&[note]).unwrap();
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn outdated_schema_is_dropped_but_pdf_list_survives() {
        let dir = tempdir().unwrap();
        {
            let conn = Connection::open(dir.path().join(".mnemonic-index.sqlite3")).unwrap();
            conn.execute_batch(
                "CREATE TABLE document_chunks (id INTEGER PRIMARY KEY, embedding BLOB NOT NULL);
                 INSERT INTO document_chunks (embedding) VALUES (x'00');
                 CREATE TABLE pdf_documents (path TEXT PRIMARY KEY, imported_at TEXT NOT NULL);
                 INSERT INTO pdf_documents VALUES ('a.pdf', '2026-01-01T00:00:00Z');",
            )
            .unwrap();
        }
        let store = IndexStore::open(dir.path()).unwrap();
        assert_eq!(store.chunk_count().unwrap(), 0);
        assert_eq!(
            store.list_pdf_documents().unwrap(),
            vec![PathBuf::from("a.pdf")]
        );
    }

    fn sample_chunk(doc_id: Uuid, text: &str) -> (DocumentChunk, Vec<f32>) {
        chunk_with(doc_id, text, vec![0.1, 0.2, 0.3])
    }

    fn chunk_with(doc_id: Uuid, text: &str, embedding: Vec<f32>) -> (DocumentChunk, Vec<f32>) {
        (
            DocumentChunk {
                doc_id,
                file_path: PathBuf::from("catatan.md"),
                page_num: None,
                char_offset: 0,
                text_content: text.to_string(),
            },
            embedding,
        )
    }

    #[test]
    fn replace_chunks_inserts_and_round_trips_embeddings() {
        let mut store = IndexStore::open_in_memory().unwrap();
        let doc_id = Uuid::new_v4();
        let chunks = vec![sample_chunk(doc_id, "halo dunia")];

        store
            .replace_chunks(doc_id, "note", "Salam", &chunks)
            .unwrap();

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
            .replace_chunks(doc_id, "note", "T", &[sample_chunk(doc_id, "versi lama")])
            .unwrap();
        assert_eq!(store.chunk_count().unwrap(), 1);

        store
            .replace_chunks(doc_id, "note", "T", &[sample_chunk(doc_id, "versi baru")])
            .unwrap();

        let stored = store.all_chunks().unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].chunk.text_content, "versi baru");
        assert!(store.keyword_chunks("lama", 5).unwrap().is_empty());
        assert_eq!(store.knn_chunks(&[0.1, 0.2, 0.3], 5).unwrap().len(), 1);
    }

    #[test]
    fn replace_chunks_does_not_touch_other_documents() {
        let mut store = IndexStore::open_in_memory().unwrap();
        let doc_a = Uuid::new_v4();
        let doc_b = Uuid::new_v4();

        store
            .replace_chunks(doc_a, "note", "A", &[sample_chunk(doc_a, "punya A")])
            .unwrap();
        store
            .replace_chunks(doc_b, "pdf", "B", &[sample_chunk(doc_b, "punya B")])
            .unwrap();
        assert_eq!(store.chunk_count().unwrap(), 2);

        store
            .replace_chunks(doc_a, "note", "A", &[sample_chunk(doc_a, "A diedit")])
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
            .replace_chunks(doc_a, "note", "A", &[sample_chunk(doc_a, "A")])
            .unwrap();
        store
            .replace_chunks(doc_b, "note", "B", &[sample_chunk(doc_b, "B")])
            .unwrap();

        store.delete_chunks_for_doc(doc_a).unwrap();

        assert_eq!(store.chunk_count().unwrap(), 1);
        assert_eq!(store.all_chunks().unwrap()[0].chunk.doc_id, doc_b);
        let knn = store.knn_chunks(&[0.1, 0.2, 0.3], 10).unwrap();
        assert_eq!(knn.len(), 1);
        assert_eq!(knn[0].0.doc_id, doc_b);
    }

    #[test]
    fn delete_chunks_for_unindexed_doc_is_a_noop() {
        let store = IndexStore::open_in_memory().unwrap();
        store.delete_chunks_for_doc(Uuid::new_v4()).unwrap();
    }

    #[test]
    fn replace_chunks_with_empty_slice_clears_the_doc() {
        let mut store = IndexStore::open_in_memory().unwrap();
        let doc_id = Uuid::new_v4();
        store
            .replace_chunks(doc_id, "note", "T", &[sample_chunk(doc_id, "isi")])
            .unwrap();
        assert_eq!(store.chunk_count().unwrap(), 1);

        store.replace_chunks(doc_id, "note", "T", &[]).unwrap();
        assert_eq!(store.chunk_count().unwrap(), 0);
    }

    #[test]
    fn replace_chunks_rejects_mismatched_dimensions() {
        let mut store = IndexStore::open_in_memory().unwrap();
        let a = Uuid::new_v4();
        store
            .replace_chunks(a, "note", "A", &[sample_chunk(a, "x")])
            .unwrap();
        let b = Uuid::new_v4();
        assert!(
            store
                .replace_chunks(b, "note", "B", &[chunk_with(b, "y", vec![1.0, 0.0])])
                .is_err()
        );
    }

    #[test]
    fn knn_chunks_ranks_by_cosine_similarity() {
        let mut store = IndexStore::open_in_memory().unwrap();
        let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        store
            .replace_chunks(a, "note", "A", &[chunk_with(a, "sama", vec![1.0, 0.0])])
            .unwrap();
        store
            .replace_chunks(b, "note", "B", &[chunk_with(b, "miring", vec![0.7, 0.7])])
            .unwrap();
        store
            .replace_chunks(c, "note", "C", &[chunk_with(c, "lawan", vec![-1.0, 0.0])])
            .unwrap();

        let hits = store.knn_chunks(&[2.0, 0.0], 2).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].0.doc_id, a);
        assert!((hits[0].1 - 1.0).abs() < 1e-5);
        assert_eq!(hits[1].0.doc_id, b);
        assert!((hits[1].1 - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
    }

    #[test]
    fn knn_on_empty_store_returns_nothing() {
        let store = IndexStore::open_in_memory().unwrap();
        assert!(store.knn_chunks(&[1.0, 0.0], 5).unwrap().is_empty());
        assert!(store.similar_documents(Uuid::new_v4(), 5).unwrap().is_empty());
    }

    #[test]
    fn keyword_chunks_matches_text_and_title_with_snippet() {
        let mut store = IndexStore::open_in_memory().unwrap();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        store
            .replace_chunks(
                a,
                "note",
                "Masakan",
                &[chunk_with(a, "Resep nasi goreng kecap manis", vec![1.0, 0.0])],
            )
            .unwrap();
        store
            .replace_chunks(
                b,
                "note",
                "Keuangan",
                &[chunk_with(b, "Laporan kuartal ketiga", vec![0.0, 1.0])],
            )
            .unwrap();

        let hits = store.keyword_chunks("nasi gor", 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chunk.doc_id, a);
        assert!(hits[0].snippet.contains("\u{2}nasi\u{3}"));

        let by_title = store.keyword_chunks("keuangan", 5).unwrap();
        assert_eq!(by_title[0].chunk.doc_id, b);

        // No document has both words: falls back to any-word matching.
        let either = store.keyword_chunks("nasi laporan", 5).unwrap();
        assert_eq!(either.len(), 2);

        // Quotes / operators in user input never break the MATCH syntax.
        assert!(store.keyword_chunks("\"AND (", 5).unwrap().is_empty());
        assert!(store.keyword_chunks("   ", 5).unwrap().is_empty());
    }

    #[test]
    fn keyword_search_ignores_diacritics() {
        let mut store = IndexStore::open_in_memory().unwrap();
        let a = Uuid::new_v4();
        store
            .replace_chunks(a, "note", "Kafe", &[chunk_with(a, "Minum di café", vec![1.0])])
            .unwrap();
        assert_eq!(store.keyword_chunks("cafe", 5).unwrap().len(), 1);
    }

    #[test]
    fn similar_documents_uses_mean_vectors_and_excludes_self() {
        let mut store = IndexStore::open_in_memory().unwrap();
        let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        store
            .replace_chunks(
                a,
                "note",
                "A",
                &[
                    chunk_with(a, "a1", vec![1.0, 0.2]),
                    chunk_with(a, "a2", vec![1.0, -0.2]),
                ],
            )
            .unwrap();
        store
            .replace_chunks(b, "note", "B", &[chunk_with(b, "b", vec![0.9, 0.1])])
            .unwrap();
        store
            .replace_chunks(c, "note", "C", &[chunk_with(c, "c", vec![0.0, 1.0])])
            .unwrap();

        let similar = store.similar_documents(a, 1).unwrap();
        assert_eq!(similar.len(), 1);
        assert_eq!(similar[0].0, b);

        store.delete_chunks_for_doc(b).unwrap();
        let similar = store.similar_documents(a, 5).unwrap();
        assert_eq!(similar.iter().map(|s| s.0).collect::<Vec<_>>(), vec![c]);
    }

    #[test]
    fn ensure_embedding_model_resets_cache_only_when_model_changes() {
        let mut store = IndexStore::open_in_memory().unwrap();
        assert!(store.ensure_embedding_model("model-a", 2).unwrap());
        let a = Uuid::new_v4();
        store
            .replace_chunks(a, "note", "A", &[chunk_with(a, "x", vec![1.0, 0.0])])
            .unwrap();

        assert!(!store.ensure_embedding_model("model-a", 2).unwrap());
        assert_eq!(store.chunk_count().unwrap(), 1);

        assert!(store.ensure_embedding_model("model-b", 3).unwrap());
        assert_eq!(store.chunk_count().unwrap(), 0);
        store
            .replace_chunks(a, "note", "A", &[chunk_with(a, "x", vec![1.0, 0.0, 0.0])])
            .unwrap();
        assert_eq!(store.knn_chunks(&[1.0, 0.0, 0.0], 1).unwrap().len(), 1);
    }

    #[test]
    fn backlinks_come_from_the_link_table_with_context() {
        let dir = tempdir().unwrap();
        let target = Note::create(dir.path(), "Target", "").unwrap();
        let a = Note::create(dir.path(), "Alpha", "intro\nLihat [[target#Bagian|ini]].").unwrap();
        let mut trashed = Note::create(dir.path(), "Sampah", "[[Target]]").unwrap();
        trashed.frontmatter.trashed = true;
        let own = Note::create(dir.path(), "Diri", "[[Diri]]").unwrap();

        let mut store = IndexStore::open_in_memory().unwrap();
        store
            .rebuild(&[target.clone(), a.clone(), trashed, own.clone()])
            .unwrap();

        let back = store.backlinks("TARGET", target.frontmatter.id).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].src_id, a.frontmatter.id);
        assert_eq!(back[0].src_title, "Alpha");
        assert_eq!(back[0].line, 1);
        assert_eq!(back[0].context, "Lihat [[target#Bagian|ini]].");
        assert!(store.backlinks("Diri", own.frontmatter.id).unwrap().is_empty());
    }

    #[test]
    fn upsert_note_refreshes_links_for_one_note() {
        let dir = tempdir().unwrap();
        let mut a = Note::create(dir.path(), "A", "[[B]]").unwrap();
        let b = Note::create(dir.path(), "B", "").unwrap();
        let mut store = IndexStore::open_in_memory().unwrap();
        store.rebuild(&[a.clone(), b.clone()]).unwrap();
        assert_eq!(store.link_edges().unwrap().len(), 1);

        a.body = "[[C]] dan [[D]]".to_string();
        a.frontmatter.title = "A2".to_string();
        store.upsert_note(&a).unwrap();

        let edges = store.link_edges().unwrap();
        let keys: Vec<&str> = edges.iter().map(|e| e.target_key.as_str()).collect();
        assert_eq!(keys, vec!["c", "d"]);
        assert_eq!(store.count().unwrap(), 2);
        assert!(store.list_titles().unwrap().contains(&"A2".to_string()));
    }

    #[test]
    fn canvas_notes_contribute_no_links() {
        let dir = tempdir().unwrap();
        let canvas = Note::create_canvas(dir.path(), "Papan").unwrap();
        let mut store = IndexStore::open_in_memory().unwrap();
        store.rebuild(&[canvas]).unwrap();
        assert!(store.link_edges().unwrap().is_empty());
    }

    #[test]
    fn graph_layout_round_trips() {
        let mut store = IndexStore::open_in_memory().unwrap();
        store
            .save_graph_layout(&[("note:1".into(), [1.5, -2.0]), ("pdf:x".into(), [0.0, 3.0])])
            .unwrap();
        let layout = store.load_graph_layout().unwrap();
        assert_eq!(layout.len(), 2);
        assert_eq!(layout["note:1"], [1.5, -2.0]);
        store.save_graph_layout(&[]).unwrap();
        assert!(store.load_graph_layout().unwrap().is_empty());
    }

    #[test]
    fn fts_query_quotes_words_and_prefixes_the_last() {
        assert_eq!(
            fts_query("nasi gor", false).as_deref(),
            Some("\"nasi\" \"gor\"*")
        );
        assert_eq!(fts_query("a-b", true).as_deref(), Some("(\"a\" OR \"b\"*)"));
        assert_eq!(fts_query(" \"( ", false), None);
        assert_eq!(
            fts_query("nasi \"sayur asem\" -telur tag:rumah", false).as_deref(),
            Some("\"nasi\"* \"sayur asem\" NOT \"telur\"")
        );
        assert_eq!(fts_query("-telur", false), None);
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

        assert_eq!(
            store.list_pdf_documents().unwrap(),
            vec![PathBuf::from("a.pdf")]
        );
    }

    #[test]
    fn remove_pdf_document_drops_only_that_path() {
        let store = IndexStore::open_in_memory().unwrap();
        store.add_pdf_document(Path::new("a.pdf")).unwrap();
        store.add_pdf_document(Path::new("b.pdf")).unwrap();

        store.remove_pdf_document(Path::new("a.pdf")).unwrap();

        assert_eq!(
            store.list_pdf_documents().unwrap(),
            vec![PathBuf::from("b.pdf")]
        );
    }
}
