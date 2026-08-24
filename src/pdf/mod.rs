//! PDF module. Fase 4 (§3.3 point 1) started with text extraction to feed
//! the ingestion pipeline (`extractor`). Fase 8 (§3.5 points 1-2) adds the
//! two remaining pieces: `renderer` (page → RGBA bitmap for the viewer,
//! via `pdfium-render`) and `editor` (merge/split/rotate/delete pages via
//! `lopdf`). Annotation/text-injection/metadata editing (§3.5 point 2's
//! remainder) are Fase 9. Callers: `core::ingestion`, `app.rs`.

pub mod editor;
pub mod extractor;
pub mod renderer;

pub use extractor::extract_pages;
pub use renderer::{PdfRenderer, RenderedPage};
