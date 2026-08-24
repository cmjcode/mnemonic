//! PDF page manipulation (§3.5 point 2, Fase 8) via `lopdf`: merge, split,
//! rotate, and delete pages. Every function reads its input(s), mutates an
//! in-memory `lopdf::Document`, and writes the result to a distinct
//! `output` path — never in place — so a bug or crash mid-operation can
//! never corrupt the source file; callers just point the viewer at
//! `output` afterward. Visual annotation, text injection, metadata
//! editing, and "save/overwrite with auto-backup" (§3.5 points 2-3) are
//! Fase 9's job, not this module's. Callers: `app.rs`'s PDF viewer.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use lopdf::{Document, Object, ObjectId};

/// Merges `inputs`, in order, into a single PDF written to `output`.
/// Bookmarks/outlines aren't preserved (§3.5 only asks for the page
/// content itself to survive the merge) — each source's `Catalog`/`Pages`
/// object trees are combined into one, and every page keeps its original
/// content/resources unchanged.
pub fn merge(inputs: &[&Path], output: &Path) -> Result<()> {
    if inputs.len() < 2 {
        bail!("merge needs at least two input files");
    }

    let mut max_id = 1u32;
    let mut documents_pages: BTreeMap<ObjectId, Object> = BTreeMap::new();
    let mut documents_objects: BTreeMap<ObjectId, Object> = BTreeMap::new();

    for path in inputs {
        let mut doc =
            Document::load(path).with_context(|| format!("loading {} for merge", path.display()))?;
        doc.renumber_objects_with(max_id);
        max_id = doc.max_id + 1;

        for object_id in doc.get_pages().into_values() {
            let object = doc
                .get_object(object_id)
                .with_context(|| format!("reading page object from {}", path.display()))?
                .to_owned();
            documents_pages.insert(object_id, object);
        }
        documents_objects.extend(doc.objects);
    }

    let mut document = Document::with_version("1.5");
    let mut catalog_object: Option<(ObjectId, Object)> = None;
    let mut pages_object: Option<(ObjectId, Object)> = None;

    for (object_id, object) in documents_objects.into_iter() {
        match object.type_name().unwrap_or(b"") {
            b"Catalog" => {
                let id = catalog_object.as_ref().map(|(id, _)| *id).unwrap_or(object_id);
                catalog_object = Some((id, object));
            }
            b"Pages" => {
                if let Ok(dictionary) = object.as_dict() {
                    let mut dictionary = dictionary.clone();
                    if let Some((_, ref old)) = pages_object
                        && let Ok(old_dict) = old.as_dict()
                    {
                        dictionary.extend(old_dict);
                    }
                    let id = pages_object.as_ref().map(|(id, _)| *id).unwrap_or(object_id);
                    pages_object = Some((id, Object::Dictionary(dictionary)));
                }
            }
            b"Page" | b"Outlines" | b"Outline" => {} // handled/dropped separately below
            _ => {
                document.objects.insert(object_id, object);
            }
        }
    }

    let (pages_id, pages_obj) = pages_object.context("no /Pages root found while merging")?;
    let (catalog_id, catalog_obj) = catalog_object.context("no /Catalog root found while merging")?;

    for (object_id, object) in documents_pages.iter() {
        if let Ok(dictionary) = object.as_dict() {
            let mut dictionary = dictionary.clone();
            dictionary.set("Parent", pages_id);
            document.objects.insert(*object_id, Object::Dictionary(dictionary));
        }
    }

    if let Ok(dictionary) = pages_obj.as_dict() {
        let mut dictionary = dictionary.clone();
        dictionary.set("Count", documents_pages.len() as u32);
        dictionary.set(
            "Kids",
            documents_pages.keys().map(|id| Object::Reference(*id)).collect::<Vec<_>>(),
        );
        document.objects.insert(pages_id, Object::Dictionary(dictionary));
    }

    if let Ok(dictionary) = catalog_obj.as_dict() {
        let mut dictionary = dictionary.clone();
        dictionary.set("Pages", pages_id);
        dictionary.remove(b"Outlines"); // not preserved across a merge
        document.objects.insert(catalog_id, Object::Dictionary(dictionary));
    }

    document.trailer.set("Root", catalog_id);
    document.max_id = document.objects.len() as u32;
    document.renumber_objects();

    document
        .save(output)
        .with_context(|| format!("saving merged PDF to {}", output.display()))?;
    Ok(())
}

/// Writes a new PDF to `output` containing only `keep_pages` (1-based page
/// numbers, matching `lopdf::Document::get_pages`'s numbering) from
/// `input`, in their original relative order — the "split off a page
/// range" half of §3.5 point 2. An empty `keep_pages` is rejected rather
/// than silently producing a blank document.
pub fn split(input: &Path, keep_pages: &[u32], output: &Path) -> Result<()> {
    if keep_pages.is_empty() {
        bail!("split needs at least one page to keep");
    }
    let mut doc = Document::load(input).with_context(|| format!("loading {}", input.display()))?;

    let keep: HashSet<u32> = keep_pages.iter().copied().collect();
    let remove: Vec<u32> = doc.get_pages().into_keys().filter(|p| !keep.contains(p)).collect();
    doc.delete_pages(&remove);

    doc.save(output)
        .with_context(|| format!("saving split PDF to {}", output.display()))?;
    Ok(())
}

/// Writes a new PDF to `output` with `pages` (1-based) removed from
/// `input`.
pub fn delete_pages(input: &Path, pages: &[u32], output: &Path) -> Result<()> {
    if pages.is_empty() {
        bail!("delete_pages needs at least one page number");
    }
    let mut doc = Document::load(input).with_context(|| format!("loading {}", input.display()))?;
    doc.delete_pages(pages);
    doc.save(output)
        .with_context(|| format!("saving PDF to {} after deleting pages", output.display()))?;
    Ok(())
}

/// Rotates `pages` (1-based; empty = every page) by `degrees` — added to
/// each page's current `/Rotate` value and normalized into `0..360` —
/// writing the result to `output`. `degrees` must be a multiple of 90, the
/// only rotation PDF viewers are guaranteed to render correctly.
pub fn rotate(input: &Path, pages: &[u32], degrees: i64, output: &Path) -> Result<()> {
    if degrees % 90 != 0 {
        bail!("rotation angle must be a multiple of 90 degrees, got {degrees}");
    }
    let mut doc = Document::load(input).with_context(|| format!("loading {}", input.display()))?;

    let target: Option<HashSet<u32>> = if pages.is_empty() { None } else { Some(pages.iter().copied().collect()) };

    for (page_number, page_id) in doc.get_pages() {
        if target.as_ref().is_some_and(|t| !t.contains(&page_number)) {
            continue;
        }
        let page_dict = doc
            .get_object_mut(page_id)
            .and_then(|obj| obj.as_dict_mut())
            .with_context(|| format!("page {page_number} missing its dictionary"))?;
        let current = page_dict.get(b"Rotate").and_then(|o| o.as_i64()).unwrap_or(0);
        page_dict.set("Rotate", ((current + degrees) % 360 + 360) % 360);
    }

    doc.save(output)
        .with_context(|| format!("saving rotated PDF to {}", output.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Stream, content::Content, content::Operation, dictionary};
    use tempfile::tempdir;

    /// Same minimal-PDF builder used by `pdf::extractor`'s tests, kept
    /// separately here so this module doesn't need to depend on test-only
    /// code in another module.
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

    #[test]
    fn merge_combines_page_counts_of_all_inputs() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a.pdf");
        let b = dir.path().join("b.pdf");
        let out = dir.path().join("merged.pdf");
        write_test_pdf(&a, &["A1", "A2"]);
        write_test_pdf(&b, &["B1"]);

        merge(&[&a, &b], &out).unwrap();

        let merged = Document::load(&out).unwrap();
        assert_eq!(merged.get_pages().len(), 3);
    }

    #[test]
    fn merge_rejects_a_single_input() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a.pdf");
        write_test_pdf(&a, &["A1"]);
        assert!(merge(&[&a], &dir.path().join("out.pdf")).is_err());
    }

    #[test]
    fn split_keeps_only_the_requested_pages() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("dok.pdf");
        let out = dir.path().join("split.pdf");
        write_test_pdf(&input, &["P1", "P2", "P3"]);

        split(&input, &[1, 3], &out).unwrap();

        let result = Document::load(&out).unwrap();
        assert_eq!(result.get_pages().len(), 2);
    }

    #[test]
    fn split_rejects_an_empty_page_list() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("dok.pdf");
        write_test_pdf(&input, &["P1"]);
        assert!(split(&input, &[], &dir.path().join("out.pdf")).is_err());
    }

    #[test]
    fn delete_pages_removes_only_the_given_page() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("dok.pdf");
        let out = dir.path().join("deleted.pdf");
        write_test_pdf(&input, &["P1", "P2", "P3"]);

        delete_pages(&input, &[2], &out).unwrap();

        let result = Document::load(&out).unwrap();
        assert_eq!(result.get_pages().len(), 2);
    }

    #[test]
    fn rotate_sets_rotate_entry_on_every_page_when_none_specified() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("dok.pdf");
        let out = dir.path().join("rotated.pdf");
        write_test_pdf(&input, &["P1", "P2"]);

        rotate(&input, &[], 90, &out).unwrap();

        let result = Document::load(&out).unwrap();
        for page_id in result.get_pages().into_values() {
            let dict = result.get_object(page_id).unwrap().as_dict().unwrap();
            assert_eq!(dict.get(b"Rotate").and_then(|o| o.as_i64()).unwrap(), 90);
        }
    }

    #[test]
    fn rotate_only_affects_the_specified_page() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("dok.pdf");
        let out = dir.path().join("rotated.pdf");
        write_test_pdf(&input, &["P1", "P2"]);

        rotate(&input, &[1], 90, &out).unwrap();

        let result = Document::load(&out).unwrap();
        let pages = result.get_pages();
        let page1_dict = result.get_object(pages[&1]).unwrap().as_dict().unwrap();
        let page2_dict = result.get_object(pages[&2]).unwrap().as_dict().unwrap();
        assert_eq!(page1_dict.get(b"Rotate").and_then(|o| o.as_i64()).unwrap(), 90);
        assert!(page2_dict.get(b"Rotate").is_err());
    }

    #[test]
    fn rotate_wraps_past_360_degrees() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("dok.pdf");
        let once = dir.path().join("once.pdf");
        let twice = dir.path().join("twice.pdf");
        write_test_pdf(&input, &["P1"]);

        rotate(&input, &[], 270, &once).unwrap();
        rotate(&once, &[], 180, &twice).unwrap();

        let result = Document::load(&twice).unwrap();
        let page_id = *result.get_pages().values().next().unwrap();
        let dict = result.get_object(page_id).unwrap().as_dict().unwrap();
        assert_eq!(dict.get(b"Rotate").and_then(|o| o.as_i64()).unwrap(), 90); // (270+180) % 360
    }

    #[test]
    fn rotate_rejects_non_multiple_of_90() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("dok.pdf");
        write_test_pdf(&input, &["P1"]);
        assert!(rotate(&input, &[], 45, &dir.path().join("out.pdf")).is_err());
    }
}
