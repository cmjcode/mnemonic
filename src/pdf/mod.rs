//! PDF module. Fase 4 (§3.3 point 1) started with text extraction to feed
//! the ingestion pipeline (`extractor`). Fase 8 (§3.5 points 1-2) added
//! `renderer` (page → RGBA bitmap for the viewer, via `pdfium-render`)
//! and `editor` (merge/split/rotate/delete pages via `lopdf`). Fase 9
//! (§3.5 points 2-3) adds `annotator` (highlight/underline/sticky-note/
//! text-injection annotations) and rounds out `editor` with a metadata
//! editor (Title/Author/Keywords) plus "save in place with auto-backup".
//! Callers: `core::ingestion`, `app.rs`.

/// Butuh PDFium; hanya pada build `gui`.
#[cfg(feature = "gui")]
pub mod annotator;
pub mod editor;
pub mod extractor;
/// Butuh PDFium; hanya pada build `gui`.
#[cfg(feature = "gui")]
pub mod renderer;

pub use extractor::extract_pages;
#[cfg(feature = "gui")]
pub use renderer::{PdfRenderer, RenderedPage};
