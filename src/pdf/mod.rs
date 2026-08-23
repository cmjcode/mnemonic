//! PDF module. Fase 4 (§3.3 point 1) only needs text extraction to feed
//! the ingestion pipeline; viewing/editing/annotation (§3.5, Fase 8-9)
//! land in later phases. Callers: `core::ingestion`.

pub mod extractor;

pub use extractor::extract_pages;
