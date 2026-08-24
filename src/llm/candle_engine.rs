//! Candle runtime & Qwen2.5-Instruct GGUF loader (§3.4 points 1 & 3):
//! wraps `candle_transformers`'s quantized Qwen2 implementation to run
//! the RAG generator fully offline on CPU. Mirrors
//! `core::embedding::EmbeddingEngine`'s split: model download is the
//! only network-touching part of this module, and `generate` stays a
//! plain function of `(prompt, max_tokens, callback) -> Result<()>` so
//! it's swappable behind the `Generator` trait (`llm::stream`) for
//! offline testing. Callers: `llm::stream::GenerationWorker` (production
//! spawn), fed by `llm::prompt::build_rag_prompt`'s output.
//!
//! **CPU parallelism (§Fase 10, §5 "multi-threading SIMD/AVX2"):** this
//! module has no threading/SIMD code of its own — both come from
//! `candle-core`'s quantized matmul, and both are already engaged without
//! anything special here: multi-threading via a `rayon` pool sized to
//! available parallelism by default (`candle_core::utils::get_num_threads`,
//! overridable with the `CANDLE_NUM_THREADS`/`RAYON_NUM_THREADS` env vars),
//! and NEON SIMD on aarch64 (Apple Silicon) automatically, since NEON is
//! architecture-baseline there. AVX2 SIMD on x86_64 is *not* on by default
//! — `candle-core` gates its AVX2 kernel on the `target-feature = "avx2"`
//! compile-time cfg, and a plain `x86_64-*` Rust target only guarantees
//! SSE2 — so the repo's `.cargo/config.toml` sets
//! `-C target-feature=+avx2,+fma` for the x86_64 release triples (see that
//! file's comment for the portability tradeoff this implies).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use candle_core::quantized::gguf_file;
use candle_core::{Device, Tensor};
use candle_transformers::generation::{LogitsProcessor, Sampling};
use candle_transformers::models::quantized_qwen2::ModelWeights;
use candle_transformers::utils::apply_repeat_penalty;
use tokenizers::Tokenizer;

use super::stream::Generator;

/// Default model on Hugging Face Hub (§3.4 point 1): the 1.5B Q4_K_M
/// quantization, ~1.1 GB, chosen per §6 risk 1 to keep model RAM use in
/// the ~1.1–1.3 GB range on standard hardware.
const DEFAULT_REPO: &str = "Qwen/Qwen2.5-1.5B-Instruct-GGUF";
const DEFAULT_GGUF_FILE: &str = "qwen2.5-1.5b-instruct-q4_k_m.gguf";
/// The GGUF repo above doesn't ship its own `tokenizer.json`; the
/// unquantized instruct repo's tokenizer is identical (quantization only
/// touches weights, not vocabulary) and is guaranteed to have one.
const TOKENIZER_REPO: &str = "Qwen/Qwen2.5-1.5B-Instruct";
const TOKENIZER_FILE: &str = "tokenizer.json";

/// Qwen2.5 chat models use ChatML's `<|im_end|>` as the turn-boundary
/// token; that's what stops generation here (§3.4 point 2's template),
/// not the base model's `<|endoftext|>`.
const EOS_TOKEN: &str = "<|im_end|>";

const SAMPLING_SEED: u64 = 299_792_458; // arbitrary fixed seed: reproducible sampling run-to-run
const TOP_K: usize = 40;
const TOP_P: f64 = 0.9;
const TEMPERATURE: f64 = 0.7;
const REPEAT_PENALTY: f32 = 1.1;
const REPEAT_LAST_N: usize = 64;

/// A Qwen2.5-Instruct GGUF model ready to stream-generate replies. Holds
/// the GGUF path rather than a loaded `ModelWeights` — see
/// `load_model`'s doc comment for why.
pub struct CandleEngine {
    gguf_path: PathBuf,
    tokenizer: Tokenizer,
    device: Device,
    eos_token_id: u32,
}

impl CandleEngine {
    /// Downloads (first run) or loads from the local Hugging Face cache
    /// (subsequent runs — fully offline once cached, matching
    /// `EmbeddingEngine::new`'s behavior) the default
    /// `Qwen2.5-1.5B-Instruct` Q4_K_M GGUF weights and tokenizer, then
    /// builds the CPU inference engine.
    pub fn new() -> Result<CandleEngine> {
        let api = hf_hub::api::sync::Api::new().context("creating Hugging Face Hub API client")?;
        let gguf_path = api
            .model(DEFAULT_REPO.to_string())
            .get(DEFAULT_GGUF_FILE)
            .with_context(|| format!("downloading {DEFAULT_GGUF_FILE} from {DEFAULT_REPO}"))?;
        let tokenizer_path = api
            .model(TOKENIZER_REPO.to_string())
            .get(TOKENIZER_FILE)
            .with_context(|| format!("downloading {TOKENIZER_FILE} from {TOKENIZER_REPO}"))?;

        Self::load(&gguf_path, &tokenizer_path)
    }

    /// Builds the engine from already-present local files — the path
    /// `new()` uses after downloading, and usable directly by tests or a
    /// custom local checkpoint under `assets/models/llm/` (§4) without
    /// touching the network.
    pub fn load(gguf_path: &Path, tokenizer_path: &Path) -> Result<CandleEngine> {
        let device = Device::Cpu;

        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow::anyhow!("loading tokenizer {}: {e}", tokenizer_path.display()))?;
        let eos_token_id = tokenizer.token_to_id(EOS_TOKEN).with_context(|| {
            format!("tokenizer is missing the expected '{EOS_TOKEN}' chat turn token")
        })?;

        let engine = CandleEngine {
            gguf_path: gguf_path.to_path_buf(),
            tokenizer,
            device,
            eos_token_id,
        };
        // Fail fast on a bad/unreadable GGUF file here rather than
        // deferring the error to the first `generate()` call.
        engine
            .load_model()
            .with_context(|| format!("loading Qwen2.5 weights from {}", gguf_path.display()))?;
        Ok(engine)
    }

    /// Builds a fresh `ModelWeights` by re-reading the GGUF file. Re-
    /// parsing an already-local file on every `generate()` call (instead
    /// of loading once in `load()` and reusing that instance) is
    /// deliberate: `quantized_qwen2::ModelWeights` keeps its KV cache as
    /// internal mutable state with no public reset method, so reusing
    /// one loaded model across chat turns would leak stale attention
    /// state (and RoPE position indices) from the previous turn into the
    /// next. A fresh load guarantees each `generate()` call starts from
    /// a clean cache, at the cost of re-parsing ~1.1 GB from local disk
    /// each turn — an accepted tradeoff for Fase 6; revisit in Fase 10
    /// (Optimasi) if candle-transformers grows a cheaper reset API.
    fn load_model(&self) -> Result<ModelWeights> {
        let mut file = std::fs::File::open(&self.gguf_path)
            .with_context(|| format!("opening GGUF weights {}", self.gguf_path.display()))?;
        let content = gguf_file::Content::read(&mut file)
            .with_context(|| format!("reading GGUF header {}", self.gguf_path.display()))?;
        ModelWeights::from_gguf(content, &mut file, &self.device)
            .with_context(|| format!("loading Qwen2.5 weights from {}", self.gguf_path.display()))
    }

    fn encode(&self, text: &str) -> Result<Vec<u32>> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("tokenizing prompt: {e}"))?;
        Ok(encoding.get_ids().to_vec())
    }

    fn decode(&self, ids: &[u32]) -> Result<String> {
        self.tokenizer
            .decode(ids, true)
            .map_err(|e| anyhow::anyhow!("decoding tokens: {e}"))
    }
}

impl Generator for CandleEngine {
    /// Streams a reply to `prompt` (already-built ChatML text from
    /// `llm::prompt::build_rag_prompt`) token-by-token, invoking
    /// `on_token` with each newly-decoded text fragment as it becomes
    /// available. Stops at `max_tokens` generated tokens or the
    /// `<|im_end|>` turn boundary, whichever comes first (§3.4 point 3).
    ///
    /// Decoding re-runs the tokenizer over the whole generated-so-far
    /// token list each step rather than decoding one id at a time,
    /// because a single new token can be part of a multi-byte UTF-8
    /// character or a multi-token word-piece — re-decoding and diffing
    /// the emitted character count is what keeps streamed fragments
    /// valid text instead of splitting a character mid-byte.
    fn generate(
        &mut self,
        prompt: &str,
        max_tokens: usize,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<()> {
        let prompt_tokens = self.encode(prompt)?;
        if prompt_tokens.is_empty() || max_tokens == 0 {
            return Ok(());
        }

        let mut model = self.load_model()?;
        let mut logits_processor = LogitsProcessor::from_sampling(
            SAMPLING_SEED,
            Sampling::TopKThenTopP {
                k: TOP_K,
                p: TOP_P,
                temperature: TEMPERATURE,
            },
        );

        let mut all_tokens = prompt_tokens.clone();
        let mut emitted_chars = self.decode(&prompt_tokens)?.chars().count();

        let input = Tensor::new(prompt_tokens.as_slice(), &self.device)?.unsqueeze(0)?;
        let mut logits = model
            .forward(&input, 0)
            .context("forward pass over prompt tokens")?
            .squeeze(0)?;

        for step in 0..max_tokens {
            let start = all_tokens.len().saturating_sub(REPEAT_LAST_N);
            let scored = apply_repeat_penalty(&logits, REPEAT_PENALTY, &all_tokens[start..])
                .context("applying repeat penalty")?;
            let next_token = logits_processor
                .sample(&scored)
                .context("sampling next token")?;
            if next_token == self.eos_token_id {
                break;
            }
            all_tokens.push(next_token);

            let decoded = self.decode(&all_tokens)?;
            let delta: String = decoded.chars().skip(emitted_chars).collect();
            emitted_chars = decoded.chars().count();
            if !delta.is_empty() {
                on_token(&delta);
            }

            if step + 1 == max_tokens {
                break;
            }
            let index_pos = prompt_tokens.len() + step;
            let input = Tensor::new(&[next_token], &self.device)?.unsqueeze(0)?;
            logits = model
                .forward(&input, index_pos)
                .with_context(|| format!("forward pass at position {index_pos}"))?
                .squeeze(0)?;
        }

        Ok(())
    }
}

/// End-to-end verification against the real Candle/Qwen2.5 model
/// (§3.4 acceptance: a non-empty streamed reply). Ignored by default —
/// it downloads/loads the ~1.1 GB GGUF weights and needs network on
/// first run; run explicitly with
/// `cargo test -- --ignored generates_a_nonempty_reply_to_a_simple_prompt`.
#[cfg(test)]
mod model_tests {
    use super::*;

    #[test]
    #[ignore]
    fn generates_a_nonempty_reply_to_a_simple_prompt() {
        let mut engine = CandleEngine::new().expect("load Qwen2.5-1.5B-Instruct GGUF");
        let prompt = crate::llm::prompt::build_rag_prompt(&[], "Sebutkan satu warna favoritmu.");

        let mut out = String::new();
        engine
            .generate(&prompt, 32, &mut |t| out.push_str(t))
            .expect("generate");

        assert!(
            !out.trim().is_empty(),
            "expected a non-empty generated reply"
        );
    }
}
