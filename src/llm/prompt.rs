//! RAG prompt template builder (§3.4 point 2): assembles the ChatML-style
//! prompt Qwen2.5-Instruct expects from a strict-grounding system
//! preamble, the retrieved context chunks, and the user's question. Pure
//! string logic — no model/tokenizer dependency — so it's independently
//! unit-testable from `llm::candle_engine`'s inference code. Also holds
//! the similarity-threshold cutoff (§6 risk 3: "LLM tidak menjawab jika
//! tidak ada dokumen yang cocok") that turns hybrid search hits
//! into the chunk list this module renders.
//! Callers: `llm::candle_engine` (its `Generator::generate` consumes the
//! built prompt) and the chat panel in `app`, which retrieves context via
//! hybrid search (`core::search::hybrid_rank`).

use crate::core::{DocumentChunk, MatchKind, SearchHit};

/// Strict-grounding system instruction — verbatim per §3.4 point 2's
/// template, so the model is told not to hallucinate past the supplied
/// context.
const SYSTEM_PREAMBLE: &str = "Anda adalah asisten cerdas. Jawablah pertanyaan pengguna HANYA berdasarkan konteks dokumen berikut. Jika informasi tidak ada di dokumen, katakan tidak tahu.";

/// Shown in place of any context blocks when retrieval found nothing
/// above the similarity threshold — keeps the "say you don't know"
/// instruction honest instead of silently omitting the section.
const NO_CONTEXT_NOTE: &str = "(Tidak ditemukan dokumen yang relevan di vault.)";

/// Minimum cosine similarity (§3.3 point 3 scoring) a semantic-only chunk
/// must clear to count as relevant — for grounding an answer
/// (`select_context`) and for search results alike. Calibrated for
/// `multilingual-e5-small`, whose similarities cluster high: measured
/// query→passage pairs scored ≥ 0.868 when relevant and ≤ 0.812 when not
/// (`core::embedding::model_tests`), so the cutoff sits in between.
pub const SIMILARITY_THRESHOLD: f32 = 0.84;

/// Builds the full ChatML prompt: system preamble + rendered context
/// chunks, then the user's turn, ending right where the assistant's
/// reply should begin (§3.4 point 2's template).
pub fn build_rag_prompt(context_chunks: &[DocumentChunk], user_query: &str) -> String {
    let context = if context_chunks.is_empty() {
        NO_CONTEXT_NOTE.to_string()
    } else {
        context_chunks
            .iter()
            .map(render_chunk_block)
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "<|im_start|>system\n{SYSTEM_PREAMBLE}\n[KONTEKS DOKUMEN]\n{context}\n<|im_end|>\n\
         <|im_start|>user\n{user_query}\n<|im_end|>\n\
         <|im_start|>assistant\n"
    )
}

/// Renders one chunk as the `File: ... (Halaman N)` block from §3.4 point
/// 2 — page number omitted for notes (`page_num: None`); sheets (§3.8.4)
/// cite their first row instead (`Baris N`).
fn render_chunk_block(chunk: &DocumentChunk) -> String {
    let location = match chunk.page_num {
        Some(row) if crate::sheet::is_sheet_path(&chunk.file_path) => {
            format!("{} (Baris {})", chunk.file_path.display(), row)
        }
        Some(page) => format!("{} (Halaman {})", chunk.file_path.display(), page),
        None => chunk.file_path.display().to_string(),
    };
    format!("---\nFile: {location}\n{}\n---", chunk.text_content)
}

/// Picks the chunks worth grounding an answer on from hybrid-ranked hits
/// (`core::search::hybrid_rank`): keyword matches always qualify, semantic-
/// only matches need a similarity of at least `threshold`. Order is
/// preserved (already best-first).
pub fn select_context(hits: &[SearchHit], threshold: f32) -> Vec<DocumentChunk> {
    hits.iter()
        .filter(|h| h.kind != MatchKind::Semantic || h.score.unwrap_or(0.0) >= threshold)
        .map(|h| h.chunk.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn chunk(file: &str, page: Option<usize>, text: &str) -> DocumentChunk {
        DocumentChunk {
            doc_id: Uuid::new_v4(),
            file_path: PathBuf::from(file),
            page_num: page,
            char_offset: 0,
            text_content: text.to_string(),
        }
    }

    #[test]
    fn prompt_includes_system_preamble_and_chatml_markers() {
        let prompt = build_rag_prompt(&[], "halo?");
        assert!(prompt.starts_with("<|im_start|>system\n"));
        assert!(prompt.contains(SYSTEM_PREAMBLE));
        assert!(prompt.contains("<|im_start|>user\nhalo?\n<|im_end|>"));
        assert!(prompt.ends_with("<|im_start|>assistant\n"));
    }

    #[test]
    fn empty_context_shows_no_relevant_docs_note() {
        let prompt = build_rag_prompt(&[], "pertanyaan");
        assert!(prompt.contains(NO_CONTEXT_NOTE));
    }

    #[test]
    fn pdf_chunk_renders_with_page_number() {
        let c = chunk("laporan.pdf", Some(4), "isi laporan");
        let prompt = build_rag_prompt(&[c], "apa isi laporan?");
        assert!(prompt.contains("File: laporan.pdf (Halaman 4)"));
        assert!(prompt.contains("isi laporan"));
    }

    #[test]
    fn note_chunk_renders_without_page_number() {
        let c = chunk("belanja.md", None, "beli susu");
        let prompt = build_rag_prompt(&[c], "belanja apa?");
        assert!(prompt.contains("File: belanja.md\n"));
        assert!(!prompt.contains("Halaman"));
    }

    #[test]
    fn multiple_chunks_each_get_their_own_block() {
        let a = chunk("a.md", None, "konten a");
        let b = chunk("b.pdf", Some(2), "konten b");
        let prompt = build_rag_prompt(&[a, b], "q");
        assert!(prompt.contains("konten a"));
        assert!(prompt.contains("konten b"));
        assert!(prompt.contains("File: a.md"));
        assert!(prompt.contains("File: b.pdf (Halaman 2)"));
    }

    fn hit(text: &str, score: Option<f32>, kind: MatchKind) -> SearchHit {
        SearchHit {
            chunk: chunk("a.md", None, text),
            score,
            kind,
            snippet: None,
        }
    }

    #[test]
    fn select_context_drops_weak_semantic_only_hits() {
        let hits = vec![
            hit("a", Some(0.9), MatchKind::Semantic),
            hit("b", Some(0.1), MatchKind::Semantic),
        ];
        let selected = select_context(&hits, 0.35);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].text_content, "a");
    }

    #[test]
    fn select_context_keeps_keyword_matches_regardless_of_similarity() {
        let hits = vec![
            hit("kw", None, MatchKind::Keyword),
            hit("both", Some(0.1), MatchKind::Both),
        ];
        assert_eq!(select_context(&hits, 0.9).len(), 2);
    }

    #[test]
    fn select_context_preserves_order() {
        let hits = vec![
            hit("b", Some(0.8), MatchKind::Semantic),
            hit("a", Some(0.5), MatchKind::Semantic),
        ];
        let selected = select_context(&hits, 0.0);
        assert_eq!(selected[0].text_content, "b");
        assert_eq!(selected[1].text_content, "a");
    }
}
