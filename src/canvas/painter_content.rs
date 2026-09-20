//! Content of text boxes on the canvas (§3.9.2): Markdown drawn "lite"
//! (headings larger and semibold, list bullets and checkboxes, quotes,
//! anchors and emphasis markers hidden), tables as a grid, code fences in
//! monospace, and ```` ```mermaid ```` fences as the real diagram scaled to
//! fit — so a section box reads like the note, and an `erDiagram` box can
//! sit next to the prose that explains it. Diagram parse+layout is cached
//! per source. Callers: `canvas::painter`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::text::LayoutJob;
use egui::{Color32, FontId, Painter, Pos2, Rect, Stroke, TextFormat, Vec2};

use crate::markdown::blocks::split_anchor;
use crate::markdown::sections::heading_level;
use crate::mermaid::{self, Rendered};
use crate::canvas::mermaid_export::mermaid_source;
use crate::ui::theme;

/// Base text size in world units.
const BASE_FONT: f32 = 13.0;
/// Most diagrams kept in the render cache.
const DIAGRAM_CACHE_LIMIT: usize = 64;

thread_local! {
    static DIAGRAMS: RefCell<HashMap<(u64, bool), Arc<Rendered>>> = RefCell::new(HashMap::new());
}

fn quantize(size: f32) -> f32 {
    if size <= 24.0 { size.round().max(1.0) } else { (size / 4.0).round() * 4.0 }
}

/// Draws `text` (a bound Markdown segment or a note's text) inside the
/// screen rect `rect`. `zoom` is the viewport zoom.
pub fn draw_box_content(painter: &Painter, rect: Rect, text: &str, zoom: f32, color: Color32, is_dark: bool) {
    if rect.width() < 4.0 || rect.height() < 4.0 || BASE_FONT * zoom < 4.0 {
        return;
    }
    let clipped = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim_start();
    if let Some(source) = mermaid_source(text) {
        draw_diagram(&clipped, rect, &source, zoom, color, is_dark);
    } else if first.starts_with('|') {
        draw_table(&clipped, rect, text, zoom, color);
    } else if first.starts_with("```") || first.starts_with("~~~") {
        let font = FontId::monospace(quantize((BASE_FONT - 1.0) * zoom));
        let galley = clipped.layout(text.to_string(), font, color, rect.width());
        clipped.galley(rect.min, galley, color);
    } else {
        let job = markdown_job(text, zoom, color, rect.width());
        let galley = clipped.layout_job(job);
        clipped.galley(rect.min, galley, color);
    }
}

/// Parsed + laid out diagram for `source`, cached.
fn rendered(ctx: &egui::Context, source: &str, dark: bool) -> Arc<Rendered> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut h);
    let key = (h.finish(), dark);
    if let Some(hit) = DIAGRAMS.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }
    let glyphs = mermaid::paint::glyph_table(ctx, source);
    let out = Arc::new(mermaid::render(source, &mermaid::RenderOptions { dark, measure: &glyphs }));
    DIAGRAMS.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= DIAGRAM_CACHE_LIMIT {
            c.clear();
        }
        c.insert(key, out.clone());
    });
    out
}

fn draw_diagram(painter: &Painter, rect: Rect, source: &str, zoom: f32, color: Color32, dark: bool) {
    let r = rendered(painter.ctx(), source, dark);
    match &r.scene {
        Some(scene) if scene.width > 0.0 && scene.height > 0.0 => {
            // Fit, but never draw larger than the diagram's natural size at this zoom.
            let scale = (rect.width() / scene.width).min(rect.height() / scene.height).min(zoom);
            let size = Vec2::new(scene.width, scene.height) * scale;
            let origin = Pos2::new(rect.center().x - size.x / 2.0, rect.min.y);
            mermaid::paint::paint(painter, scene, origin, scale);
        }
        _ => {
            let msg = r
                .diagnostics
                .first()
                .map(|d| format!("mermaid · {}:{} {}", d.line, d.col, d.message))
                .unwrap_or_else(|| format!("mermaid · {}", r.kind.name()));
            let font = FontId::monospace(quantize((BASE_FONT - 1.0) * zoom));
            let galley = painter.layout(format!("{msg}\n\n{source}"), font, color, rect.width());
            painter.galley(rect.min, galley, color);
        }
    }
}

/// Cells of a table row (`| a | b |` → `["a", "b"]`).
fn cells(row: &str) -> Vec<String> {
    let t = row.trim().trim_start_matches('|').trim_end_matches('|');
    t.split('|').map(|c| c.trim().replace("**", "")).collect()
}

fn is_separator(row: &str) -> bool {
    let t = row.trim();
    !t.is_empty() && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

fn draw_table(painter: &Painter, rect: Rect, text: &str, zoom: f32, color: Color32) {
    let rows: Vec<Vec<String>> = text
        .lines()
        .filter(|l| l.trim_start().starts_with('|') && !is_separator(l))
        .map(cells)
        .collect();
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    if cols == 0 {
        return;
    }
    let font = quantize(BASE_FONT * zoom * 0.95);
    let row_h = font * 1.7;
    let col_w = rect.width() / cols as f32;
    let line = Stroke::new(1.0, color.gamma_multiply(0.35));
    for (r, row) in rows.iter().enumerate() {
        let y = rect.min.y + r as f32 * row_h;
        if y > rect.max.y {
            break;
        }
        if r == 0 {
            painter.rect_filled(
                Rect::from_min_size(Pos2::new(rect.min.x, y), Vec2::new(rect.width(), row_h)),
                2.0,
                color.gamma_multiply(0.08),
            );
        }
        for (c, cell) in row.iter().enumerate() {
            let cell_rect = Rect::from_min_size(Pos2::new(rect.min.x + c as f32 * col_w, y), Vec2::new(col_w, row_h));
            let fid = if r == 0 { theme::semibold(font) } else { FontId::proportional(font) };
            let galley = painter.layout_no_wrap(cell.clone(), fid, color);
            painter
                .with_clip_rect(cell_rect.shrink(2.0).intersect(painter.clip_rect()))
                .galley(cell_rect.min + Vec2::new(6.0 * zoom, (row_h - galley.size().y) / 2.0), galley, color);
        }
        painter.line_segment([Pos2::new(rect.min.x, y + row_h), Pos2::new(rect.max.x, y + row_h)], line);
    }
    let bottom = (rect.min.y + rows.len() as f32 * row_h).min(rect.max.y);
    for c in 1..cols {
        let x = rect.min.x + c as f32 * col_w;
        painter.line_segment([Pos2::new(x, rect.min.y), Pos2::new(x, bottom)], line);
    }
}

/// `[[target|alias]]` → `alias`, `[[target]]` → `target`, `**`/`__`/`` ` `` removed.
fn inline_plain(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find("[[") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("]]") {
            Some(end) => {
                let inner = &after[..end];
                let shown = inner.rsplit('|').next().unwrap_or(inner);
                out.push_str(shown.split('#').next().filter(|s| !s.is_empty()).unwrap_or(shown));
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.replace("**", "").replace("__", "").replace('`', "")
}

/// Markdown-lite layout of `text`.
fn markdown_job(text: &str, zoom: f32, color: Color32, width: f32) -> LayoutJob {
    let mut job = LayoutJob {
        wrap: egui::text::TextWrapping { max_width: width, ..Default::default() },
        ..Default::default()
    };
    let base = BASE_FONT * zoom;
    let faint = color.gamma_multiply(0.7);
    let mut first = true;
    for raw in text.lines() {
        let line = split_anchor(raw).0;
        if !first {
            job.append("\n", 0.0, TextFormat::simple(FontId::proportional(quantize(base * 0.6)), color));
        }
        first = false;
        let t = line.trim_start();
        if t.is_empty() {
            continue;
        }
        let (content, format) = if let Some(level) = heading_level(line) {
            let scale = match level {
                1 => 1.45,
                2 => 1.25,
                _ => 1.1,
            };
            let content = t.trim_start_matches('#').trim().to_string();
            (content, TextFormat::simple(theme::semibold(quantize(base * scale)), color))
        } else {
            let indent = "  ".repeat((line.len() - t.len()) / 2);
            let body = if let Some(r) = t.strip_prefix("- [ ] ").or_else(|| t.strip_prefix("* [ ] ")) {
                format!("{indent}☐ {r}")
            } else if let Some(r) = ["- [x] ", "- [X] ", "* [x] "].iter().find_map(|m| t.strip_prefix(m)) {
                format!("{indent}☑ {r}")
            } else if let Some(r) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ ")) {
                format!("{indent}• {r}")
            } else if let Some(r) = t.strip_prefix('>') {
                let mut f = TextFormat::simple(FontId::proportional(quantize(base)), faint);
                f.italics = true;
                job.append(&format!("▍{}", inline_plain(r.trim_start())), 0.0, f);
                continue;
            } else {
                format!("{indent}{t}")
            };
            (body, TextFormat::simple(FontId::proportional(quantize(base)), color))
        };
        job.append(&inline_plain(&content), 0.0, format);
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_plain_strips_markup() {
        assert_eq!(inline_plain("Lihat [[Catatan#^ab|ini]] dan **tebal** `kode`"), "Lihat ini dan tebal kode");
        assert_eq!(inline_plain("[[Catatan]]"), "Catatan");
    }

    #[test]
    fn table_cells_skip_separator() {
        assert_eq!(cells("| a | **b** |"), vec!["a", "b"]);
        assert!(is_separator("|---|:--:|"));
        assert!(!is_separator("| a |"));
    }
}
