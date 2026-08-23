//! Local generative AI assistant via Candle + Qwen2.5 (§3.4, Fase 6):
//! `candle_engine` loads the quantized Qwen2.5-1.5B-Instruct GGUF model
//! and runs CPU inference; `prompt` builds the strict-grounding ChatML
//! RAG prompt from retrieved `core::ingestion::DocumentChunk`s; `stream`
//! is the background worker that runs generation off the UI thread and
//! streams decoded text fragments back over a channel (§6 risk 2).
//! Callers: future chat UI (§Fase 7) — retrieval
//! (`core::embedding::top_k` over `core::storage::IndexStore::all_chunks`)
//! stays in the caller, same split as `core::indexer` leaving
//! `IndexStore` writes to `app.rs`.

pub mod candle_engine;
pub mod prompt;
pub mod stream;

pub use candle_engine::CandleEngine;
pub use prompt::{SIMILARITY_THRESHOLD, build_rag_prompt, select_context};
pub use stream::{DEFAULT_MAX_TOKENS, GenerationEvent, GenerationWorker, Generator};
