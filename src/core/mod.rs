//! Core backend services shared across UI modules: SQLite index cache,
//! text chunking, and FastEmbed-based embedding/retrieval (§3.3).
//! Callers: `app.rs`.

pub mod chunker;
pub mod embedding;
pub mod ingestion;
pub mod storage;

pub use embedding::{EmbeddingEngine, cosine_similarity, top_k};
pub use ingestion::DocumentChunk;
pub use storage::IndexStore;
