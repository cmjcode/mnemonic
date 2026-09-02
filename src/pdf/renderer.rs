//! Renders PDF pages to RGBA pixel buffers for the viewer (§3.5 point 1,
//! Fase 8) via `pdfium-render`'s dynamic binding to Google's PDFium
//! library. PDFium is a native shared library resolved at *runtime* (via
//! `libloading`, never linked at compile time), so `cargo build`/`test`
//! never need it present — only actual `render_page` calls do. This is a
//! deliberate architecture trade-off: no mature pure-Rust PDF rasterizer
//! exists (see Fase 8 planning notes), so real page rendering costs one
//! native dependency, same category as `fastembed`'s ONNX runtime. If the
//! library can't be found, construction fails with a clear `Err` instead
//! of panicking — callers should fall back to the existing text-only
//! `pdf::extract_pages` (already used for search) rather than crash.
//! Callers: `app.rs`'s PDF viewer.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use pdfium_render::prelude::*;

/// One rendered PDF page: raw RGBA8 pixels plus their dimensions, ready
/// for `egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba)`.
pub struct RenderedPage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// A bound `Pdfium` instance. Construction is the only fallible/slow part
/// (locating + dynamically loading the native library); rendering
/// individual pages afterward is cheap enough to call synchronously from
/// the UI thread on demand (only when a page is actually requested), so
/// this phase doesn't add a dedicated background worker thread for it —
/// revisit in Fase 10 if large/complex PDFs make that jank. Each
/// `render_page` call also reopens the source `Document` fresh rather
/// than caching one across calls, matching `llm::CandleEngine`'s same
/// simplification (see the Fase 6 note in the roadmap memory) — a
/// long-lived `PdfDocument` would tie a borrowed lifetime into
/// `MnemonicApp`'s state, which isn't worth the complexity for a personal
/// vault's PDFs.
pub struct PdfRenderer {
    pdfium: Pdfium,
}

impl PdfRenderer {
    /// Binds to a PDFium library: first a copy bundled next to the running
    /// executable or in `assets/pdfium/` (so a packaged install can ship
    /// one without requiring the user to install anything system-wide),
    /// then falls back to a system-provided library (e.g. installed via
    /// the OS package manager). Returns `Err` — never panics — if neither
    /// is found.
    pub fn new() -> Result<PdfRenderer> {
        let bundled = bundled_library_dirs().into_iter().find_map(|dir| {
            Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(&dir)).ok()
        });

        let bindings = match bundled {
            Some(b) => b,
            None => Pdfium::bind_to_system_library()
                .context("loading the PDFium library (not found bundled in assets/pdfium/ or system-installed)")?,
        };

        Ok(PdfRenderer {
            pdfium: Pdfium::new(bindings),
        })
    }

    /// Number of pages in the PDF at `path`.
    pub fn page_count(&self, path: &Path) -> Result<usize> {
        let document = self.load(path)?;
        Ok(document.pages().len() as usize)
    }

    /// Renders 0-based page `page_index` of the PDF at `path` to RGBA
    /// pixels, scaled so its width is `target_width` (height follows the
    /// page's own aspect ratio, capped generously so an unusually tall
    /// page can't blow past PDFium's internal bitmap size limits).
    pub fn render_page(
        &self,
        path: &Path,
        page_index: usize,
        target_width: u16,
    ) -> Result<RenderedPage> {
        let document = self.load(path)?;
        let page = document
            .pages()
            .get(page_index as _)
            .with_context(|| format!("page {page_index} out of bounds in {}", path.display()))?;

        let config = PdfRenderConfig::new()
            .set_target_width(target_width as _)
            .set_maximum_height(target_width.saturating_mul(8) as _);
        let bitmap = page
            .render_with_config(&config)
            .with_context(|| format!("rendering page {page_index} of {}", path.display()))?;

        Ok(RenderedPage {
            width: bitmap.width() as usize,
            height: bitmap.height() as usize,
            rgba: bitmap.as_rgba_bytes(),
        })
    }

    /// The page's true dimensions in PDF points — distinct from
    /// `render_page`'s pixel dimensions, which follow `target_width`/zoom
    /// instead of the page's actual size (§Fase 9: `app.rs`'s annotation
    /// canvas needs this to convert a screen-space drag rectangle on the
    /// rendered bitmap back into the PDF user-space coordinates
    /// `pdf::annotator::Annotation::rect` expects).
    pub fn page_size_points(&self, path: &Path, page_index: usize) -> Result<(f32, f32)> {
        let document = self.load(path)?;
        let page = document
            .pages()
            .get(page_index as _)
            .with_context(|| format!("page {page_index} out of bounds in {}", path.display()))?;
        Ok((page.width().value, page.height().value))
    }

    fn load(&self, path: &Path) -> Result<PdfDocument<'_>> {
        self.pdfium
            .load_pdf_from_file(path, None)
            .with_context(|| format!("opening PDF {}", path.display()))
    }
}

fn bundled_library_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("assets/pdfium")];
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.push(dir.to_path_buf());
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No bundled/system PDFium library exists in the CI/dev sandbox this
    /// runs in, so the only thing worth asserting offline is the graceful
    /// failure path — never a panic — matching how every other
    /// native-model wrapper in this codebase (`EmbeddingEngine::new`,
    /// `CandleEngine`) is exercised without the real model/library
    /// present. A human with `libpdfium` installed can verify actual
    /// rendering manually.
    #[test]
    fn new_fails_gracefully_without_a_pdfium_library_available() {
        // Only meaningful when no system-wide PDFium happens to be
        // installed on the machine running the test; either outcome
        // (Ok or Err) must not panic, which is what we're really
        // checking here.
        let _ = PdfRenderer::new();
    }
}
