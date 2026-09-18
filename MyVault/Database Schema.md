---
id: 08a9b0c1-d2e3-4567-89ab-cdef01234567
title: Database Schema
type: note
created: 2026-08-20T09:00:00+07:00
modified: 2026-09-16T11:30:00+07:00
pinned: false
color: default
tags: [database, schema, backend]
aliases: [DB Schema, Skema Database]
archived: false
trashed: false
---

# 🗄️ Database Schema — Platform AI

Skema database untuk [[Proyek Startup AI]]. Arsitektur lengkap di [[Arsitektur Microservice]].

## ER Diagram

```mermaid
erDiagram
    VAULT ||--o{ NOTE : contains
    VAULT ||--o{ SHEET : contains
    VAULT ||--o{ PDF_DOC : contains
    NOTE ||--o{ CHUNK : "split into"
    NOTE }o--o{ TAG : "labeled with"
    NOTE ||--o{ WIKILINK : "links to"
    NOTE ||--o{ BLOCK_ANCHOR : has
    SHEET ||--o{ CHUNK : "chunked as"
    PDF_DOC ||--o{ CHUNK : "extracted to"
    CHUNK ||--|| EMBEDDING : "vectorized"

    VAULT {
        string path PK
        datetime last_scan
        int note_count
        int indexed_count
    }
    NOTE {
        uuid id PK
        string title
        string path
        string folder
        enum note_type "note|checklist|canvas"
        text body
        string color
        boolean pinned
        boolean archived
        boolean trashed
        datetime created
        datetime modified
        string content_hash
    }
    TAG {
        string name PK
        string color
        int note_count
    }
    WIKILINK {
        uuid source_id FK
        string target_title
        string target_heading
        int line_number
        string resolved_path
    }
    BLOCK_ANCHOR {
        uuid note_id FK
        string anchor_id
        int line_number
        text content_preview
    }
    CHUNK {
        uuid id PK
        uuid doc_id FK
        string doc_type "note|sheet|pdf"
        int char_offset
        int page_num
        text content
        string content_hash
    }
    EMBEDDING {
        uuid chunk_id FK
        blob vector "384-dim float32"
        string model_name
    }
    SHEET {
        uuid id PK
        string path
        string delimiter
        int row_count
        int col_count
    }
    PDF_DOC {
        uuid id PK
        string path
        int page_count
        string title_meta
        string author_meta
    }
```

## Tabel SQLite Index

> [!note] Index ≠ Source of Truth
> SQLite hanya menyimpan **index cache**. Source of truth selalu file `.md`
> di disk. Index bisa dihapus dan dibangun ulang kapan saja.

### `notes_index`

```sql
CREATE TABLE notes_index (
    id          TEXT PRIMARY KEY,
    title       TEXT NOT NULL,
    path        TEXT NOT NULL UNIQUE,
    folder      TEXT DEFAULT '',
    note_type   TEXT DEFAULT 'note',
    color       TEXT DEFAULT 'default',
    pinned      INTEGER DEFAULT 0,
    archived    INTEGER DEFAULT 0,
    trashed     INTEGER DEFAULT 0,
    created     TEXT NOT NULL,
    modified    TEXT NOT NULL,
    content_hash TEXT,
    has_canvas  INTEGER DEFAULT 0
);

CREATE INDEX idx_notes_folder ON notes_index(folder);
CREATE INDEX idx_notes_modified ON notes_index(modified DESC);
```

### `chunks` + FTS5

```sql
CREATE TABLE chunks (
    id            TEXT PRIMARY KEY,
    doc_id        TEXT NOT NULL,
    doc_type      TEXT DEFAULT 'note',
    file_path     TEXT NOT NULL,
    page_num      INTEGER,
    char_offset   INTEGER NOT NULL,
    content       TEXT NOT NULL,
    content_hash  TEXT
);

-- Full-text search index (keyword/BM25)
CREATE VIRTUAL TABLE chunks_fts USING fts5(
    content,
    content=chunks,
    content_rowid=rowid
);

-- Vector embeddings (semantic search via sqlite-vec)
CREATE VIRTUAL TABLE chunks_vec USING vec0(
    chunk_id TEXT PRIMARY KEY,
    embedding FLOAT[384]
);
```

### `links`

```sql
CREATE TABLE links (
    source_id      TEXT NOT NULL,
    target_title   TEXT NOT NULL,
    target_heading TEXT,
    line_number    INTEGER,
    resolved_path  TEXT,
    UNIQUE(source_id, target_title, line_number)
);

CREATE INDEX idx_links_target ON links(target_title);
```

## Query Patterns

### Hybrid Search (RRF)

```sql
-- 1. Keyword search via FTS5
SELECT doc_id, snippet(chunks_fts, 0, '<b>', '</b>', '...', 32) as snippet,
       rank as bm25_score
FROM chunks_fts
WHERE chunks_fts MATCH ?
ORDER BY rank
LIMIT 20;

-- 2. Semantic search via sqlite-vec
SELECT chunk_id, distance
FROM chunks_vec
WHERE embedding MATCH ?
  AND k = 20
ORDER BY distance;

-- 3. Combine with RRF in application layer
```

### Backlinks Query

```sql
SELECT n.id, n.title, n.path, l.line_number
FROM links l
JOIN notes_index n ON n.id = l.source_id
WHERE l.target_title = ? COLLATE NOCASE
   OR l.target_title = ? COLLATE NOCASE  -- file stem
ORDER BY n.modified DESC;
```

Lihat juga: [[Arsitektur Microservice#Entity Relationship — Data Model]]

#database #schema #backend #SQL
