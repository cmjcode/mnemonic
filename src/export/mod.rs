//! Print and export of notes in their reading theme (§3.2.5): the note
//! becomes one self-contained HTML page whose CSS is generated from the
//! theme's `[print]` colours (`export::html`), saved as `.html`, turned
//! into a PDF by a headless Chromium-based browser, or opened in the
//! default browser with its print dialog up — so paper, PDF and screen
//! share one look. Callers: `app::reading` (menu, palette),
//! `api::VaultService::export_note` (CLI/MCP).

pub mod html;
pub mod system;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::markdown::renderer::{EmbedContent, EmbedResolver};
use crate::markdown::wikilink::{self, title_key};
use crate::notes::Vault;
use crate::reading_theme::ReadingTheme;

pub use html::HtmlOptions;

/// File formats a note can be exported to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Html,
    Pdf,
}

impl ExportFormat {
    /// `html` / `pdf` (case-insensitive).
    pub fn parse(s: &str) -> Option<ExportFormat> {
        match s.trim().to_ascii_lowercase().as_str() {
            "html" | "htm" => Some(ExportFormat::Html),
            "pdf" => Some(ExportFormat::Pdf),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Html => "html",
            ExportFormat::Pdf => "pdf",
        }
    }
}

/// Everything about the note being exported.
pub struct ExportSource<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub theme: &'a ReadingTheme,
    pub resolve_embed: &'a EmbedResolver<'a>,
    /// Page language, e.g. `id`.
    pub lang: &'a str,
}

impl ExportSource<'_> {
    fn document(&self, auto_print: bool) -> String {
        let opts = HtmlOptions { auto_print, lang: self.lang.to_string() };
        html::note_document(self.title, self.body, self.theme, self.resolve_embed, &opts)
    }
}

/// Writes the note to `out` as HTML or PDF. PDF needs a Chromium-based
/// browser (`system::find_browser`).
pub fn export_note(src: &ExportSource<'_>, format: ExportFormat, out: &Path) -> Result<()> {
    let html = src.document(false);
    match format {
        ExportFormat::Html => std::fs::write(out, html).with_context(|| format!("writing {}", out.display())),
        ExportFormat::Pdf => {
            let browser = system::find_browser().with_context(|| {
                format!(
                    "no Chrome, Edge, Chromium or Brave found to render the PDF; install one, set {} to its path, or export HTML and print it",
                    system::BROWSER_ENV
                )
            })?;
            let page = scratch_file(src.title, "html")?;
            std::fs::write(&page, html).with_context(|| format!("writing {}", page.display()))?;
            let result = system::html_to_pdf(&browser, &page, out);
            let _ = std::fs::remove_file(&page);
            result
        }
    }
}

/// Writes a print-ready page (print dialog opens on load) to the temp
/// folder, opens it in the default browser, and returns its path.
pub fn print_note(src: &ExportSource<'_>) -> Result<PathBuf> {
    let page = scratch_file(src.title, "html")?;
    std::fs::write(&page, src.document(true)).with_context(|| format!("writing {}", page.display()))?;
    system::open_path(&page)?;
    Ok(page)
}

/// Resolves `![[target]]` in `vault`: a note by title/stem/alias (its
/// body, for transclusion) or an attachment anywhere in the vault by file
/// name (hidden folders skipped, like the note scan). Shared by the Live
/// view and export so both show the same embeds.
pub fn resolve_embed(vault: &Vault, target: &str) -> Option<EmbedContent> {
    let key = title_key(target);
    if let Some(note) = vault
        .notes
        .iter()
        .filter(|n| !n.frontmatter.trashed)
        .find(|n| wikilink::link_keys_for(n).contains(&key))
    {
        return Some(EmbedContent::Note { title: note.frontmatter.title.clone(), body: note.body.clone() });
    }
    let wanted = target.trim();
    let found = walkdir::WalkDir::new(&vault.root)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || !(e.file_type().is_dir()
                    && e.file_name().to_str().is_some_and(crate::notes::vault::is_skipped_dir_name))
        })
        .flatten()
        .find(|e| e.file_type().is_file() && e.file_name().to_string_lossy().eq_ignore_ascii_case(wanted))
        .map(|e| e.path().to_path_buf())?;
    if crate::sheet::is_sheet_path(&found) {
        return Some(EmbedContent::Sheet(found));
    }
    Some(EmbedContent::Image(found))
}

/// A file in `<temp>/mnemonic-export/` named after the note.
fn scratch_file(title: &str, ext: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("mnemonic-export");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir.join(format!("{}.{ext}", file_stem_for(title))))
}

/// A safe file name for a note title (`Resep: Kue/Roti` → `Resep- Kue-Roti`).
pub fn file_stem_for(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { '-' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() { "note".to_string() } else { trimmed.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reading_theme::ThemeRegistry;

    #[test]
    fn formats_and_file_names() {
        assert_eq!(ExportFormat::parse(" PDF "), Some(ExportFormat::Pdf));
        assert_eq!(ExportFormat::parse("docx"), None);
        assert_eq!(ExportFormat::Html.extension(), "html");
        assert_eq!(file_stem_for("Resep: Kue/Roti"), "Resep- Kue-Roti");
        assert_eq!(file_stem_for(" .. "), "note");
    }

    #[test]
    fn html_export_writes_a_themed_page() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("a.html");
        let reg = ThemeRegistry::default();
        let src = ExportSource {
            title: "Uji",
            body: "## Sub\n- [x] ok",
            theme: reg.get("ocean"),
            resolve_embed: &|_| None,
            lang: "id",
        };
        export_note(&src, ExportFormat::Html, &out).unwrap();
        let html = std::fs::read_to_string(&out).unwrap();
        assert!(html.contains(&reg.get("ocean").print.h2.hex()));
        assert!(html.contains("<h2>Sub</h2>"));
    }
}
