//! Core backend services shared across UI modules: SQLite index cache.
//! Chunking/embedding (§3.3) land here in a later phase. Callers: `app.rs`.

pub mod storage;

pub use storage::IndexStore;
