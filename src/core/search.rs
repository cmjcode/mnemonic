//! Pure search-ranking logic for the Search tab (§Fase 7): merges
//! substring keyword matches over notes with semantic (embedding cosine
//! similarity) matches over `core::storage::StoredChunk`s, so `app.rs`
//! can render one unified result list. Kept free of `egui`/IO so it's
//! independently unit-testable — the actual query embedding happens in
//! `core::indexer::IndexingWorker` (background thread, off the UI
//! thread), and this module only ever consumes its output (`Vec<f32>`)
//! plus already-loaded `StoredChunk`s/`Note`s. Callers: `app.rs`.

use std::collections::HashSet;

use uuid::Uuid;

use super::embedding::top_k;
use super::ingestion::DocumentChunk;
use super::storage::StoredChunk;
use crate::notes::{Note, query};

/// One row of a unified search result list. `score` is `None` for a pure
/// keyword match that has no semantic ranking to show (e.g. a note whose
/// text hasn't been indexed/embedded yet, or when semantic retrieval
/// hasn't resolved).
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub chunk: DocumentChunk,
    pub score: Option<f32>,
}

/// Case-insensitive substring match over note titles/bodies — the
/// "keyword" half of §Fase 7's combined search. Trashed notes are always
/// excluded; archived notes are still searchable. An empty (or
/// whitespace-only) query matches nothing rather than the whole vault.
pub fn keyword_search<'a>(notes: &'a [Note], query_text: &str) -> Vec<&'a Note> {
    let q = query_text.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    notes
        .iter()
        .filter(|n| !n.frontmatter.trashed)
        .filter(|n| n.frontmatter.title.to_lowercase().contains(&q) || n.body.to_lowercase().contains(&q))
        .collect()
}

/// Ranks `chunks` against `query_embedding` by cosine similarity (via
/// `core::embedding::top_k`), keeping only scores at or above `threshold`
/// — the "semantic" half of §Fase 7's combined search. Callers typically
/// pass `llm::SIMILARITY_THRESHOLD` so search and RAG retrieval agree on
/// what counts as "relevant" (§6 risk 3).
pub fn semantic_search(
    query_embedding: &[f32],
    chunks: &[StoredChunk],
    k: usize,
    threshold: f32,
) -> Vec<SearchHit> {
    let vectors: Vec<Vec<f32>> = chunks.iter().map(|c| c.embedding.clone()).collect();
    top_k(query_embedding, &vectors, k)
        .into_iter()
        .filter(|(_, score)| *score >= threshold)
        .map(|(idx, score)| SearchHit {
            chunk: chunks[idx].chunk.clone(),
            score: Some(score),
        })
        .collect()
}

/// Merges semantic chunk hits and keyword note hits into one display
/// list: semantic hits first (already relevance-sorted), then any
/// keyword-matched note whose id isn't already represented by a semantic
/// hit — so a note found by both routes doesn't show twice. Keyword-only
/// notes are rendered as a synthetic hit (a short snippet of the note
/// body, `score: None`) since they have no `StoredChunk` scoring behind
/// them.
pub fn merge_results(semantic: Vec<SearchHit>, keyword_notes: &[&Note]) -> Vec<SearchHit> {
    let mut seen: HashSet<Uuid> = semantic.iter().map(|h| h.chunk.doc_id).collect();
    let mut out = semantic;
    for note in keyword_notes {
        if seen.insert(note.frontmatter.id) {
            out.push(SearchHit {
                chunk: DocumentChunk {
                    doc_id: note.frontmatter.id,
                    file_path: note.path.clone(),
                    page_num: None,
                    char_offset: 0,
                    text_content: query::snippet(&note.body, 200),
                },
                score: None,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn stored_chunk(doc_id: Uuid, file: &str, text: &str, embedding: Vec<f32>) -> StoredChunk {
        StoredChunk {
            chunk: DocumentChunk {
                doc_id,
                file_path: PathBuf::from(file),
                page_num: None,
                char_offset: 0,
                text_content: text.to_string(),
            },
            embedding,
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
    fn semantic_search_ranks_by_similarity_and_drops_below_threshold() {
        let doc_a = Uuid::new_v4();
        let doc_b = Uuid::new_v4();
        let chunks = vec![
            stored_chunk(doc_a, "a.md", "cocok kuat", vec![1.0, 0.0]),
            stored_chunk(doc_b, "b.md", "tidak relevan", vec![0.0, 1.0]),
        ];

        let hits = semantic_search(&[1.0, 0.0], &chunks, 5, 0.5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chunk.doc_id, doc_a);
        assert_eq!(hits[0].score, Some(1.0));
    }

    #[test]
    fn semantic_search_respects_k() {
        let chunks = vec![
            stored_chunk(Uuid::new_v4(), "a.md", "a", vec![1.0, 0.0]),
            stored_chunk(Uuid::new_v4(), "b.md", "b", vec![0.9, 0.1]),
            stored_chunk(Uuid::new_v4(), "c.md", "c", vec![0.8, 0.2]),
        ];
        let hits = semantic_search(&[1.0, 0.0], &chunks, 2, 0.0);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn merge_results_deduplicates_notes_already_found_semantically() {
        let dir = tempdir().unwrap();
        let note = Note::create(dir.path(), "Ganda", "isi ganda").unwrap();
        let semantic = vec![SearchHit {
            chunk: DocumentChunk {
                doc_id: note.frontmatter.id,
                file_path: note.path.clone(),
                page_num: None,
                char_offset: 0,
                text_content: "isi ganda".to_string(),
            },
            score: Some(0.9),
        }];

        let merged = merge_results(semantic, &[&note]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].score, Some(0.9));
    }

    #[test]
    fn merge_results_appends_keyword_only_notes_with_no_score() {
        let dir = tempdir().unwrap();
        let note = Note::create(dir.path(), "Hanya Kata Kunci", "isinya").unwrap();

        let merged = merge_results(Vec::new(), &[&note]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].chunk.doc_id, note.frontmatter.id);
        assert_eq!(merged[0].score, None);
    }
}
