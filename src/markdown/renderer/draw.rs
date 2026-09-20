//! Draws one Live-view block (§3.2.1, §3.2.2) in the colours of the active
//! reading theme (§3.2.5): the block's Markdown goes through
//! `egui_commonmark` inside a style scope whose text/strong/link/code
//! colours come from `ThemeColors`; list markers, blockquotes, Obsidian
//! callouts (`> [!type] Title`), checkboxes and rules are drawn here so
//! their accents are themeable too. Mermaid fences draw natively.
//! Callers: `markdown::renderer::render_cached`.

use std::sync::Arc;

use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

use super::transform::{
    image_embed_target, toggle_checklist_line, transform_canvas_code_blocks, transform_inline,
    transform_note_embeds,
};
use super::{EmbedContent, EmbedResolver, RenderCache, RenderOutcome, diagram_key};
use crate::markdown::live_blocks::{self, BlockKind, LiveBlock};
use crate::reading_theme::ThemeColors;

/// Body text size of the Live view (a little larger than UI text, for reading).
pub(super) const BODY_SIZE: f32 = 15.5;
/// `TextStyle::Heading` size in the Live view; `egui_commonmark` scales
/// H1–H6 between this and `BODY_SIZE`.
const HEADING_SIZE: f32 = 30.0;
/// Height of an empty line.
pub(super) use super::BLANK_HEIGHT;
/// Horizontal offset per column of list indentation.
const INDENT_PX_PER_COLUMN: f32 = 7.0;

/// What a block needs from the note and the app to draw itself.
pub(super) struct BlockEnv<'a> {
    pub colors: &'a ThemeColors,
    pub is_resolved: &'a dyn Fn(&str) -> bool,
    pub resolve_embed: &'a EmbedResolver<'a>,
}

/// Callout types understood in `> [!type]`, with the Obsidian aliases:
/// `(type, Material icon, default title)`.
const CALLOUTS: &[(&str, &str, &str)] = {
    use egui_icons::icons::*;
    &[
        ("note", ICON_INFO.codepoint, "Note"),
        ("info", ICON_INFO.codepoint, "Info"),
        ("todo", ICON_CHECK_BOX.codepoint, "Todo"),
        ("abstract", ICON_SUMMARIZE.codepoint, "Abstract"),
        ("summary", ICON_SUMMARIZE.codepoint, "Summary"),
        ("tldr", ICON_SUMMARIZE.codepoint, "TL;DR"),
        ("tip", ICON_LIGHTBULB.codepoint, "Tip"),
        ("hint", ICON_LIGHTBULB.codepoint, "Hint"),
        ("success", ICON_CHECK_CIRCLE.codepoint, "Success"),
        ("check", ICON_CHECK_CIRCLE.codepoint, "Check"),
        ("done", ICON_CHECK_CIRCLE.codepoint, "Done"),
        ("important", ICON_PRIORITY_HIGH.codepoint, "Important"),
        ("example", ICON_LIST.codepoint, "Example"),
        ("question", ICON_HELP.codepoint, "Question"),
        ("help", ICON_HELP.codepoint, "Help"),
        ("faq", ICON_HELP.codepoint, "FAQ"),
        ("warning", ICON_WARNING.codepoint, "Warning"),
        ("attention", ICON_WARNING.codepoint, "Attention"),
        ("caution", ICON_REPORT.codepoint, "Caution"),
        ("danger", ICON_BOLT.codepoint, "Danger"),
        ("error", ICON_ERROR.codepoint, "Error"),
        ("failure", ICON_CANCEL.codepoint, "Failure"),
        ("fail", ICON_CANCEL.codepoint, "Fail"),
        ("missing", ICON_CANCEL.codepoint, "Missing"),
        ("bug", ICON_BUG_REPORT.codepoint, "Bug"),
        ("quote", ICON_FORMAT_QUOTE.codepoint, "Quote"),
        ("cite", ICON_FORMAT_QUOTE.codepoint, "Cite"),
    ]
};

/// Icon and default title of a callout type (`None` = unknown type).
fn callout_style(kind: &str) -> Option<(&'static str, &'static str)> {
    CALLOUTS.iter().find(|(id, _, _)| *id == kind).map(|(_, icon, label)| (*icon, *label))
}

/// Sets the reading colours and typography for one block.
fn apply_block_style(ui: &mut egui::Ui, c: &ThemeColors, kind: BlockKind) {
    let style = ui.style_mut();
    style.interaction.selectable_labels = false;
    style.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(BODY_SIZE));
    style.text_styles.insert(egui::TextStyle::Heading, egui::FontId::proportional(HEADING_SIZE));
    style.spacing.item_spacing.y = 4.0;
    let (text, strong) = match kind {
        BlockKind::Heading(level) => (c.heading(level), c.heading(level)),
        BlockKind::Quote => (c.muted, c.strong),
        _ => (c.text, c.strong),
    };
    let v = &mut style.visuals;
    v.widgets.noninteractive.fg_stroke.color = text.into();
    v.widgets.active.fg_stroke.color = strong.into();
    v.hyperlink_color = c.link.into();
    v.code_bg_color = c.code_bg.into();
    v.weak_text_color = Some(c.muted.into());
    v.widgets.noninteractive.bg_stroke.color = c.table_border.into();
}

/// Draws `block` (whose Markdown is `source`). Returns `true` when the
/// user asked to edit it through the block's own edit button (atomic
/// blocks whose content swallows clicks).
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_block(
    ui: &mut egui::Ui,
    cache: &mut CommonMarkCache,
    index: &mut RenderCache,
    body: &str,
    block: &LiveBlock,
    source: &str,
    env: &BlockEnv<'_>,
    outcome: &mut RenderOutcome,
) -> bool {
    apply_block_style(ui, env.colors, block.kind);
    index.md_runs = 0;
    let indent = match block.kind {
        BlockKind::ListItem | BlockKind::Checklist { .. } | BlockKind::Paragraph | BlockKind::Quote => {
            (block.indent.min(24) as f32) * INDENT_PX_PER_COLUMN
        }
        _ => 0.0,
    };
    let width = ui.available_width();
    let left = ui.cursor().left();
    let top = ui.cursor().top();
    ui.horizontal_top(|ui| {
        ui.add_space(indent);
        ui.vertical(|ui| {
            ui.set_width(width - indent);
            draw_content(ui, cache, index, body, block, source, env, outcome);
        });
    });
    if !block.kind.is_atomic() {
        return false;
    }
    let rect = egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(left + width, ui.cursor().top()));
    edit_button(ui, rect, block.lines.start)
}

#[allow(clippy::too_many_arguments)]
fn draw_content(
    ui: &mut egui::Ui,
    cache: &mut CommonMarkCache,
    index: &mut RenderCache,
    body: &str,
    block: &LiveBlock,
    source: &str,
    env: &BlockEnv<'_>,
    outcome: &mut RenderOutcome,
) {
    let c = env.colors;
    match block.kind {
        BlockKind::Blank => {
            ui.allocate_space(egui::vec2(ui.available_width(), BLANK_HEIGHT));
        }
        BlockKind::Rule => {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), egui::Sense::hover());
            ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.5, c.rule));
        }
        BlockKind::Heading(level) => {
            if level == 1 {
                ui.add_space(4.0);
            }
            markdown(ui, cache, index, &transform_inline(source, env.is_resolved), env);
        }
        BlockKind::Checklist { checked } => {
            let line_idx = block.lines.start;
            // `- [ ] text`: drop the list marker, then the box.
            let text = source.trim_start().get(2..).and_then(|rest| rest.get(3..)).unwrap_or("").trim_start();
            ui.horizontal_top(|ui| {
                if checkbox(ui, checked, c).clicked() {
                    outcome.updated_body = Some(toggle_checklist_line(body, line_idx));
                }
                ui.vertical(|ui| {
                    if checked {
                        ui.visuals_mut().widgets.noninteractive.fg_stroke.color = c.muted.into();
                    }
                    let shown = if checked && !text.is_empty() { format!("~~{text}~~") } else { text.to_string() };
                    markdown(ui, cache, index, &transform_inline(&shown, env.is_resolved), env);
                });
            });
        }
        BlockKind::Quote => {
            let inner: String = source
                .lines()
                .map(|l| {
                    let rest = l.trim_start().strip_prefix('>').unwrap_or(l);
                    rest.strip_prefix(' ').unwrap_or(rest)
                })
                .collect::<Vec<_>>()
                .join("\n");
            let resp = egui::Frame::NONE
                .inner_margin(egui::Margin { left: 14, right: 0, top: 2, bottom: 2 })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let text = transform_inline(&inner, env.is_resolved);
                    if text.trim().is_empty() {
                        ui.add_space(BODY_SIZE);
                    } else {
                        markdown(ui, cache, index, &text, env);
                    }
                })
                .response;
            let r = resp.rect;
            ui.painter().vline(r.left() + 3.0, r.y_range(), egui::Stroke::new(3.0, c.quote_bar));
        }
        BlockKind::ListItem => list_item(ui, cache, index, source, env),
        BlockKind::Callout => {
            let expanded = transform_note_embeds(source, env.resolve_embed, 0);
            callout(ui, cache, index, &expanded, env);
        }
        BlockKind::Mermaid => {
            let fence_line = block.lines.start;
            let inner: Vec<&str> = source.lines().skip(1).collect();
            let mermaid_src = inner[..inner.len().saturating_sub(1)].join("\n");
            let dark = ui.visuals().dark_mode;
            let rendered = Arc::clone(index.diagrams.entry(diagram_key(&mermaid_src, dark)).or_insert_with(|| {
                let glyphs = crate::mermaid::paint::glyph_table(ui.ctx(), &mermaid_src);
                Arc::new(crate::mermaid::render(&mermaid_src, &crate::mermaid::RenderOptions { dark, measure: &glyphs }))
            }));
            render_mermaid(ui, cache, &rendered, &mermaid_src, fence_line, outcome);
        }
        _ => {
            // Embeds (transclusions, sheet previews) and legacy canvas
            // fences expand into more Markdown, drawn as blocks of their own.
            let expanded = transform_note_embeds(&transform_canvas_code_blocks(source), env.resolve_embed, 0);
            if expanded == source {
                render_with_images(ui, cache, index, &transform_inline(source, env.is_resolved), env);
            } else {
                nested(ui, cache, index, &expanded, env);
            }
        }
    }
}

/// Markdown that may hold several blocks (callout content, expanded
/// embeds), drawn block by block so callouts inside it render as callouts.
fn nested(ui: &mut egui::Ui, cache: &mut CommonMarkCache, index: &mut RenderCache, text: &str, env: &BlockEnv<'_>) {
    let lines: Vec<&str> = text.lines().collect();
    for block in live_blocks::split_blocks(text) {
        let source = lines[block.lines.clone()].join("\n");
        match block.kind {
            BlockKind::Callout => callout(ui, cache, index, &source, env),
            BlockKind::Blank => ui.add_space(BLANK_HEIGHT / 2.0),
            _ => render_with_images(ui, cache, index, &transform_inline(&source, env.is_resolved), env),
        }
    }
}

/// `- item` / `3. item` with a themed bullet or number and tight spacing.
fn list_item(ui: &mut egui::Ui, cache: &mut CommonMarkCache, index: &mut RenderCache, source: &str, env: &BlockEnv<'_>) {
    let t = source.trim_start();
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    let (number, rest) = if digits > 0 {
        (Some(&t[..=digits]), t[digits + 1..].trim_start())
    } else {
        (None, t.get(1..).unwrap_or("").trim_start())
    };
    let color: egui::Color32 = env.colors.strong.into();
    let row_height = BODY_SIZE * 1.4;
    ui.horizontal_top(|ui| {
        match number {
            Some(n) => {
                let font = egui::FontId::proportional(BODY_SIZE);
                let galley = ui.painter().layout_no_wrap(n.to_string(), font, color);
                let (rect, _) = ui.allocate_exact_size(egui::vec2((galley.size().x + 6.0).max(18.0), row_height), egui::Sense::hover());
                let pos = egui::pos2(rect.right() - 6.0 - galley.size().x, rect.center().y - galley.size().y / 2.0);
                ui.painter().galley(pos, galley, color);
            }
            None => {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, row_height), egui::Sense::hover());
                ui.painter().circle_filled(egui::pos2(rect.center().x - 2.0, rect.center().y), 2.8, color);
            }
        }
        ui.vertical(|ui| {
            if rest.is_empty() {
                ui.add_space(row_height);
            } else {
                render_with_images(ui, cache, index, &transform_inline(rest, env.is_resolved), env);
            }
        });
    });
}

/// An Obsidian callout: accent bar, tinted background, icon + title (its
/// own Markdown, or the type's name), then the `>` content.
fn callout(ui: &mut egui::Ui, cache: &mut CommonMarkCache, index: &mut RenderCache, source: &str, env: &BlockEnv<'_>) {
    let mut lines = source.lines();
    let head = lines.next().unwrap_or("");
    let head = head.trim_start().strip_prefix('>').unwrap_or(head).trim_start();
    let (kind, title) = head
        .strip_prefix("[!")
        .and_then(|r| r.split_once(']'))
        .map(|(k, t)| (k.trim().to_ascii_lowercase(), t.trim_start_matches(['+', '-']).trim()))
        .unwrap_or_else(|| ("note".to_string(), head));
    let inner: Vec<&str> = lines
        .map(|l| {
            let rest = l.trim_start().strip_prefix('>').unwrap_or(l);
            rest.strip_prefix(' ').unwrap_or(rest)
        })
        .collect();
    let (icon, label) = callout_style(&kind).unwrap_or((egui_icons::icons::ICON_INFO.codepoint, ""));
    let accent: egui::Color32 = env.colors.callout(&kind).into();
    let resp = egui::Frame::NONE
        .fill(accent.gamma_multiply(0.10))
        .corner_radius(6)
        .inner_margin(egui::Margin { left: 14, right: 10, top: 7, bottom: 7 })
        .outer_margin(egui::Margin { left: 0, right: 0, top: 2, bottom: 2 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.label(egui::RichText::new(icon).size(BODY_SIZE + 1.0).color(accent));
                ui.vertical(|ui| {
                    let v = &mut ui.visuals_mut().widgets;
                    v.noninteractive.fg_stroke.color = accent;
                    v.active.fg_stroke.color = accent;
                    let fallback;
                    let title = if title.is_empty() {
                        fallback = if label.is_empty() { kind.clone() } else { label.to_string() };
                        format!("**{fallback}**")
                    } else {
                        format!("**{title}**")
                    };
                    render_with_images(ui, cache, index, &transform_inline(&title, env.is_resolved), env);
                });
            });
            let content = inner.join("\n");
            if !content.trim().is_empty() {
                ui.add_space(2.0);
                nested(ui, cache, index, &content, env);
            }
        })
        .response;
    let r = resp.rect;
    ui.painter().vline(r.left() + 1.5, r.y_range(), egui::Stroke::new(3.0, accent));
}

/// A themed checkbox: accent-filled with a check mark when done.
fn checkbox(ui: &mut egui::Ui, checked: bool, c: &ThemeColors) -> egui::Response {
    let size = BODY_SIZE + 1.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size + 6.0, size + 4.0), egui::Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let bx = egui::Rect::from_center_size(rect.center() - egui::vec2(3.0, 0.0), egui::vec2(size - 2.0, size - 2.0));
    let accent: egui::Color32 = c.checkbox.into();
    let painter = ui.painter();
    if checked {
        painter.rect_filled(bx, 4.0, accent);
        let pts = vec![
            egui::pos2(bx.left() + bx.width() * 0.24, bx.center().y),
            egui::pos2(bx.left() + bx.width() * 0.43, bx.bottom() - bx.height() * 0.26),
            egui::pos2(bx.right() - bx.width() * 0.2, bx.top() + bx.height() * 0.27),
        ];
        painter.line(pts, egui::Stroke::new(2.0, egui::Color32::WHITE));
    } else {
        let stroke = if resp.hovered() { accent } else { egui::Color32::from(c.muted) };
        painter.rect_stroke(bx, 4.0, egui::Stroke::new(1.5, stroke), egui::StrokeKind::Inside);
    }
    resp
}

/// The ✎ button shown over an atomic block while hovered, since code
/// blocks, tables and diagrams take clicks for themselves.
fn edit_button(ui: &mut egui::Ui, block_rect: egui::Rect, line: usize) -> bool {
    if !ui.rect_contains_pointer(block_rect) {
        return false;
    }
    let center = egui::pos2(block_rect.right() - 16.0, block_rect.top() + 14.0);
    let rect = egui::Rect::from_center_size(center, egui::vec2(24.0, 24.0));
    let resp = ui
        .interact(rect, ui.id().with(("live_edit_btn", line)), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Markdown");
    let p = crate::ui::pal();
    let fill = if resp.hovered() { p.hover } else { p.card };
    ui.painter().rect(rect, 6.0, fill, egui::Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        egui_icons::icons::ICON_EDIT.codepoint,
        egui::FontId::proportional(14.0),
        p.text_dim,
    );
    resp.clicked()
}

fn markdown(ui: &mut egui::Ui, cache: &mut CommonMarkCache, index: &mut RenderCache, text: &str, env: &BlockEnv<'_>) {
    render_with_images(ui, cache, index, text, env);
}

/// Renders `text`, drawing image embeds (`![[file.png]]` on a line of
/// their own) as real images between the Markdown runs around them.
fn render_with_images(ui: &mut egui::Ui, cache: &mut CommonMarkCache, index: &mut RenderCache, text: &str, env: &BlockEnv<'_>) {
    let mut run = String::new();
    let mut in_fence = false;
    // Each run in its own id scope: `egui_commonmark` names tables
    // `_table/0`, `_table/1`, … per `show`, so two runs in one `Ui`
    // (nested embeds, callouts) would give their tables the same `Grid` id.
    let flush = |ui: &mut egui::Ui, cache: &mut CommonMarkCache, index: &mut RenderCache, run: &mut String| {
        if !run.trim().is_empty() {
            index.md_runs += 1;
            ui.push_id(("md_run", index.md_runs), |ui| {
                CommonMarkViewer::new()
                    .enable_scroll_to_heading(true)
                    .render_math_fn(Some(&render_math))
                    .show(ui, cache, run);
            });
        }
        run.clear();
    };
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
        }
        if !in_fence
            && let Some(name) = image_embed_target(t)
            && let Some(EmbedContent::Image(path)) = (env.resolve_embed)(name)
        {
            flush(ui, cache, index, &mut run);
            draw_image_embed(ui, index, &path, name);
            continue;
        }
        run.push_str(line);
        run.push('\n');
    }
    flush(ui, cache, index, &mut run);
}

/// `egui_commonmark` math hook (§Fase 1.8): `$x^2$` inline as italic
/// Unicode, `$$…$$` centered and larger, via `markdown::math`.
fn render_math(ui: &mut egui::Ui, tex: &str, inline: bool) {
    let text = crate::markdown::math::to_unicode(tex);
    let color = ui.visuals().text_color();
    if inline {
        ui.label(egui::RichText::new(text).italics().color(color));
    } else {
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(text).size(crate::ui::theme::TEXT_LG).italics().color(color));
            });
        });
    }
}

fn draw_image_embed(ui: &mut egui::Ui, index: &mut RenderCache, path: &std::path::Path, name: &str) {
    let texture = index
        .textures
        .entry(path.to_path_buf())
        .or_insert_with(|| {
            let decoded = image::open(path)
                .map_err(|e| log::warn!("renderer: cannot decode {}: {e}", path.display()))
                .ok()?;
            let rgba = decoded.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
            Some(ui.ctx().load_texture(format!("embed:{}", path.display()), color, egui::TextureOptions::LINEAR))
        })
        .clone();
    match texture {
        Some(tex) => {
            ui.add_space(4.0);
            ui.add(egui::Image::from_texture(&tex).max_width(ui.available_width()).corner_radius(4.0));
            ui.add_space(4.0);
        }
        None => {
            ui.label(egui::RichText::new(format!("🖼 {name}")).italics());
        }
    }
}

/// Draws a Mermaid diagram (or, when it can't be drawn, its source as a
/// code block) followed by its error diagnostics, and routes clicks:
/// `[[Note]]`/bare targets become wikilink navigation, URLs open, and a
/// node without a link reports its source line (to edit it).
fn render_mermaid(
    ui: &mut egui::Ui,
    cache: &mut CommonMarkCache,
    rendered: &crate::mermaid::Rendered,
    source: &str,
    fence_line: usize,
    outcome: &mut RenderOutcome,
) {
    match &rendered.scene {
        Some(scene) => {
            let act = ui.push_id(("mermaid", fence_line), |ui| crate::mermaid::paint::show(ui, scene)).inner;
            if let Some(link) = act.clicked_link {
                let target = link.trim();
                if let Some(inner) = target.strip_prefix("[[").and_then(|t| t.strip_suffix("]]")) {
                    outcome.clicked_wikilink = Some(inner.split('|').next().unwrap_or(inner).to_string());
                } else if target.contains("://") || target.starts_with("mailto:") {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(target));
                } else if !target.starts_with('#') {
                    outcome.clicked_wikilink = Some(target.to_string());
                }
            }
            if let Some(line) = act.clicked_line {
                outcome.clicked_source_line = Some(fence_line + line);
            }
        }
        None => {
            CommonMarkViewer::new().show(ui, cache, &format!("```mermaid\n{source}\n```"));
        }
    }
    let error_color = ui.visuals().error_fg_color;
    for d in rendered.diagnostics.iter().filter(|d| d.is_error()).take(5) {
        ui.label(egui::RichText::new(format!("⚠ {}:{}  {}", d.line, d.col, d.message)).small().color(error_color));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callout_types_have_icons_and_titles() {
        assert_eq!(callout_style("danger").map(|(_, l)| l), Some("Danger"));
        assert!(callout_style("tidak-ada").is_none());
        assert!(CALLOUTS.iter().all(|(id, _, _)| id.chars().all(|c| c.is_ascii_lowercase())));
    }
}
