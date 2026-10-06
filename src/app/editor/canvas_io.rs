//! Canvas file actions (§3.9.4, §Fase 3): export the canvas as Mermaid
//! (a `.md` of fences, or a `.mmd` for a single diagram; also copied to the
//! clipboard), turn the note's ```` ```mermaid ```` fences into canvas
//! objects, and Draw.io import/export. Dialogs run on the UI thread like the
//! rest of the canvas dock. Callers: `app::editor::canvas_surface`.

use crate::canvas::mermaid_export::{ExportOptions, export_canvas};
use crate::canvas::mermaid_import::diagram_to_elements;
use crate::canvas::{self, BlockBinding, CanvasDocument, outline};
use crate::i18n::LocaleManager;
use crate::markdown::MarkdownEditor;
use crate::ui::ToastKind;

fn file_stem(title: &str) -> String {
    let s: String = title.chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect();
    if s.is_empty() { "canvas".into() } else { s }
}

/// Saves the canvas as Mermaid and copies it to the clipboard.
pub(super) fn export_mermaid(
    ctx: &egui::Context,
    canvas: &CanvasDocument,
    tr: &LocaleManager,
) -> Option<(ToastKind, String)> {
    let export = export_canvas(canvas, &ExportOptions { title: canvas.title.clone(), mindmap: true });
    if export.diagrams.is_empty() {
        return Some((ToastKind::Error, tr.t("canvas-export-mermaid-empty", &[])));
    }
    let (ext, text) = if export.diagrams.len() == 1 {
        ("mmd", format!("{}\n", export.diagrams[0].source.trim_end()))
    } else {
        ("md", export.to_markdown())
    };
    ctx.copy_text(text.clone());
    let path = rfd::FileDialog::new()
        .add_filter("Mermaid", &[ext])
        .set_file_name(format!("{}.{ext}", file_stem(&canvas.title)))
        .save_file();
    let count = export.diagrams.len().to_string();
    let done = match path {
        None => tr.t("canvas-export-mermaid-copied", &[("count", &count)]),
        Some(path) => match std::fs::write(&path, text) {
            Ok(()) => tr.t("canvas-export-mermaid-success", &[("count", &count)]),
            Err(e) => return Some((ToastKind::Error, format!("{}: {e}", tr.t("canvas-export-failed", &[])))),
        },
    };
    Some(if export.warnings.is_empty() {
        (ToastKind::Success, done)
    } else {
        (ToastKind::Info, format!("{done} · {}", export.warnings.join("; ")))
    })
}

/// Turns every ```mermaid fence of the note into canvas objects, laid out
/// side by side starting at world `at`. The fences stay in the Markdown.
pub(super) fn import_note_mermaid(editor: &mut MarkdownEditor, at: [f32; 2], tr: &LocaleManager) -> (ToastKind, String) {
    let fences = crate::mermaid::fenced_blocks(&editor.note.body);
    if fences.is_empty() {
        return (ToastKind::Error, tr.t("canvas-import-mermaid-none", &[]));
    }
    let mut x = at[0];
    let mut errors: Vec<String> = Vec::new();
    let mut made = 0usize;
    let mut new: Vec<canvas::CanvasElement> = Vec::new();
    for (i, f) in fences.iter().enumerate() {
        match diagram_to_elements(&f.source, [x, at[1]]) {
            Ok(elems) => {
                x = elems.iter().map(|e| e.bounding_rect().max.x).fold(x, f32::max) + 120.0;
                made += 1;
                new.extend(elems);
            }
            Err(e) => errors.push(format!("#{}: {e}", i + 1)),
        }
    }
    editor.ensure_canvas();
    if let Some(canvas) = editor.canvas.as_mut() {
        canvas.elements.extend(new);
        outline::reattach_connectors(canvas, None);
    }
    editor.sync_canvas_to_body();
    let msg = tr.t("canvas-import-mermaid-success", &[("count", &made.to_string())]);
    if errors.is_empty() { (ToastKind::Success, msg) } else { (ToastKind::Info, format!("{msg} · {}", errors.join("; "))) }
}

pub(super) fn export_drawio(canvas: &CanvasDocument, tr: &LocaleManager) -> Option<(ToastKind, String)> {
    let save_path = rfd::FileDialog::new()
        .add_filter("Draw.io", &["drawio", "xml"])
        .set_file_name(format!("{}.drawio", canvas.title.replace(' ', "_")))
        .save_file()?;
    Some(match std::fs::write(&save_path, canvas.to_drawio_xml()) {
        Ok(()) => (ToastKind::Success, tr.t("canvas-export-success", &[])),
        Err(e) => (ToastKind::Error, format!("{}: {e}", tr.t("canvas-export-failed", &[]))),
    })
}

/// A Draw.io import: the new document (`None` if empty / failed), the text
/// vertices that became bound blocks, and the toast.
pub(super) type DrawioImport = (Option<CanvasDocument>, Vec<(BlockBinding, String)>, (ToastKind, String));

/// Bound Draw.io import (§Fase 3): "text" vertices become Markdown blocks,
/// every other shape stays diagram-only. `None` when the dialog was cancelled.
pub(super) fn import_drawio(canvas: &CanvasDocument, screen: egui::Vec2, tr: &LocaleManager) -> Option<DrawioImport> {
    let load_path = rfd::FileDialog::new().add_filter("Draw.io", &["drawio", "xml"]).pick_file()?;
    let title = load_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| canvas.title.clone());
    let result = std::fs::read_to_string(&load_path)
        .map_err(anyhow::Error::from)
        .and_then(|xml| canvas::DrawioImporter::from_xml_bound(&title, &xml));
    Some(match result {
        Ok((imported, _)) if imported.elements.is_empty() => {
            (None, Vec::new(), (ToastKind::Error, tr.t("canvas-import-empty", &[])))
        }
        Ok((mut imported, new_blocks)) => {
            let bounds =
                imported.elements.iter().map(|e| e.bounding_rect()).fold(egui::Rect::NOTHING, |a, r| a.union(r));
            imported.viewport = canvas.viewport.clone();
            imported.viewport.fit_rect(bounds, screen);
            let count = imported.elements.len().to_string();
            let bound = new_blocks.len().to_string();
            let message = format!(
                "{} · {}",
                tr.t("canvas-import-success", &[("count", &count)]),
                tr.t("canvas-import-bound", &[("count", &bound)])
            );
            (Some(imported), new_blocks, (ToastKind::Success, message))
        }
        Err(e) => (None, Vec::new(), (ToastKind::Error, format!("{}: {e}", tr.t("canvas-import-failed", &[])))),
    })
}
