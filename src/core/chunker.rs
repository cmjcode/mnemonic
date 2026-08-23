//! Sliding-window text chunker (§3.3 point 2): splits a document's plain
//! text into overlapping windows sized for embedding. "Token" here means a
//! whitespace-delimited word — a simple proxy good enough for chunk sizing
//! without pulling in a full tokenizer just for this step (the embedding
//! model's own tokenizer, in `core::embedding`, handles the real
//! tokenization at inference time). Callers: `core::ingestion`.

/// One windowed slice of source text, with the byte offset (spec's
/// `char_offset`) into the original string where it starts.
#[derive(Debug, Clone, PartialEq)]
pub struct TextChunk {
    pub char_offset: usize,
    pub text: String,
}

/// Default window size from §3.3 point 2: 300 words per chunk.
pub const DEFAULT_CHUNK_TOKENS: usize = 300;
/// Default overlap from §3.3 point 2: 50 words shared between consecutive
/// chunks, so a sentence spanning a window boundary still lands whole in
/// at least one chunk.
pub const DEFAULT_OVERLAP_TOKENS: usize = 50;

/// Splits `text` into overlapping chunks of `chunk_tokens` words, advancing
/// `chunk_tokens - overlap_tokens` words per step. Blank/empty input yields
/// no chunks; text shorter than one window yields exactly one chunk holding
/// all of it.
///
/// # Panics
/// If `overlap_tokens >= chunk_tokens` — the window would never advance.
pub fn chunk_text(text: &str, chunk_tokens: usize, overlap_tokens: usize) -> Vec<TextChunk> {
    assert!(
        overlap_tokens < chunk_tokens,
        "chunk_text: overlap_tokens ({overlap_tokens}) must be < chunk_tokens ({chunk_tokens})"
    );

    let words = word_offsets(text);
    if words.is_empty() {
        return Vec::new();
    }

    let step = chunk_tokens - overlap_tokens;
    let mut chunks = Vec::new();
    let mut start = 0usize;
    loop {
        let end = (start + chunk_tokens).min(words.len());
        let byte_start = words[start].0;
        let byte_end = words[end - 1].1;
        chunks.push(TextChunk {
            char_offset: byte_start,
            text: text[byte_start..byte_end].to_string(),
        });
        if end == words.len() {
            break;
        }
        start += step;
    }
    chunks
}

/// (byte_start, byte_end) for each whitespace-delimited word in `text`, in
/// order.
fn word_offsets(text: &str) -> Vec<(usize, usize)> {
    let mut offsets = Vec::new();
    let mut word_start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(start) = word_start.take() {
                offsets.push((start, i));
            }
        } else if word_start.is_none() {
            word_start = Some(i);
        }
    }
    if let Some(start) = word_start {
        offsets.push((start, text.len()));
    }
    offsets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_yields_no_chunks() {
        assert_eq!(chunk_text("", 300, 50), Vec::new());
        assert_eq!(chunk_text("   \n\t  ", 300, 50), Vec::new());
    }

    #[test]
    fn text_shorter_than_one_window_yields_a_single_chunk() {
        let chunks = chunk_text("satu dua tiga", 300, 50);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].char_offset, 0);
        assert_eq!(chunks[0].text, "satu dua tiga");
    }

    #[test]
    fn sliding_window_overlaps_by_the_configured_word_count() {
        let text = "one two three four five six seven";
        let chunks = chunk_text(text, 4, 1);

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].text, "one two three four");
        assert_eq!(chunks[1].text, "four five six seven");
        // "four" (word index 3) is the shared overlap word.
        assert_eq!(chunks[0].char_offset, 0);
        assert_eq!(chunks[1].char_offset, text.find("four").unwrap());
    }

    #[test]
    fn exact_multiple_of_step_does_not_duplicate_a_trailing_chunk() {
        // 6 words, chunk=3, overlap=0 -> step=3 -> exactly two chunks, no
        // empty/duplicate third chunk once the window reaches the end.
        let chunks = chunk_text("a b c d e f", 3, 0);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].text, "a b c");
        assert_eq!(chunks[1].text, "d e f");
    }

    #[test]
    fn char_offset_accounts_for_multibyte_characters() {
        // "café" is 5 bytes (é is 2 bytes UTF-8); the second word must
        // start at its correct byte offset, not its char count.
        let text = "café répertoire";
        let chunks = chunk_text(text, 1, 0);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[1].char_offset, "café ".len());
        assert_eq!(chunks[1].text, "répertoire");
    }

    #[test]
    #[should_panic(expected = "overlap_tokens")]
    fn overlap_not_smaller_than_chunk_size_panics() {
        chunk_text("a b c", 4, 4);
    }
}
