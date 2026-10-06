//! Canvas for agents (§3.9.5): the section outline of a note (the boxes a
//! canvas shows, with their anchors and nesting) and the whole canvas
//! exported as Mermaid — the same `canvas::mermaid_export` the desktop
//! "Export Mermaid" button uses. Read-only: a note without a `.canvas`
//! sidecar is exported from its section outline built in memory; nothing
//! is written. Callers: `api::mcp`, `src/bin/mnemonic-cli.rs`.

use anyhow::Result;

use super::VaultService;
use super::types::{CanvasMermaid, CanvasMermaidRequest, SectionInfo, SectionList};
use crate::canvas::mermaid_export::{ExportOptions, export_canvas};
use crate::markdown::MarkdownEditor;
use crate::markdown::sections::{self, SegmentKind};

impl VaultService {
    /// Every section segment of a note (what becomes a canvas box).
    pub fn list_sections(&self, reference: &str) -> Result<SectionList> {
        let note = self.resolve(reference)?;
        let segs = sections::segments(&note.body);
        let sections = segs
            .iter()
            .map(|s| SectionInfo {
                id: s.id.clone(),
                kind: match &s.kind {
                    SegmentKind::Section { .. } => "section",
                    SegmentKind::Table => "table",
                    SegmentKind::Mermaid => "mermaid",
                    SegmentKind::Code { .. } => "code",
                    SegmentKind::Text => "text",
                }
                .to_string(),
                level: match s.kind {
                    SegmentKind::Section { level } => Some(level),
                    _ => None,
                },
                parent: s.parent.and_then(|p| segs[p].id.clone()),
                line: s.start_line + 1,
                end_line: s.end_line + 1,
                summary: s.summary(),
            })
            .collect();
        Ok(SectionList { path: self.rel(&note.path), sections })
    }

    /// The note's canvas as Mermaid diagrams (one per diagram family).
    pub fn canvas_mermaid(&self, req: &CanvasMermaidRequest) -> Result<CanvasMermaid> {
        let note = self.resolve(&req.r#ref)?.clone();
        let path = self.rel(&note.path);
        let has_sidecar = note.has_sidecar;
        let title = note.frontmatter.title.clone();
        // Exactly what the app shows; in memory only (never saved).
        let mut editor = MarkdownEditor::open_in(note, Some(self.root()));
        let canvas = editor.ensure_canvas().clone();
        let export = export_canvas(&canvas, &ExportOptions { title, mindmap: req.mindmap });
        Ok(CanvasMermaid {
            path,
            has_sidecar,
            markdown: export.to_markdown(),
            diagrams: export.diagrams,
            warnings: export.warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mermaid;
    use crate::notes::Note;

    #[test]
    fn sections_and_mermaid_export_without_writing_anything() {
        let dir = tempfile::tempdir().unwrap();
        let body = "# Produk\nVisi.\n\n## Data\n\n```mermaid\nerDiagram\n  A ||--o{ B : has\n```\n\n## Tim\nOrang.\n";
        Note::create(dir.path(), "Proyek", body).unwrap();
        let svc = VaultService::open(dir.path()).unwrap();

        let list = svc.list_sections("Proyek").unwrap();
        let kinds: Vec<&str> = list.sections.iter().map(|s| s.kind.as_str()).collect();
        assert_eq!(kinds, vec!["section", "section", "mermaid", "section"]);
        assert_eq!(list.sections[1].level, Some(2));
        assert_eq!(list.sections[0].summary, "Produk");

        let out = svc.canvas_mermaid(&CanvasMermaidRequest { r#ref: "Proyek".into(), mindmap: true }).unwrap();
        assert!(!out.has_sidecar);
        let kinds: Vec<&str> = out.diagrams.iter().map(|d| d.kind.as_str()).collect();
        assert_eq!(kinds, vec!["flowchart", "embedded", "mindmap"]);
        for d in &out.diagrams {
            assert!(mermaid::validate(&d.source).1.iter().all(|x| !x.is_error()), "{}", d.source);
        }
        // Read-only: the note on disk is untouched (no anchors, no sidecar).
        let on_disk = std::fs::read_to_string(dir.path().join("Proyek.md")).unwrap();
        assert!(!on_disk.contains(" ^"), "{on_disk}");
        assert!(!dir.path().join("Proyek.canvas").exists());
    }
}
