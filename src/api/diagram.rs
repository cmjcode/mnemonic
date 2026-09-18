//! Mermaid diagrams for agents (§3.7.7): list the ```` ```mermaid ````
//! fences of a note, validate a diagram (typed diagnostics with line and
//! column, also mapped to note lines), and render it to SVG — all through
//! the same `mermaid` module the desktop app draws with. The free
//! functions work on raw sources without a vault (`mnemonic-cli diagram
//! validate --file x.mmd`). Callers: `api::mcp`, `src/bin/mnemonic-cli.rs`.

use anyhow::{Result, bail};

use super::VaultService;
use super::types::{DiagramCheck, DiagramDiagnostic, DiagramInfo, DiagramList, DiagramRender, DiagramRequest};
use crate::mermaid::{self, Diagnostic, RenderOptions, Severity, text::ApproxMeasure};

impl VaultService {
    /// Every mermaid fence of a note, each validated.
    pub fn list_diagrams(&self, reference: &str) -> Result<DiagramList> {
        let note = self.resolve(reference)?;
        let diagrams = mermaid::fenced_blocks(&note.body)
            .into_iter()
            .enumerate()
            .map(|(index, block)| DiagramInfo {
                index,
                line: block.fence_line + 1,
                closed: block.closed,
                check: check_source(&block.source, Some(block.fence_line)),
                source: block.source,
            })
            .collect();
        Ok(DiagramList { path: self.rel(&note.path), diagrams })
    }

    pub fn validate_diagram(&self, req: &DiagramRequest) -> Result<DiagramCheck> {
        let (source, fence) = self.diagram_source(req)?;
        Ok(check_source(&source, fence))
    }

    pub fn render_diagram(&self, req: &DiagramRequest) -> Result<DiagramRender> {
        let (source, fence) = self.diagram_source(req)?;
        render_source(&source, req.dark, fence)
    }

    /// The requested diagram text and, for note diagrams, the 0-based
    /// line of its opening fence.
    fn diagram_source(&self, req: &DiagramRequest) -> Result<(String, Option<usize>)> {
        if let Some(src) = &req.source {
            return Ok((src.clone(), None));
        }
        let Some(reference) = &req.r#ref else {
            bail!("give either `source` (diagram text) or `ref` (a note) with an optional `index`");
        };
        let note = self.resolve(reference)?;
        match mermaid::fenced_blocks(&note.body).into_iter().nth(req.index) {
            Some(b) => Ok((b.source, Some(b.fence_line))),
            None => bail!("note {reference:?} has no mermaid diagram #{}", req.index),
        }
    }
}

fn to_api(d: &Diagnostic, fence: Option<usize>) -> DiagramDiagnostic {
    DiagramDiagnostic {
        line: d.line,
        // Fence on 0-based line f → diagram line L is 1-based note line f + 1 + L.
        note_line: fence.map(|f| f + 1 + d.line),
        col: d.col,
        severity: match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
        .to_string(),
        message: d.message.clone(),
    }
}

/// Validate a raw diagram source (`fence` = 0-based fence line when the
/// source came from a note).
pub fn check_source(source: &str, fence: Option<usize>) -> DiagramCheck {
    let (kind, diagnostics) = mermaid::validate(source);
    DiagramCheck {
        kind: kind.name().to_string(),
        supported: kind.is_supported(),
        valid: !diagnostics.iter().any(Diagnostic::is_error),
        diagnostics: diagnostics.iter().map(|d| to_api(d, fence)).collect(),
    }
}

/// Render a raw diagram source to SVG. Fails when nothing can be drawn.
pub fn render_source(source: &str, dark: bool, fence: Option<usize>) -> Result<DiagramRender> {
    let rendered = mermaid::render(source, &RenderOptions { dark, measure: &ApproxMeasure });
    let diagnostics: Vec<DiagramDiagnostic> = rendered.diagnostics.iter().map(|d| to_api(d, fence)).collect();
    let Some(scene) = rendered.scene else {
        let msgs: Vec<String> = rendered.diagnostics.iter().map(|d| d.to_string()).collect();
        bail!("diagram cannot be rendered: {}", msgs.join("; "));
    };
    Ok(DiagramRender {
        kind: rendered.kind.name().to_string(),
        format: "svg".into(),
        width: scene.width,
        height: scene.height,
        svg: mermaid::svg::to_svg(&scene),
        valid: !rendered.diagnostics.iter().any(Diagnostic::is_error),
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::Note;
    use tempfile::tempdir;

    #[test]
    fn lists_validates_and_renders_note_diagrams() {
        let dir = tempdir().unwrap();
        let body = "Intro\n\n```mermaid\nflowchart LR\n  A --> B\n```\n\n```mermaid\nflowchart TD\n  A -> B\n```\n";
        Note::create(dir.path(), "Diagrams", body).unwrap();
        let svc = VaultService::open(dir.path()).unwrap();

        let list = svc.list_diagrams("Diagrams").unwrap();
        assert_eq!(list.diagrams.len(), 2);
        assert!(list.diagrams[0].check.valid);
        assert_eq!(list.diagrams[0].check.kind, "flowchart");
        let bad = &list.diagrams[1];
        assert!(!bad.check.valid);
        let err = &bad.check.diagnostics[0];
        assert_eq!(err.line, 2);
        assert_eq!(err.note_line, Some(bad.line + err.line));

        let req = DiagramRequest { r#ref: Some("Diagrams".into()), ..DiagramRequest::default() };
        let out = svc.render_diagram(&req).unwrap();
        assert!(out.svg.starts_with("<svg") && out.width > 0.0);

        let missing = DiagramRequest { r#ref: Some("Diagrams".into()), index: 5, ..DiagramRequest::default() };
        assert!(svc.render_diagram(&missing).is_err());
    }

    #[test]
    fn raw_sources_need_no_vault() {
        assert!(check_source("flowchart LR\nA-->B", None).valid);
        assert_eq!(check_source("sequenceDiagram\nA->>B: hi", None).kind, "sequence");
        assert!(render_source("nonsense", false, None).is_err());
    }
}
