//! Local generative AI assistant via Candle + Qwen2.5 (§3.4, Fase 6):
//! `candle_engine` loads the quantized Qwen2.5-1.5B-Instruct GGUF model
//! and runs CPU inference; `prompt` builds the strict-grounding ChatML
//! RAG prompt from retrieved `core::ingestion::DocumentChunk`s; `stream`
//! is the background worker that runs generation off the UI thread and
//! streams decoded text fragments back over a channel (§6 risk 2).
//! Callers: future chat UI (§Fase 7) — retrieval
//! (hybrid search over `core::storage::IndexStore`)
//! stays in the caller, same split as `core::indexer` leaving
//! `IndexStore` writes to `app.rs`.

/// Mesin Candle (model di perangkat); butuh fitur `semantic`.
#[cfg(feature = "semantic")]
pub mod candle_engine;
pub mod prompt;
/// Worker generasi latar; butuh fitur `semantic`.
#[cfg(feature = "semantic")]
pub mod stream;

#[cfg(feature = "semantic")]
pub use candle_engine::CandleEngine;
pub use prompt::{SIMILARITY_THRESHOLD, build_rag_prompt, select_context};
/// Batas token bawaan `ask` — dipakai juga oleh skema tool MCP, jadi
/// nilainya tetap ada pada build tanpa fitur `semantic`.
pub const DEFAULT_MAX_TOKENS: usize = 512;
#[cfg(feature = "semantic")]
pub use stream::{GenerationEvent, GenerationWorker, Generator};
