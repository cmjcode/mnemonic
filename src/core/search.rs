//! Pure search-ranking logic: fuses the semantic half (sqlite-vec cosine
//! KNN, `IndexStore::knn_chunks`) and the keyword half (FTS5/BM25,
//! `IndexStore::keyword_chunks`) into one ranked list with Reciprocal
//! Rank Fusion, and applies optional cross-encoder rerank scores. Kept
//! free of `egui`/IO so it's unit-testable — retrieval and query
//! embedding happen elsewhere (`core::storage`, `core::indexer`), this
//! module only consumes their output. Callers: `app`.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use super::ingestion::DocumentChunk;
use super::storage::KeywordChunk;
use crate::notes::Note;
pub use crate::notes::query::ParsedQuery;

/// RRF damping constant from the original paper (Cormack et al. 2009):
/// large enough that rank 1 vs 2 in one list doesn't dominate agreement
/// between lists.
const RRF_K: f32 = 60.0;

/// Which retrieval route(s) found a hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    Semantic,
    Keyword,
    Both,
}

/// One row of a unified search result list.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub chunk: DocumentChunk,
    /// Cosine similarity when the semantic route found this chunk.
    pub score: Option<f32>,
    pub kind: MatchKind,
    /// Keyword snippet; matched terms are wrapped in
    /// `storage::HIGHLIGHT_START`/`HIGHLIGHT_END`.
    pub snippet: Option<String>,
}

/// Case-insensitive substring match over note titles/bodies — an
/// in-memory keyword filter for callers without an index. Trashed notes
/// are always excluded; an empty query matches nothing.
pub fn keyword_search<'a>(notes: &'a [Note], query_text: &str) -> Vec<&'a Note> {
    let q = query_text.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    notes
        .iter()
        .filter(|n| !n.frontmatter.trashed)
        .filter(|n| {
            n.frontmatter.title.to_lowercase().contains(&q) || n.body.to_lowercase().contains(&q)
        })
        .collect()
}

/// Options for `hybrid_rank`.
#[derive(Debug, Clone, Copy)]
pub struct HybridOptions {
    /// Maximum hits returned.
    pub k: usize,
    /// Semantic-only hits below this cosine similarity are dropped: KNN
    /// always returns *something*, even for nonsense queries, so without
    /// a floor irrelevant chunks would fill the list.
    pub min_similarity: f32,
    /// Keep only the best chunk of each document (search results list)
    /// rather than several chunks (RAG context).
    pub one_per_doc: bool,
}

/// Fuses semantic and keyword candidates (each already best-first) with
/// Reciprocal Rank Fusion: `score = Σ 1 / (RRF_K + rank)` over the lists
/// a chunk appears in, so chunks both routes agree on rise to the top.
pub fn hybrid_rank(
    semantic: Vec<(DocumentChunk, f32)>,
    keyword: Vec<KeywordChunk>,
    opts: HybridOptions,
) -> Vec<SearchHit> {
    type Key = (Uuid, Option<usize>, usize);
    let key = |c: &DocumentChunk| -> Key { (c.doc_id, c.page_num, c.char_offset) };

    let mut fused: HashMap<Key, (f32, SearchHit)> = HashMap::new();
    for (rank, (chunk, similarity)) in semantic.into_iter().enumerate() {
        let rrf = 1.0 / (RRF_K + rank as f32 + 1.0);
        fused.insert(
            key(&chunk),
            (
                rrf,
                SearchHit {
                    chunk,
                    score: Some(similarity),
                    kind: MatchKind::Semantic,
                    snippet: None,
                },
            ),
        );
    }
    for (rank, kw) in keyword.into_iter().enumerate() {
        let rrf = 1.0 / (RRF_K + rank as f32 + 1.0);
        match fused.get_mut(&key(&kw.chunk)) {
            Some((score, hit)) => {
                *score += rrf;
                hit.kind = MatchKind::Both;
                hit.snippet = Some(kw.snippet);
            }
            None => {
                fused.insert(
                    key(&kw.chunk),
                    (
                        rrf,
                        SearchHit {
                            chunk: kw.chunk,
                            score: None,
                            kind: MatchKind::Keyword,
                            snippet: Some(kw.snippet),
                        },
                    ),
                );
            }
        }
    }

    let mut ranked: Vec<(f32, SearchHit)> = fused
        .into_values()
        .filter(|(_, hit)| {
            hit.kind != MatchKind::Semantic || hit.score.unwrap_or(0.0) >= opts.min_similarity
        })
        .collect();
    // Ties (e.g. rank 1 in different lists) fall back to similarity so
    // the order is deterministic.
    ranked.sort_by(|a, b| {
        b.0.total_cmp(&a.0).then_with(|| {
            b.1.score
                .unwrap_or(f32::MIN)
                .total_cmp(&a.1.score.unwrap_or(f32::MIN))
        })
    });

    let mut seen_docs = HashSet::new();
    ranked
        .into_iter()
        .map(|(_, hit)| hit)
        .filter(|hit| !opts.one_per_doc || seen_docs.insert(hit.chunk.doc_id))
        .take(opts.k)
        .collect()
}

/// Text sent to the cross-encoder for each hit.
pub fn rerank_documents(hits: &[SearchHit]) -> Vec<String> {
    hits.iter().map(|h| h.chunk.text_content.clone()).collect()
}

/// Reorders `hits` by cross-encoder `scores` (same order as `hits`),
/// best first; the fused order breaks ties. Mismatched lengths leave
/// `hits` untouched (a stale rerank result for a different hit list).
pub fn apply_rerank(hits: Vec<SearchHit>, scores: &[f32]) -> Vec<SearchHit> {
    if hits.len() != scores.len() {
        return hits;
    }
    let mut paired: Vec<(usize, f32, SearchHit)> = hits
        .into_iter()
        .enumerate()
        .map(|(i, h)| (i, scores[i], h))
        .collect();
    paired.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    paired.into_iter().map(|(_, _, h)| h).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn chunk(doc_id: Uuid, offset: usize, text: &str) -> DocumentChunk {
        DocumentChunk {
            doc_id,
            file_path: PathBuf::from("a.md"),
            page_num: None,
            char_offset: offset,
            text_content: text.to_string(),
        }
    }

    fn kw(chunk: DocumentChunk) -> KeywordChunk {
        KeywordChunk {
            snippet: format!("[{}]", chunk.text_content),
            chunk,
            bm25: -1.0,
        }
    }

    fn opts(k: usize, min_similarity: f32, one_per_doc: bool) -> HybridOptions {
        HybridOptions {
            k,
            min_similarity,
            one_per_doc,
        }
    }

    #[test]
    fn keyword_search_matches_title_or_body_case_insensitively() {
        let dir = tempdir().unwrap();
        let a = Note::create(dir.path(), "Belanja Mingguan", "beli susu").unwrap();
        let b = Note::create(dir.path(), "Catatan Lain", "isi lain").unwrap();

        let notes = vec![a, b];
        let result = keyword_search(&notes, "SUSU");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].frontmatter.title, "Belanja Mingguan");
    }

    #[test]
    fn keyword_search_excludes_trashed_notes() {
        let dir = tempdir().unwrap();
        let mut trashed = Note::create(dir.path(), "Sampah Kucing", "").unwrap();
        trashed.frontmatter.trashed = true;

        let notes = vec![trashed];
        assert!(keyword_search(&notes, "kucing").is_empty());
    }

    #[test]
    fn keyword_search_with_empty_query_matches_nothing() {
        let dir = tempdir().unwrap();
        let a = Note::create(dir.path(), "Judul", "isi").unwrap();
        assert!(keyword_search(&[a], "   ").is_empty());
    }

    #[test]
    fn chunks_found_by_both_routes_outrank_single_route_hits() {
        let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let semantic = vec![(chunk(a, 0, "a"), 0.95), (chunk(b, 0, "b"), 0.90)];
        let keyword = vec![kw(chunk(c, 0, "c")), kw(chunk(b, 0, "b"))];

        let hits = hybrid_rank(semantic, keyword, opts(10, 0.0, true));
        let order: Vec<Uuid> = hits.iter().map(|h| h.chunk.doc_id).collect();
        assert_eq!(order[0], b);
        assert_eq!(hits[0].kind, MatchKind::Both);
        assert_eq!(hits[0].score, Some(0.90));
        assert_eq!(hits[0].snippet.as_deref(), Some("[b]"));
        assert_eq!(hits.len(), 3);
        // Equal RRF (rank 1 in one list each): the semantic hit with a
        // similarity sorts before the score-less keyword hit.
        assert_eq!(order[1], a);
        assert_eq!(hits[2].kind, MatchKind::Keyword);
    }

    #[test]
    fn weak_semantic_only_hits_are_dropped_but_keyword_hits_stay() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let semantic = vec![(chunk(a, 0, "a"), 0.2), (chunk(b, 0, "b"), 0.2)];
        let keyword = vec![kw(chunk(b, 0, "b"))];
        let hits = hybrid_rank(semantic, keyword, opts(10, 0.5, true));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chunk.doc_id, b);
    }

    #[test]
    fn one_per_doc_keeps_the_best_chunk_only_when_requested() {
        let a = Uuid::new_v4();
        let semantic = vec![(chunk(a, 0, "a0"), 0.9), (chunk(a, 10, "a1"), 0.8)];
        assert_eq!(
            hybrid_rank(semantic.clone(), vec![], opts(10, 0.0, true)).len(),
            1
        );
        let all = hybrid_rank(semantic, vec![], opts(10, 0.0, false));
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].chunk.char_offset, 0);
    }

    #[test]
    fn hybrid_rank_respects_k() {
        let semantic: Vec<_> = (0..5)
            .map(|i| (chunk(Uuid::new_v4(), 0, "x"), 0.9 - i as f32 * 0.01))
            .collect();
        assert_eq!(hybrid_rank(semantic, vec![], opts(2, 0.0, true)).len(), 2);
    }

    #[test]
    fn apply_rerank_reorders_by_score_and_ignores_stale_results() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let hits = hybrid_rank(
            vec![(chunk(a, 0, "a"), 0.9), (chunk(b, 0, "b"), 0.8)],
            vec![],
            opts(10, 0.0, true),
        );
        assert_eq!(
            rerank_documents(&hits),
            vec!["a".to_string(), "b".to_string()]
        );
        let reranked = apply_rerank(hits.clone(), &[0.1, 5.0]);
        assert_eq!(reranked[0].chunk.doc_id, b);
        assert_eq!(apply_rerank(hits.clone(), &[1.0]), hits);
    }
}
