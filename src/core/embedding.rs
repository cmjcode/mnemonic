//! Local semantic embedding via FastEmbed (§3.3 point 3): wraps
//! `fastembed::TextEmbedding` with `multilingual-e5-small` (384-D, trained
//! on ~100 languages including Indonesian — the previous English-only
//! `all-MiniLM-L6-v2` ranked Indonesian notes poorly) and a local
//! cross-encoder reranker, plus the cosine-similarity helpers. Model
//! weights download to FastEmbed's on-disk cache the first time a model
//! is constructed — that network/disk hit is isolated to construction, so
//! `cosine_similarity`/`top_k` stay pure and testable offline. Callers:
//! `core::indexer` (embedding chunks, queries, reranking).

use anyhow::{Context, Result};
use fastembed::{
    EmbeddingModel, RerankInitOptions, RerankerModel, TextEmbedding, TextInitOptions, TextRerank,
};

/// Identifies the embedding model in the index cache
/// (`IndexStore::ensure_embedding_model`): changing the model changes this
/// id, which invalidates every stored vector.
pub const EMBEDDING_MODEL_ID: &str = "multilingual-e5-small";
/// Vector size produced by `EMBEDDING_MODEL_ID`.
pub const EMBEDDING_DIM: usize = 384;
/// Minimum cosine similarity between two documents' mean embeddings for
/// them to count as related ("Related notes", graph AI edges). Passage ↔
/// passage similarities run higher than query ↔ passage ones (measured:
/// ≥ 0.923 for related pairs, up to 0.894 for unrelated ones), so this is
/// stricter than `llm::SIMILARITY_THRESHOLD`; checked by
/// `model_tests::similarity_threshold_separates_relevant_from_unrelated`.
pub const RELATED_DOC_SIMILARITY: f32 = 0.91;

/// E5 models are trained with asymmetric prefixes: stored text is a
/// "passage", search input a "query". Omitting them measurably hurts
/// retrieval quality.
const PASSAGE_PREFIX: &str = "passage: ";
const QUERY_PREFIX: &str = "query: ";

/// A loaded FastEmbed text embedding model. Construction is the only
/// network/IO-touching part of this module.
pub struct EmbeddingEngine {
    model: TextEmbedding,
}

impl EmbeddingEngine {
    /// Loads `multilingual-e5-small` — downloads it into the FastEmbed
    /// cache directory on first run, then loads from there on subsequent
    /// runs (fully offline once cached).
    pub fn new() -> Result<EmbeddingEngine> {
        let model =
            TextEmbedding::try_new(TextInitOptions::new(EmbeddingModel::MultilingualE5Small))
                .context("loading multilingual-e5-small embedding model")?;
        Ok(EmbeddingEngine { model })
    }

    /// Embeds a batch of document texts (chunks). Output order matches
    /// `texts`.
    pub fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let prefixed: Vec<String> = texts
            .iter()
            .map(|t| format!("{PASSAGE_PREFIX}{t}"))
            .collect();
        self.model
            .embed(&prefixed, None)
            .context("embedding text batch")
    }

    /// Embeds a single query string (search bar / chat input).
    pub fn embed_query(&mut self, query: &str) -> Result<Vec<f32>> {
        let mut out = self
            .model
            .embed(&[format!("{QUERY_PREFIX}{query}")], None)
            .context("embedding query")?;
        out.pop().context("embedding query returned no vector")
    }
}

/// A local cross-encoder that scores (query, passage) pairs jointly —
/// slower than vector similarity but noticeably more precise, so it only
/// reorders the top hybrid-search candidates. Optional (off by default).
pub struct RerankEngine {
    model: TextRerank,
}

impl RerankEngine {
    /// Loads `jina-reranker-v2-base-multilingual` (downloads on first use).
    pub fn new() -> Result<RerankEngine> {
        let model = TextRerank::try_new(RerankInitOptions::new(
            RerankerModel::JINARerankerV2BaseMultiligual,
        ))
        .context("loading jina-reranker-v2-base-multilingual model")?;
        Ok(RerankEngine { model })
    }

    /// Relevance score for each document, in `documents` order (higher is
    /// more relevant; scores are only comparable within one call).
    pub fn rerank(&mut self, query: &str, documents: &[String]) -> Result<Vec<f32>> {
        let docs: Vec<&str> = documents.iter().map(String::as_str).collect();
        let results = self
            .model
            .rerank(query, docs, false, None)
            .context("reranking search candidates")?;
        let mut scores = vec![f32::MIN; documents.len()];
        for r in results {
            if let Some(slot) = scores.get_mut(r.index) {
                *slot = r.score;
            }
        }
        Ok(scores)
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
/// by default — it downloads/loads the multilingual-e5-small weights and
/// needs network on first run; run explicitly with
/// `cargo test -- --ignored embedding_ranks_semantically_similar_text_higher`.
#[cfg(test)]
mod model_tests {
    use super::*;

    #[test]
    #[ignore]
    fn embedding_ranks_semantically_similar_text_higher() {
        let mut engine = EmbeddingEngine::new().expect("load multilingual-e5-small");

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

    /// Checks that `llm::SIMILARITY_THRESHOLD` separates relevant from
    /// unrelated passages for the real model; prints the similarities so
    /// the threshold can be re-tuned when the model changes. Run with
    /// `cargo test -- --ignored similarity_threshold_separates -- --nocapture`.
    #[test]
    #[ignore]
    fn similarity_threshold_separates_relevant_from_unrelated() {
        let threshold = crate::llm::SIMILARITY_THRESHOLD;
        let mut engine = EmbeddingEngine::new().expect("load multilingual-e5-small");
        let cases: &[(&str, &str, bool)] = &[
            ("resep nasi goreng", "Bahan: nasi putih, telur, kecap manis, bawang merah. Tumis bawang lalu masukkan nasi.", true),
            ("cara membuat nasi goreng", "Resep Nasi Goreng\nPanaskan minyak, tumis bumbu, masukkan nasi dan kecap.", true),
            ("laporan keuangan kuartal tiga", "Pendapatan Q3 turun 12% dibanding kuartal sebelumnya karena biaya operasional naik.", true),
            ("meeting dengan klien", "Notulen rapat bersama klien PT Maju: pembahasan timeline proyek dan anggaran.", true),
            ("how to deploy the server", "Langkah deploy: build image docker, push ke registry, lalu restart service di server produksi.", true),
            ("resep nasi goreng", "Pendapatan Q3 turun 12% dibanding kuartal sebelumnya karena biaya operasional naik.", false),
            ("laporan keuangan kuartal tiga", "Bahan: nasi putih, telur, kecap manis, bawang merah.", false),
            ("meeting dengan klien", "Daftar belanja mingguan: susu, roti, sabun cuci, dan buah apel.", false),
            ("how to deploy the server", "Puisi tentang hujan di sore hari yang sunyi.", false),
            ("jadwal olahraga", "Kode sumber fungsi parser JSON dalam bahasa Rust.", false),
        ];
        let mut min_relevant = f32::MAX;
        let mut max_unrelated = f32::MIN;
        for (query, passage, relevant) in cases {
            let q = engine.embed_query(query).unwrap();
            let p = engine.embed(&[passage.to_string()]).unwrap().pop().unwrap();
            let sim = cosine_similarity(&q, &p);
            println!("{sim:.3} relevant={relevant} {query:?} vs {passage:?}");
            if *relevant {
                min_relevant = min_relevant.min(sim);
            } else {
                max_unrelated = max_unrelated.max(sim);
            }
        }
        println!("min relevant {min_relevant:.3}, max unrelated {max_unrelated:.3}, threshold {threshold}");
        assert!(max_unrelated < threshold, "unrelated passages must fall below the threshold");
        assert!(min_relevant >= threshold, "relevant passages must clear the threshold");

        // Passage ↔ passage (document similarity for "Related notes" and
        // graph AI edges) has its own distribution.
        let docs: &[(&str, &str, bool)] = &[
            ("Resep Nasi Goreng\nTumis bawang, masukkan nasi dan kecap manis.", "Resep Mie Goreng\nRebus mie, tumis bumbu, campur dengan kecap.", true),
            ("Rapat proyek MNEMONIC: bahas fitur pencarian dan graph.", "Notulen meeting sprint: fitur search AI dan tampilan graf relasi.", true),
            ("Laporan keuangan Q3: pendapatan turun, biaya naik.", "Anggaran tahun depan: target pendapatan dan efisiensi biaya.", true),
            ("Resep Nasi Goreng\nTumis bawang, masukkan nasi dan kecap manis.", "Laporan keuangan Q3: pendapatan turun, biaya naik.", false),
            ("Rapat proyek MNEMONIC: bahas fitur pencarian dan graph.", "Puisi tentang hujan di sore hari yang sunyi.", false),
            ("Daftar belanja: susu, roti, sabun.", "Catatan belajar Rust: ownership dan borrowing.", false),
        ];
        let related = RELATED_DOC_SIMILARITY;
        let (mut min_rel, mut max_unrel) = (f32::MAX, f32::MIN);
        for (a, b, relevant) in docs {
            let v = engine.embed(&[a.to_string(), b.to_string()]).unwrap();
            let sim = cosine_similarity(&v[0], &v[1]);
            println!("{sim:.3} doc relevant={relevant} {a:?} vs {b:?}");
            if *relevant {
                min_rel = min_rel.min(sim);
            } else {
                max_unrel = max_unrel.max(sim);
            }
        }
        println!("docs: min relevant {min_rel:.3}, max unrelated {max_unrel:.3}, related threshold {related}");
        assert!(max_unrel < related && min_rel >= related);
    }
}
