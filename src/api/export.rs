//! Reading themes and note export for agents (§3.2.5): list the themes
//! (built-ins and plugins, with their colours) and export a note to HTML
//! or PDF in a theme's print colours — the same `reading_theme` and
//! `export` code the desktop app prints with. Callers: `api::mcp`,
//! `src/bin/mnemonic-cli.rs`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use super::VaultService;
use super::types::{ExportRequest, ExportResult, ThemeInfo, ThemeList};
use crate::export::{self, ExportFormat, ExportSource};
use crate::reading_theme::{self, ThemeColors, ThemeRegistry, ThemeSource};

impl VaultService {
    /// Built-in and plugin themes (per-user and this vault's folder).
    pub fn themes(&self) -> ThemeRegistry {
        ThemeRegistry::load(&reading_theme::theme_dirs(Some(self.root())))
    }

    pub fn list_themes(&self) -> ThemeList {
        let registry = self.themes();
        ThemeList {
            themes: registry
                .list()
                .iter()
                .map(|t| ThemeInfo {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    author: t.author.clone(),
                    description: t.description.clone(),
                    source: match &t.source {
                        ThemeSource::BuiltIn => "built-in".into(),
                        ThemeSource::File(p) => p.display().to_string(),
                    },
                    light: colors_map(&t.light),
                    dark: colors_map(&t.dark),
                    print: colors_map(&t.print),
                })
                .collect(),
            plugin_dirs: reading_theme::theme_dirs(Some(self.root())).iter().map(|d| d.display().to_string()).collect(),
            problems: registry.problems.clone(),
        }
    }

    /// Exports a note as HTML (inline or to `out`) or PDF (to `out`).
    pub fn export_note(&self, req: &ExportRequest) -> Result<ExportResult> {
        let Some(format) = ExportFormat::parse(&req.format) else {
            bail!("unknown export format {:?}: use html or pdf", req.format);
        };
        let note = self.resolve(&req.r#ref)?;
        let registry = self.themes();
        let theme_id = match req.theme.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            Some(id) if registry.contains(id) => id.to_string(),
            Some(id) => {
                let known: Vec<&str> = registry.list().iter().map(|t| t.id.as_str()).collect();
                bail!("unknown theme {id:?}; available: {}", known.join(", "));
            }
            None => reading_theme::note_theme(&note.frontmatter.extra)
                .filter(|id| registry.contains(id))
                .unwrap_or(reading_theme::DEFAULT_THEME)
                .to_string(),
        };
        let theme = registry.get(&theme_id);
        let resolve = |target: &str| export::resolve_embed(&self.vault, target);
        let src = ExportSource {
            title: &note.frontmatter.title,
            body: &note.body,
            theme,
            resolve_embed: &resolve,
            lang: "en",
        };
        let mut result = ExportResult {
            note: self.rel(&note.path),
            format: format.extension().into(),
            theme: theme.id.clone(),
            path: None,
            html: None,
            bytes: 0,
        };
        match req.out.as_deref() {
            Some(out) => {
                let path = PathBuf::from(out);
                let path = if path.is_absolute() { path } else { self.root().join(path) };
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
                }
                export::export_note(&src, format, &path)?;
                result.bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                result.path = Some(path.display().to_string());
            }
            None if format == ExportFormat::Html => {
                let html = export::html::note_document(src.title, src.body, theme, &resolve, &export::HtmlOptions {
                    auto_print: false,
                    lang: src.lang.into(),
                });
                result.bytes = html.len() as u64;
                result.html = Some(html);
            }
            None => bail!("PDF export needs `out` (a file path)"),
        }
        Ok(result)
    }
}

fn colors_map(c: &ThemeColors) -> BTreeMap<String, String> {
    c.entries().into_iter().map(|(k, v)| (k.to_string(), v.hex())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::CreateNoteRequest;

    fn service() -> (tempfile::TempDir, VaultService) {
        let dir = tempfile::tempdir().unwrap();
        let mut svc = VaultService::open(dir.path()).unwrap();
        svc.create_note(&CreateNoteRequest {
            title: "Resep".into(),
            body: "## Bahan\n> [!tip] Kiat\n> Pakai **gula aren**.\n".into(),
            folder: None,
            tags: vec![],
        })
        .unwrap();
        (dir, svc)
    }

    fn req(format: &str, theme: Option<&str>, out: Option<&str>) -> ExportRequest {
        ExportRequest {
            r#ref: "Resep".into(),
            format: format.into(),
            theme: theme.map(Into::into),
            out: out.map(Into::into),
        }
    }

    #[test]
    fn themes_are_listed_with_colours() {
        let (_d, svc) = service();
        let list = svc.list_themes();
        let pelangi = list.themes.iter().find(|t| t.id == "pelangi").unwrap();
        assert_eq!(pelangi.source, "built-in");
        assert!(pelangi.light["h1"].starts_with('#'));
        assert_eq!(pelangi.light.len(), ThemeColors::FIELDS.len());
        assert!(list.plugin_dirs.iter().any(|d| d.ends_with("themes")));
    }

    #[test]
    fn html_export_inline_and_to_a_file() {
        let (dir, svc) = service();
        let inline = svc.export_note(&req("html", Some("sunset"), None)).unwrap();
        let html = inline.html.unwrap();
        assert_eq!(inline.theme, "sunset");
        assert!(html.contains("callout-tip") && html.contains("<strong>gula aren</strong>"));
        let written = svc.export_note(&req("HTML", None, Some("out/resep.html"))).unwrap();
        assert_eq!(written.theme, "mnemonic");
        assert!(dir.path().join("out/resep.html").exists());
        assert!(written.bytes > 0 && written.html.is_none());
    }

    #[test]
    fn bad_requests_are_explained() {
        let (_d, svc) = service();
        let err = svc.export_note(&req("docx", None, None)).unwrap_err().to_string();
        assert!(err.contains("html or pdf"), "{err}");
        let err = svc.export_note(&req("html", Some("tidak-ada"), None)).unwrap_err().to_string();
        assert!(err.contains("pelangi"), "{err}");
        let err = svc.export_note(&req("pdf", None, None)).unwrap_err().to_string();
        assert!(err.contains("out"), "{err}");
    }
}
