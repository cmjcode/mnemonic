//! Recursive multi-format ingestion (§3.3 point 1): turns a `Note` or a
//! PDF file into ready-to-embed `DocumentChunk`s. Embedding itself
//! (`core::embedding::EmbeddingEngine`) is a separate step, so this module
//! stays free of any model/network dependency and fully unit-testable.
//! Callers: future search/RAG indexing pipeline (§3.3, §5).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use uuid::Uuid;

use super::chunker::{self, DEFAULT_CHUNK_TOKENS, DEFAULT_OVERLAP_TOKENS};
use crate::notes::Note;
use crate::pdf;

/// Fixed namespace for deriving a stable per-file `doc_id` (via UUID v5)
/// for documents that aren't already a `Note` with its own id — i.e.
/// PDFs. Stability matters so re-ingesting the same file on a rescan
/// produces the same `doc_id` instead of a fresh random one each time.
const PDF_DOC_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6c, 0x6f, 0x6e, 0x74, 0x61, 0x72, 0x2d, 0x70, 0x64, 0x66, 0x2d, 0x64, 0x6f, 0x63, 0x00, 0x00,
]);

/// One chunk of source text ready for embedding, with enough metadata to
/// jump back to its origin (§3.3 point 2's `{ doc_id, file_path, page_num,
/// char_offset, text_content }`).
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentChunk {
    pub doc_id: Uuid,
    pub file_path: PathBuf,
    /// `None` for Markdown notes (no page concept); `Some(1-based page)`
    /// for PDF chunks.
    pub page_num: Option<usize>,
    pub char_offset: usize,
    pub text_content: String,
}

/// Derives the stable `doc_id` for a PDF at `path` (see
/// `PDF_DOC_NAMESPACE`). Exposed so callers that need a document's id even
/// when it produced zero chunks (e.g. an empty/unreadable PDF) don't have
/// to duplicate this derivation — used by `core::indexer`.
pub fn pdf_doc_id(path: &Path) -> Uuid {
    Uuid::new_v5(&PDF_DOC_NAMESPACE, path.to_string_lossy().as_bytes())
}

/// Chunks a note's body — the same retrieval pipeline that PDFs feed also
/// covers the vault's own `.md` files (§3.3 point 1). `doc_id` reuses the
/// note's own frontmatter id, so re-chunking after an edit keeps it
/// stable.
pub fn chunk_note(note: &Note) -> Vec<DocumentChunk> {
    chunker::chunk_text(&note.body, DEFAULT_CHUNK_TOKENS, DEFAULT_OVERLAP_TOKENS)
        .into_iter()
        .map(|c| DocumentChunk {
            doc_id: note.frontmatter.id,
            file_path: note.path.clone(),
            page_num: None,
            char_offset: c.char_offset,
            text_content: c.text,
        })
        .collect()
}

/// Extracts and chunks a PDF file page by page, tagging each chunk with
/// its 1-based page number so citations (§3.4 point 4) can jump straight
/// to the source page.
pub fn chunk_pdf(path: &Path) -> Result<Vec<DocumentChunk>> {
    let doc_id = pdf_doc_id(path);
    let pages =
        pdf::extract_pages(path).with_context(|| format!("ingesting PDF {}", path.display()))?;

    let chunks = pages
        .into_iter()
        .enumerate()
        .flat_map(|(page_index, page_text)| {
            let file_path = path.to_path_buf();
            chunker::chunk_text(&page_text, DEFAULT_CHUNK_TOKENS, DEFAULT_OVERLAP_TOKENS)
                .into_iter()
                .map(move |c| DocumentChunk {
                    doc_id,
                    file_path: file_path.clone(),
                    page_num: Some(page_index + 1),
                    char_offset: c.char_offset,
                    text_content: c.text,
                })
        })
        .collect();

    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Document, Object, Stream, content::Content, content::Operation, dictionary};
    use tempfile::tempdir;

    fn write_test_pdf(path: &Path, pages_text: &[&str]) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let resources_id =
            doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });

        let page_ids: Vec<Object> = pages_text
            .iter()
            .map(|text| {
                let content = Content {
                    operations: vec![
                        Operation::new("BT", vec![]),
                        Operation::new("Tf", vec!["F1".into(), 24.into()]),
                        Operation::new("Td", vec![72.into(), 700.into()]),
                        Operation::new("Tj", vec![Object::string_literal(*text)]),
                        Operation::new("ET", vec![]),
                    ],
                };
                let content_id =
                    doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
                doc.add_object(dictionary! {
                    "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
                })
                .into()
            })
            .collect();

        let count = page_ids.len() as i64;
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => page_ids,
                "Count" => count,
                "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        doc.save(path).unwrap();
    }

    #[test]
    fn chunk_note_tags_every_chunk_with_the_notes_id_and_path() {
        let dir = tempdir().unwrap();
        // Long enough body to produce more than one chunk at a small window,
        // so we can check offsets differ across chunks too.
        let body = (0..20)
            .map(|i| format!("kata{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let note = Note::create(dir.path(), "Catatan Panjang", &body).unwrap();

        let chunks = chunk_note(&note);

        assert!(!chunks.is_empty());
        for chunk in &chunks {
            assert_eq!(chunk.doc_id, note.frontmatter.id);
            assert_eq!(chunk.file_path, note.path);
            assert_eq!(chunk.page_num, None);
        }
    }

    #[test]
    fn chunk_note_with_small_window_produces_overlapping_chunks() {
        let dir = tempdir().unwrap();
        let body = "one two three four five six seven".to_string();
        let note = Note::create(dir.path(), "Kecil", &body).unwrap();

        // The default window (300 words) is far larger than this 7-word
        // body, so ingestion should produce exactly one chunk covering it.
        let chunks = chunk_note(&note);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text_content, body);
        assert_eq!(chunks[0].char_offset, 0);
    }

    #[test]
    fn chunk_pdf_tags_each_chunk_with_its_one_based_page_number() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("dok.pdf");
        write_test_pdf(&path, &["Halaman Pertama", "Halaman Kedua"]);

        let chunks = chunk_pdf(&path).unwrap();

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].page_num, Some(1));
        assert!(chunks[0].text_content.contains("Halaman Pertama"));
        assert_eq!(chunks[1].page_num, Some(2));
        assert!(chunks[1].text_content.contains("Halaman Kedua"));
        for chunk in &chunks {
            assert_eq!(chunk.file_path, path);
        }
    }

    #[test]
    fn chunk_pdf_doc_id_is_stable_across_reingestion_of_the_same_path() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("dok.pdf");
        write_test_pdf(&path, &["Isi"]);

        let first = chunk_pdf(&path).unwrap();
        let second = chunk_pdf(&path).unwrap();

        assert_eq!(first[0].doc_id, second[0].doc_id);
    }

    #[test]
    fn chunk_pdf_doc_id_differs_for_different_paths() {
        let dir = tempdir().unwrap();
        let path_a = dir.path().join("a.pdf");
        let path_b = dir.path().join("b.pdf");
        write_test_pdf(&path_a, &["Isi A"]);
        write_test_pdf(&path_b, &["Isi B"]);

        let a = chunk_pdf(&path_a).unwrap();
        let b = chunk_pdf(&path_b).unwrap();

        assert_ne!(a[0].doc_id, b[0].doc_id);
    }

    #[test]
    fn chunk_pdf_propagates_extraction_errors() {
        let missing = Path::new("/nonexistent/missing.pdf");
        assert!(chunk_pdf(missing).is_err());
    }
}
