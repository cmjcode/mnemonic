//! Per-page PDF text extraction (§3.3 point 1) via the `pdf-extract` crate.
//! Feeds `core::ingestion`'s chunker the same way a note's Markdown body
//! does. Callers: `core::ingestion::chunk_pdf`.

use std::path::Path;

use anyhow::{Context, Result};

/// Extracts text from the PDF at `path`, one string per page — `result[0]`
/// is page 1, `result[1]` is page 2, and so on. `core::ingestion::chunk_pdf`
/// pairs each entry with its 1-based page number for citations (§3.4
/// point 4).
pub fn extract_pages(path: &Path) -> Result<Vec<String>> {
    pdf_extract::extract_text_by_pages(path)
        .with_context(|| format!("extracting text from PDF {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Document, Object, Stream, content::Content, content::Operation, dictionary};
    use tempfile::tempdir;

    /// Builds a minimal multi-page PDF with one line of text per page, for
    /// exercising `extract_pages` without shipping a binary fixture file.
    fn write_test_pdf(path: &Path, pages_text: &[&str]) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();

        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });

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
                let page_id = doc.add_object(dictionary! {
                    "Type" => "Page",
                    "Parent" => pages_id,
                    "Contents" => content_id,
                });
                page_id.into()
            })
            .collect();

        let count = page_ids.len() as i64;
        let pages = dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids,
            "Count" => count,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));

        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);

        doc.save(path).unwrap();
    }

    #[test]
    fn extracts_text_per_page_in_order() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sample.pdf");
        write_test_pdf(&path, &["Halaman Pertama", "Halaman Kedua"]);

        let pages = extract_pages(&path).unwrap();

        assert_eq!(pages.len(), 2);
        assert!(pages[0].contains("Halaman Pertama"));
        assert!(pages[1].contains("Halaman Kedua"));
    }

    #[test]
    fn missing_file_returns_error_instead_of_panicking() {
        let result = extract_pages(Path::new("/nonexistent/path/does-not-exist.pdf"));
        assert!(result.is_err());
    }
}
