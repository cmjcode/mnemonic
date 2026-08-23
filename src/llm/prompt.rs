//! RAG prompt template builder (§3.4 point 2): assembles the ChatML-style
//! prompt Qwen2.5-Instruct expects from a strict-grounding system
//! preamble, the retrieved context chunks, and the user's question. Pure
//! string logic — no model/tokenizer dependency — so it's independently
//! unit-testable from `llm::candle_engine`'s inference code. Also holds
//! the similarity-threshold cutoff (§6 risk 3: "LLM tidak menjawab jika
//! tidak ada dokumen yang cocok") that turns `core::embedding::top_k`
//! scores into the chunk list this module renders.
//! Callers: `llm::candle_engine` (its `Generator::generate` consumes the
//! built prompt), future chat UI (§Fase 7) which drives retrieval via
//! `core::embedding::top_k` over `core::storage::IndexStore::all_chunks`.

use crate::core::DocumentChunk;

/// Strict-grounding system instruction — verbatim per §3.4 point 2's
/// template, so the model is told not to hallucinate past the supplied
/// context.
const SYSTEM_PREAMBLE: &str = "Anda adalah asisten cerdas. Jawablah pertanyaan pengguna HANYA berdasarkan konteks dokumen berikut. Jika informasi tidak ada di dokumen, katakan tidak tahu.";

/// Shown in place of any context blocks when retrieval found nothing
/// above the similarity threshold — keeps the "say you don't know"
/// instruction honest instead of silently omitting the section.
const NO_CONTEXT_NOTE: &str = "(Tidak ditemukan dokumen yang relevan di vault.)";

/// Minimum cosine similarity (§3.3 point 3 scoring) a chunk must clear to
/// be considered relevant enough to ground an answer on. Chunks below
/// this are dropped by `select_context` before they ever reach the
/// prompt, rather than trusting the LLM alone to notice weak matches.
pub const SIMILARITY_THRESHOLD: f32 = 0.35;

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
/// 2 — page number omitted for notes (`page_num: None`).
fn render_chunk_block(chunk: &DocumentChunk) -> String {
    let location = match chunk.page_num {
        Some(page) => format!("{} (Halaman {})", chunk.file_path.display(), page),
        None => chunk.file_path.display().to_string(),
    };
    format!("---\nFile: {location}\n{}\n---", chunk.text_content)
}

/// Filters `core::embedding::top_k`-scored candidates down to the ones
/// worth grounding an answer on (score >= `threshold`), resolving each
/// `(candidate_index, score)` back to its `DocumentChunk` via
/// `all_chunks[candidate_index]`. Order is preserved — `top_k` already
/// sorts descending by score.
pub fn select_context(
    scored: &[(usize, f32)],
    all_chunks: &[DocumentChunk],
    threshold: f32,
) -> Vec<DocumentChunk> {
    scored
        .iter()
        .filter(|(_, score)| *score >= threshold)
        .filter_map(|(idx, _)| all_chunks.get(*idx).cloned())
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

    #[test]
    fn select_context_drops_scores_below_threshold() {
        let chunks = vec![chunk("a.md", None, "a"), chunk("b.md", None, "b")];
        let scored = vec![(0, 0.9), (1, 0.1)];
        let selected = select_context(&scored, &chunks, 0.35);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].text_content, "a");
    }

    #[test]
    fn select_context_preserves_score_order() {
        let chunks = vec![chunk("a.md", None, "a"), chunk("b.md", None, "b")];
        let scored = vec![(1, 0.8), (0, 0.5)]; // b scored higher than a
        let selected = select_context(&scored, &chunks, 0.0);
        assert_eq!(selected[0].text_content, "b");
        assert_eq!(selected[1].text_content, "a");
    }

    #[test]
    fn select_context_with_all_below_threshold_yields_empty_vec() {
        let chunks = vec![chunk("a.md", None, "a")];
        let scored = vec![(0, 0.1)];
        assert!(select_context(&scored, &chunks, 0.35).is_empty());
    }
}
