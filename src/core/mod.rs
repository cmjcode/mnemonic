//! Core backend services shared across UI modules: SQLite index cache,
//! text chunking, FastEmbed-based embedding/retrieval (§3.3), the
//! background indexing worker that ties them together (§5), and the
//! pure keyword+semantic search-ranking logic (§Fase 7) that consumes
//! all of the above. Callers: `app.rs`.

pub mod chunker;
pub mod embedding;
pub mod indexer;
pub mod ingestion;
pub mod search;
pub mod storage;

pub use embedding::{EmbeddingEngine, cosine_similarity, top_k};
pub use indexer::{DocType, IndexJob, IndexResult, IndexingWorker};
pub use ingestion::DocumentChunk;
pub use search::{SearchHit, keyword_search, merge_results, semantic_search};
pub use storage::{IndexStore, StoredChunk};
