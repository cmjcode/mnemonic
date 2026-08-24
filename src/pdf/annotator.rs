//! Visual PDF annotation (§3.5 point 2's "Visual Annotation" bullet, plus
//! the "Text Modification/Injection" bullet, Fase 9): highlighter,
//! underline, sticky notes, and free-text injection, each modeled as a
//! standard PDF annotation dictionary appended to a page's `/Annots`
//! array via `lopdf` — no custom rendering code is needed for any of
//! this, since `pdfium-render`'s default `PdfRenderConfig` already draws
//! annotations (`FPDF_ANNOT` is on by default — see `pdf::renderer`),
//! so anything written here shows up in `PdfRenderer::render_page`'s
//! output the next time the annotated file is (re)opened. Text Injection
//! deliberately reuses the annotation route (`/FreeText`) rather than
//! hand-editing the page's content stream/font resources directly — far
//! more robust for arbitrary PDFs, and every viewer already knows how to
//! render it.
//!
//! Like `pdf::editor`, every function here reads `input` and writes to a
//! distinct `output` path, never in place — Fase 9's "overwrite with
//! auto-backup" (§3.5 point 3, `pdf::editor::save_over`) is applied
//! separately, *after* annotations have been baked into a staged file, so
//! a bug here can never corrupt a file the user hasn't explicitly chosen
//! to overwrite. Callers: `app.rs`'s PDF viewer annotation canvas.

use std::path::Path;

use anyhow::{Context, Result, bail};
use lopdf::{Dictionary, Document, Object};

/// Which kind of mark `Annotation::kind` is (§3.5 point 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationKind {
    Highlight,
    Underline,
    StickyNote,
    /// "Text Injection": a `/FreeText` annotation that draws `contents`
    /// inside `rect` directly on the page.
    TextInjection,
}

/// One annotation to place on `page` (1-based, matching `pdf::editor`'s
/// page numbering) at `rect` — `(x0, y0, x1, y1)` in PDF user-space
/// points, origin bottom-left, matching `pdfium-render`'s
/// `PdfPage::width()`/`height()` coordinate system so `app.rs` can convert
/// a screen-space drag rectangle straight across. `color` is `(r, g, b)`
/// in `0.0..=1.0`. `contents` is the sticky-note comment or the injected
/// text; left empty for `Highlight`/`Underline`.
#[derive(Debug, Clone)]
pub struct Annotation {
    pub kind: AnnotationKind,
    pub page: u32,
    pub rect: (f32, f32, f32, f32),
    pub color: (f32, f32, f32),
    pub contents: String,
}

/// Reads `input`, appends every entry of `annotations` to its target
/// page's `/Annots` array, and writes the result to `output`. An
/// annotation whose `page` doesn't exist in `input` is skipped rather
/// than erroring the whole batch (matches `pdf::editor::rotate`'s
/// "ignore what doesn't apply" stance for stale UI state — e.g. an
/// annotation queued against a page since deleted by another op).
pub fn add_annotations(input: &Path, annotations: &[Annotation], output: &Path) -> Result<()> {
    if annotations.is_empty() {
        bail!("add_annotations needs at least one annotation");
    }
    let mut doc = Document::load(input).with_context(|| format!("loading {}", input.display()))?;
    let pages = doc.get_pages();

    for annotation in annotations {
        let Some(&page_id) = pages.get(&annotation.page) else { continue };
        let annot_id = doc.add_object(Object::Dictionary(build_annotation_dict(annotation)));

        let page_dict = doc
            .get_object_mut(page_id)
            .and_then(|obj| obj.as_dict_mut())
            .with_context(|| format!("page {} missing its dictionary", annotation.page))?;
        let mut annots: Vec<Object> =
            page_dict.get(b"Annots").ok().and_then(|o| o.as_array().ok()).cloned().unwrap_or_default();
        annots.push(Object::Reference(annot_id));
        page_dict.set("Annots", annots);
    }

    doc.compress();
    doc.save(output).with_context(|| format!("saving annotated PDF to {}", output.display()))?;
    Ok(())
}

/// Builds the `/Annot` dictionary for one `Annotation`, per PDF 1.7
/// §12.5.6's `/Highlight`, `/Underline`, `/Text`, and `/FreeText`
/// subtypes.
fn build_annotation_dict(annotation: &Annotation) -> Dictionary {
    let (x0, y0, x1, y1) = annotation.rect;
    let (r, g, b) = annotation.color;

    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"Annot".to_vec()));
    dict.set("Rect", Object::Array(vec![Object::Real(x0), Object::Real(y0), Object::Real(x1), Object::Real(y1)]));
    dict.set("C", Object::Array(vec![Object::Real(r), Object::Real(g), Object::Real(b)]));
    dict.set("F", Object::Integer(4)); // Print flag: visible when printed too, not just on screen.
    dict.set("Contents", Object::string_literal(annotation.contents.as_bytes()));

    match annotation.kind {
        AnnotationKind::Highlight | AnnotationKind::Underline => {
            let subtype: &[u8] = if annotation.kind == AnnotationKind::Highlight { b"Highlight" } else { b"Underline" };
            dict.set("Subtype", Object::Name(subtype.to_vec()));
            // QuadPoints order per spec: top-left, top-right, bottom-left, bottom-right.
            dict.set(
                "QuadPoints",
                Object::Array(vec![
                    Object::Real(x0),
                    Object::Real(y1),
                    Object::Real(x1),
                    Object::Real(y1),
                    Object::Real(x0),
                    Object::Real(y0),
                    Object::Real(x1),
                    Object::Real(y0),
                ]),
            );
        }
        AnnotationKind::StickyNote => {
            dict.set("Subtype", Object::Name(b"Text".to_vec()));
            dict.set("Name", Object::Name(b"Comment".to_vec()));
            dict.set("Open", Object::Boolean(false));
        }
        AnnotationKind::TextInjection => {
            dict.set("Subtype", Object::Name(b"FreeText".to_vec()));
            dict.set("DA", Object::string_literal(&b"0 0 0 rg /Helv 12 Tf"[..]));
            dict.set("Border", Object::Array(vec![0.into(), 0.into(), 0.into()]));
        }
    }
    dict
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Stream, content::Content, content::Operation, dictionary};
    use tempfile::tempdir;

    /// Same minimal-PDF builder used by `pdf::editor`'s tests.
    fn write_test_pdf(path: &Path, pages_text: &[&str]) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let resources_id = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });

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
                let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
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

    fn sample_annotation(kind: AnnotationKind, page: u32) -> Annotation {
        Annotation { kind, page, rect: (72.0, 700.0, 200.0, 720.0), color: (1.0, 0.9, 0.2), contents: "note".into() }
    }

    #[test]
    fn add_annotations_rejects_an_empty_batch() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("dok.pdf");
        write_test_pdf(&input, &["P1"]);
        assert!(add_annotations(&input, &[], &dir.path().join("out.pdf")).is_err());
    }

    #[test]
    fn add_annotations_appends_to_the_target_pages_annots_array() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("dok.pdf");
        let out = dir.path().join("annotated.pdf");
        write_test_pdf(&input, &["P1", "P2"]);

        add_annotations(&input, &[sample_annotation(AnnotationKind::Highlight, 1)], &out).unwrap();

        let result = Document::load(&out).unwrap();
        let pages = result.get_pages();
        let page1 = result.get_object(pages[&1]).unwrap().as_dict().unwrap();
        let annots = page1.get(b"Annots").unwrap().as_array().unwrap();
        assert_eq!(annots.len(), 1);
        let page2 = result.get_object(pages[&2]).unwrap().as_dict().unwrap();
        assert!(page2.get(b"Annots").is_err());
    }

    #[test]
    fn add_annotations_skips_an_out_of_range_page_without_erroring() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("dok.pdf");
        let out = dir.path().join("annotated.pdf");
        write_test_pdf(&input, &["P1"]);

        add_annotations(&input, &[sample_annotation(AnnotationKind::Underline, 99)], &out).unwrap();

        let result = Document::load(&out).unwrap();
        let page1 = result.get_object(result.get_pages()[&1]).unwrap().as_dict().unwrap();
        assert!(page1.get(b"Annots").is_err());
    }

    #[test]
    fn highlight_and_underline_get_quad_points() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("dok.pdf");
        let out = dir.path().join("annotated.pdf");
        write_test_pdf(&input, &["P1"]);

        add_annotations(
            &input,
            &[sample_annotation(AnnotationKind::Highlight, 1), sample_annotation(AnnotationKind::Underline, 1)],
            &out,
        )
        .unwrap();

        let result = Document::load(&out).unwrap();
        let page1 = result.get_object(result.get_pages()[&1]).unwrap().as_dict().unwrap();
        for annot_ref in page1.get(b"Annots").unwrap().as_array().unwrap() {
            let annot_id = annot_ref.as_reference().unwrap();
            let annot = result.get_object(annot_id).unwrap().as_dict().unwrap();
            assert!(annot.get(b"QuadPoints").is_ok());
        }
    }

    #[test]
    fn sticky_note_uses_text_subtype() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("dok.pdf");
        let out = dir.path().join("annotated.pdf");
        write_test_pdf(&input, &["P1"]);

        add_annotations(&input, &[sample_annotation(AnnotationKind::StickyNote, 1)], &out).unwrap();

        let result = Document::load(&out).unwrap();
        let page1 = result.get_object(result.get_pages()[&1]).unwrap().as_dict().unwrap();
        let annot_ref = &page1.get(b"Annots").unwrap().as_array().unwrap()[0];
        let annot = result.get_object(annot_ref.as_reference().unwrap()).unwrap().as_dict().unwrap();
        assert_eq!(annot.get(b"Subtype").unwrap().as_name().unwrap(), b"Text");
    }

    #[test]
    fn text_injection_uses_freetext_subtype_and_carries_contents() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("dok.pdf");
        let out = dir.path().join("annotated.pdf");
        write_test_pdf(&input, &["P1"]);

        let mut annotation = sample_annotation(AnnotationKind::TextInjection, 1);
        annotation.contents = "Injected text".into();
        add_annotations(&input, &[annotation], &out).unwrap();

        let result = Document::load(&out).unwrap();
        let page1 = result.get_object(result.get_pages()[&1]).unwrap().as_dict().unwrap();
        let annot_ref = &page1.get(b"Annots").unwrap().as_array().unwrap()[0];
        let annot = result.get_object(annot_ref.as_reference().unwrap()).unwrap().as_dict().unwrap();
        assert_eq!(annot.get(b"Subtype").unwrap().as_name().unwrap(), b"FreeText");
        let contents = annot.get(b"Contents").unwrap();
        assert!(matches!(contents, Object::String(bytes, _) if bytes == b"Injected text"));
    }

    #[test]
    fn multiple_annotations_on_the_same_page_all_land_in_annots() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("dok.pdf");
        let out = dir.path().join("annotated.pdf");
        write_test_pdf(&input, &["P1"]);

        add_annotations(
            &input,
            &[
                sample_annotation(AnnotationKind::Highlight, 1),
                sample_annotation(AnnotationKind::StickyNote, 1),
                sample_annotation(AnnotationKind::TextInjection, 1),
            ],
            &out,
        )
        .unwrap();

        let result = Document::load(&out).unwrap();
        let page1 = result.get_object(result.get_pages()[&1]).unwrap().as_dict().unwrap();
        assert_eq!(page1.get(b"Annots").unwrap().as_array().unwrap().len(), 3);
    }
}
