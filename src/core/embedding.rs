//! Local semantic embedding via FastEmbed (§3.3 point 3): wraps
//! `fastembed::TextEmbedding` (default model `all-MiniLM-L6-v2`, 384-D
//! vectors, §1.2/§2.1) and the cosine-similarity ranking used to score
//! chunks against a query. Model weights download to FastEmbed's on-disk
//! cache the first time `EmbeddingEngine::new` runs — that network/disk
//! hit is isolated to construction, so `cosine_similarity`/`top_k` stay
//! pure and independently testable offline. Callers: `core::ingestion`
//! (embedding chunks), future search/RAG pipeline (§3.3, §3.4).

use anyhow::{Context, Result};
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

/// A loaded FastEmbed text embedding model. Construction is the only
/// network/IO-touching part of this module.
pub struct EmbeddingEngine {
    model: TextEmbedding,
}

impl EmbeddingEngine {
    /// Loads the default retriever model (`all-MiniLM-L6-v2`) — downloads
    /// it into the FastEmbed cache directory on first run, then loads from
    /// there on subsequent runs (fully offline once cached).
    pub fn new() -> Result<EmbeddingEngine> {
        let model = TextEmbedding::try_new(TextInitOptions::new(EmbeddingModel::AllMiniLML6V2))
            .context("loading all-MiniLM-L6-v2 embedding model")?;
        Ok(EmbeddingEngine { model })
    }

    /// Embeds a batch of chunk texts (e.g. `DocumentChunk::text_content`
    /// values). Output order matches `texts`.
    pub fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.model
            .embed(texts, None)
            .context("embedding text batch")
    }

    /// Embeds a single query string (search bar / chat input).
    pub fn embed_query(&mut self, query: &str) -> Result<Vec<f32>> {
        let mut out = self
            .model
            .embed(&[query.to_string()], None)
            .context("embedding query")?;
        out.pop().context("embedding query returned no vector")
    }
}

/// Cosine similarity between two equal-length embedding vectors. Returns
/// `0.0` for a zero-length-norm vector (undefined direction) rather than
/// dividing by zero, so a degenerate embedding can't poison ranking with a
/// NaN.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(
        a.len(),
        b.len(),
        "cosine_similarity: vectors must be equal length"
    );
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

/// Serializes an embedding vector to little-endian `f32` bytes for the
/// SQLite `BLOB` cache column (`core::storage`'s `document_chunks` table,
/// §5). Paired with `bytes_to_embedding` for the read side.
pub fn embedding_to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Deserializes bytes produced by `embedding_to_bytes` back into a vector.
/// A trailing partial `f32` (fewer than 4 leftover bytes — shouldn't
/// happen for data written by this module, but a corrupt/foreign BLOB is
/// possible) is silently dropped rather than panicking.
pub fn bytes_to_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Ranks `candidates` against `query_embedding` by cosine similarity,
/// descending, returning at most the top `k` as `(candidate_index, score)`
/// — the "Top-K Chunks" retrieval step (§3.3 point 3).
pub fn top_k(query_embedding: &[f32], candidates: &[Vec<f32>], k: usize) -> Vec<(usize, f32)> {
    let mut scored: Vec<(usize, f32)> = candidates
        .iter()
        .enumerate()
        .map(|(i, v)| (i, cosine_similarity(query_embedding, v)))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.truncate(k);
    scored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_vectors_score_one() {
        let v = vec![0.1, 0.2, 0.3, 0.4];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn orthogonal_vectors_score_zero() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn opposite_vectors_score_negative_one() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        assert!((cosine_similarity(&a, &b) - -1.0).abs() < 1e-6);
    }

    #[test]
    fn zero_vector_scores_zero_instead_of_nan() {
        let zero = vec![0.0, 0.0, 0.0];
        let other = vec![1.0, 2.0, 3.0];
        assert_eq!(cosine_similarity(&zero, &other), 0.0);
    }

    #[test]
    fn top_k_orders_descending_and_truncates() {
        let query = vec![1.0, 0.0];
        let candidates = vec![
            vec![0.0, 1.0],  // orthogonal -> 0.0
            vec![1.0, 0.0],  // identical -> 1.0
            vec![0.7, 0.7],  // partial match -> ~0.707
            vec![-1.0, 0.0], // opposite -> -1.0
        ];

        let ranked = top_k(&query, &candidates, 2);

        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].0, 1); // the identical vector wins
        assert_eq!(ranked[1].0, 2); // then the partial match
        assert!(ranked[0].1 > ranked[1].1);
    }

    #[test]
    fn top_k_saturates_at_the_candidate_count() {
        let query = vec![1.0, 0.0];
        let candidates = vec![vec![1.0, 0.0]];
        assert_eq!(top_k(&query, &candidates, 5).len(), 1);
    }

    #[test]
    fn embedding_bytes_round_trip() {
        let v = vec![0.0, -1.5, 3.25, f32::MIN, f32::MAX, 1e-30];
        let bytes = embedding_to_bytes(&v);
        assert_eq!(bytes.len(), v.len() * 4);
        assert_eq!(bytes_to_embedding(&bytes), v);
    }

    #[test]
    fn bytes_to_embedding_drops_a_trailing_partial_f32() {
        let mut bytes = embedding_to_bytes(&[1.0, 2.0]);
        bytes.push(0xFF); // 9 bytes: 2 whole f32s + 1 stray byte
        assert_eq!(bytes_to_embedding(&bytes), vec![1.0, 2.0]);
    }
}

/// End-to-end verification against the real FastEmbed model (§3.3 point 3
/// acceptance: cosine similarity ranking over notes & documents). Ignored
/// by default — it downloads/loads the ~80 MB `all-MiniLM-L6-v2` weights
/// and needs network on first run; run explicitly with
/// `cargo test -- --ignored embedding_ranks_semantically_similar_text_higher`.
#[cfg(test)]
mod model_tests {
    use super::*;

    #[test]
    #[ignore]
    fn embedding_ranks_semantically_similar_text_higher() {
        let mut engine = EmbeddingEngine::new().expect("load all-MiniLM-L6-v2");

        let query = engine.embed_query("resep masakan nasi goreng").unwrap();
        let candidates = engine
            .embed(&[
                "Bahan-bahan nasi goreng: nasi putih, telur, kecap manis, bawang merah."
                    .to_string(),
                "Laporan keuangan kuartal ketiga menunjukkan penurunan pendapatan.".to_string(),
            ])
            .unwrap();

        let ranked = top_k(&query, &candidates, 2);
        assert_eq!(
            ranked[0].0, 0,
            "the recipe chunk should outrank the financial report chunk for a cooking query"
        );
        assert!(ranked[0].1 > ranked[1].1);
    }
}
