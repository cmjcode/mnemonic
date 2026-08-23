//! Core backend services shared across UI modules: SQLite index cache,
//! text chunking, FastEmbed-based embedding/retrieval (§3.3), and the
//! background indexing worker that ties them together (§5).
//! Callers: `app.rs`.

pub mod chunker;
pub mod embedding;
pub mod indexer;
pub mod ingestion;
pub mod storage;

pub use embedding::{EmbeddingEngine, cosine_similarity, top_k};
pub use indexer::{DocType, IndexJob, IndexResult, IndexingWorker};
pub use ingestion::DocumentChunk;
pub use storage::{IndexStore, StoredChunk};
