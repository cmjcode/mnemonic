//! Core backend services shared across UI modules: SQLite index cache,
//! text chunking, FastEmbed-based embedding/retrieval (§3.3), the
//! background indexing worker that ties them together (§5), and the
//! pure keyword+semantic search-ranking logic (§Fase 7) that consumes
//! all of the above. Callers: `app.rs`.

pub mod chunker;
pub mod embedding;
/// Worker indeks latar (butuh model embedding).
#[cfg(feature = "semantic")]
pub mod indexer;
pub mod ingestion;
pub mod search;
pub mod storage;

pub use embedding::{EMBEDDING_DIM, EMBEDDING_MODEL_ID, cosine_similarity, top_k};
#[cfg(feature = "semantic")]
pub use embedding::{EmbeddingEngine, RerankEngine};
#[cfg(feature = "semantic")]
pub use indexer::{DocType, IndexJob, IndexResult, IndexingWorker};
pub use ingestion::DocumentChunk;
pub use search::{HybridOptions, MatchKind, SearchHit, hybrid_rank, keyword_search};
pub use storage::{Backlink, IndexStore, KeywordChunk, LinkEdge, StoredChunk};
