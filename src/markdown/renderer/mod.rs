//! Renders a note body to `egui` widgets via `egui_commonmark` (§3.2.2):
//! wikilinks become clickable links, checklists become interactive
//! checkboxes, headings get auto-generated anchor ids for the Outline
//! panel, and GitHub-style callouts (`> [!note]`, `> [!warning]`, ...)
//! render through `egui_commonmark`'s built-in alert support. Callers:
//! `app.rs` (via `markdown::editor::MarkdownEditor::render`/`outline`,
//! which own a `RenderCache` per open note).
//!
//! `RenderCache` + `render_cached` (§Fase 10, §6 risk 5 mitigation) are the
//! long-document performance pass: `render`/`headings` alone re-walk and
//! re-parse the *entire* body on every call, and `egui`'s immediate-mode
//! model means that call happens on every single repaint the editor panel
//! gets (mouse move, cursor blink, scrollbar drag) — not just on actual
//! edits. `RenderCache` memoizes the body→segments/wikilink-targets/outline
//! parse behind a content hash, and `render_cached` additionally
//! virtualizes: segments outside the visible scroll viewport (± a buffer)
//! only reserve blank vertical space (`ui.allocate_space`) instead of
//! paying for a full `CommonMarkViewer::show` markdown parse+layout. This
//! is the practical analogue of the roadmap's "dirty region tracking" for
//! an immediate-mode renderer that has no persistent widget tree to diff
//! against — recompute-on-change plus skip-what's-offscreen, rather than
//! true incremental AST patching.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

use super::blocks;
use super::wikilink;

/// A heading extracted from a note body, with a slug suitable for
/// `egui_commonmark`'s `#slug` scroll-to-heading links — backs the
/// Outline/TOC panel (§3.2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub title: String,
    pub slug: String,
}

/// What happened during a `render()` call that the caller (`app.rs`) needs
/// to act on.
/// What an `![[embed]]` resolves to (§Fase 1.3): another note's text
/// (transclusion), an image file in the vault, or a CSV/XLSX sheet shown
/// as a preview table (§3.8.3).
#[derive(Debug, Clone)]
pub enum EmbedContent {
    Note { title: String, body: String },
    Image(std::path::PathBuf),
    Sheet(std::path::PathBuf),
}

/// Looks up `![[target]]` (title/file name, `#heading` removed by the
/// caller if wanted). `None` = unresolved.
pub type EmbedResolver<'a> = dyn Fn(&str) -> Option<EmbedContent> + 'a;

/// Nesting depth allowed for note transclusion.
const MAX_EMBED_DEPTH: usize = 3;

#[derive(Debug, Default)]
pub struct RenderOutcome {
    /// A checklist checkbox was toggled; this is the note body with that
    /// line's `[ ]`/`[x]` marker flipped.
    pub updated_body: Option<String>,
    /// The user clicked a `[[wikilink]]`; this is its reference
    /// (`Title` or `Title#Heading`, alias removed).
    pub clicked_wikilink: Option<String>,
    /// The user clicked an inline `#tag` (without the `#`).
    pub clicked_tag: Option<String>,
    /// The user clicked a Mermaid node without a link: its 0-based line
    /// in the note body (for jumping to the diagram source).
    pub clicked_source_line: Option<usize>,
}

/// Render `body` into `ui`, always fully (no memoization, no
/// virtualization) — a thin wrapper over `render_cached` with a throwaway
/// `RenderCache` and an unbounded viewport. Kept for any caller without a
/// per-note `RenderCache` to reuse across frames; `app.rs`'s note editor
/// calls `render_cached` (via `MarkdownEditor::render`) directly so repeated
/// frames actually benefit from the memoization (§Fase 10).
pub fn render(ui: &mut egui::Ui, cache: &mut CommonMarkCache, body: &str) -> RenderOutcome {
    let mut index = RenderCache::default();
    render_cached(ui, cache, &mut index, body, egui::Rect::EVERYTHING, &|_| true, &|_| None)
}

/// Extra content-space padding rendered above/below the visible viewport
/// (§6 risk 5 "virtualized scrolling"), so a segment already has its real
/// layout by the time it scrolls fully into view instead of popping in a
/// frame late.
const VIRTUALIZATION_BUFFER: f32 = 600.0;

/// Memoized parse of a note body, owned by the caller (`MarkdownEditor`)
/// across frames and invalidated (`invalidate`) whenever the body actually
/// changes. See the module doc comment for why this exists.
#[derive(Default)]
pub struct RenderCache {
    body_hash: Option<u64>,
    segments: Vec<Segment>,
    wikilink_targets: Vec<String>,
    /// Inline `#tag`s, for click hooks.
    tag_targets: Vec<String>,
    /// A block id (`^id`) the rendered view should scroll to on the next
    /// frame that draws it — set by `MarkdownEditor::scroll_to_block`.
    pub scroll_to_block: Option<String>,
    /// Decoded image embeds, by file path (`None` = failed to decode).
    textures: HashMap<std::path::PathBuf, Option<egui::TextureHandle>>,
    /// One estimated-then-measured height per segment, content-space
    /// pixels, parallel to `segments`. Drives `visible_segment_range`.
    heights: Vec<f32>,
    outline: Vec<Heading>,
    /// Laid-out Mermaid diagrams keyed by `diagram_key(source, dark)`:
    /// parse + layout run once per distinct source, not per frame (§3.7).
    diagrams: HashMap<u64, Arc<crate::mermaid::Rendered>>,
}

impl RenderCache {
    /// Drops the memoized parse — call whenever the underlying body may
    /// have changed (`MarkdownEditor::set_body`/`undo`/`redo`).
    pub fn invalidate(&mut self) {
        self.body_hash = None;
    }

    /// Re-parses `body` into `segments`/`wikilink_targets`/`outline` only
    /// if it differs from the last body this cache was built from —
    /// otherwise a no-op, so a frame where nothing changed doesn't re-walk
    /// the whole document just because `egui` repainted for an unrelated
    /// reason (cursor blink, mouse move elsewhere in the window, ...).
    fn ensure_fresh(&mut self, body: &str) {
        let hash = hash_body(body);
        if self.body_hash == Some(hash) {
            return;
        }
        self.segments = segment_lines(body);
        let mut references: Vec<String> = wikilink::parse_wikilinks(body)
            .into_iter()
            .map(|o| o.link.reference())
            .collect();
        references.sort_unstable();
        references.dedup();
        self.wikilink_targets = references;
        self.tag_targets = crate::notes::tags::inline_tags(body);
        self.heights = self.segments.iter().map(estimate_height).collect();
        self.outline = headings(body);
        // Keep only diagrams still present (either theme).
        let live: Vec<u64> = self
            .segments
            .iter()
            .filter_map(|s| match s {
                Segment::Mermaid { source, .. } => Some([diagram_key(source, false), diagram_key(source, true)]),
                _ => None,
            })
            .flatten()
            .collect();
        self.diagrams.retain(|k, _| live.contains(k));
        self.body_hash = Some(hash);
    }

    /// The heading outline for `body` (Outline side panel), recomputed only
    /// when `body` changed since the last call on this cache.
    pub fn outline(&mut self, body: &str) -> &[Heading] {
        self.ensure_fresh(body);
        &self.outline
    }
}

fn diagram_key(source: &str, dark: bool) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    dark.hash(&mut hasher);
    hasher.finish()
}

fn hash_body(body: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    body.hash(&mut hasher);
    hasher.finish()
}

/// Rough pre-render height estimate for a segment that hasn't been measured
/// yet, so the first frame after opening/editing a long note can already
/// virtualize instead of being forced to fully render every segment once
/// just to learn its height. Deliberately coarse — `render_cached`
/// overwrites it with the real measured height the moment a segment is
/// actually rendered (same idea as a browser's pre-reflow layout guess).
fn estimate_height(segment: &Segment) -> f32 {
    const LINE_HEIGHT: f32 = 22.0;
    let lines = match segment {
        Segment::Markdown(text) => text.lines().count().max(1),
        Segment::Checklist { .. } => 1,
        Segment::Mermaid { .. } => return 320.0,
    };
    lines as f32 * LINE_HEIGHT
}

/// Given cumulative segment `heights` (content-space) and the visible
/// viewport `[viewport_top, viewport_bottom]` (same coordinate space, from
/// `egui::ScrollArea::show_viewport`), returns the index range of segments
/// that overlap the viewport padded by `buffer` on both sides. Pure and
/// independent of `egui` so it's directly unit-testable.
fn visible_segment_range(heights: &[f32], viewport_top: f32, viewport_bottom: f32, buffer: f32) -> Range<usize> {
    let lo = viewport_top - buffer;
    let hi = viewport_bottom + buffer;
    let mut y = 0.0;
    let mut start = heights.len();
    let mut end = 0;
    for (i, h) in heights.iter().enumerate() {
        let seg_top = y;
        let seg_bottom = y + h;
        if seg_bottom >= lo && seg_top <= hi {
            start = start.min(i);
            end = i + 1;
        }
        y = seg_bottom;
    }
    if start >= end { 0..0 } else { start..end }
}

/// Render `body` into `ui`, memoized against `index` and virtualized
/// against `viewport` (content-space, from `ScrollArea::show_viewport`):
/// segments outside `viewport` (± a buffer) only reserve blank vertical
/// space instead of paying for a full markdown parse+layout (§Fase 10, §6
/// risk 5). Returns any checklist toggle or wikilink click that occurred
/// this frame so the caller can persist/navigate — identical contract to
/// `render`. `is_resolved(target)` tells whether a link target exists;
/// unresolved links render in italics (clicking one creates the note).
#[allow(clippy::too_many_arguments)]
pub fn render_cached(
    ui: &mut egui::Ui,
    cache: &mut CommonMarkCache,
    index: &mut RenderCache,
    body: &str,
    viewport: egui::Rect,
    is_resolved: &dyn Fn(&str) -> bool,
    resolve_embed: &EmbedResolver<'_>,
) -> RenderOutcome {
    index.ensure_fresh(body);
    let mut outcome = RenderOutcome::default();

    // Re-register link hooks for this frame's wikilinks so egui_commonmark
    // intercepts clicks instead of trying to open them as external URLs
    // (egui_commonmark's `CommonMarkCache::add_link_hook`). Cleared and
    // rebuilt every frame to avoid unbounded growth as the user visits
    // different notes.
    cache.link_hooks_clear();
    for title in &index.wikilink_targets {
        cache.add_link_hook(wikilink_destination(title));
    }
    for tag in &index.tag_targets {
        cache.add_link_hook(tag_destination(tag));
    }
    let scroll_target = index.scroll_to_block.take();

    let visible = visible_segment_range(&index.heights, viewport.top(), viewport.bottom(), VIRTUALIZATION_BUFFER);

    for i in 0..index.segments.len() {
        let top = ui.cursor().top();
        if visible.contains(&i) {
            match &index.segments[i] {
                Segment::Markdown(text) => {
                    let canvas_processed = transform_canvas_code_blocks(text);
                    let embedded = transform_note_embeds(&canvas_processed, resolve_embed, 0);
                    let transformed = transform_inline(&embedded, is_resolved);
                    render_with_images(ui, cache, index, &transformed, resolve_embed);
                }
                Segment::Mermaid { line_idx, source } => {
                    let (line_idx, source) = (*line_idx, source.clone());
                    let dark = ui.visuals().dark_mode;
                    let rendered = Arc::clone(index.diagrams.entry(diagram_key(&source, dark)).or_insert_with(|| {
                        let glyphs = crate::mermaid::paint::glyph_table(ui.ctx(), &source);
                        Arc::new(crate::mermaid::render(
                            &source,
                            &crate::mermaid::RenderOptions { dark, measure: &glyphs },
                        ))
                    }));
                    render_mermaid(ui, cache, &rendered, &source, line_idx, &mut outcome);
                }
                Segment::Checklist { line_idx, checked, text } => {
                    let line_idx = *line_idx;
                    let is_checked_initially = *checked;
                    let canvas_processed = transform_canvas_code_blocks(text);
                    let transformed = transform_inline(&canvas_processed, is_resolved);
                    ui.horizontal(|ui| {
                        let mut is_checked = is_checked_initially;
                        if ui.checkbox(&mut is_checked, "").changed() {
                            outcome.updated_body = Some(toggle_checklist_line(body, line_idx));
                        }
                        CommonMarkViewer::new()
                            .render_math_fn(Some(&render_math))
                            .show(ui, cache, &transformed);
                    });
                }
            }
            // `CommonMarkViewer::show` resets all link hooks to `false` at
            // the *start* of the next call (`prepare_show`), so a click
            // must be read back right after the segment that produced it,
            // not once at the end of the whole document.
            capture_clicked_wikilink(cache, &index.wikilink_targets, &mut outcome);
            capture_clicked_tag(cache, &index.tag_targets, &mut outcome);
        } else {
            ui.allocate_space(egui::vec2(ui.available_width(), index.heights[i]));
        }
        let bottom = ui.cursor().top();
        index.heights[i] = (bottom - top).max(1.0);
        if let Some(id) = scroll_target.as_deref()
            && segment_has_anchor(&index.segments[i], id)
        {
            let rect = egui::Rect::from_min_max(
                egui::pos2(ui.min_rect().left(), top),
                egui::pos2(ui.min_rect().right(), bottom),
            );
            ui.scroll_to_rect(rect, Some(egui::Align::TOP));
        }
    }

    outcome
}

/// Renders `text`, drawing image embeds (`![[file.png]]` on a line of
/// their own) as real images between the Markdown runs around them.
fn render_with_images(
    ui: &mut egui::Ui,
    cache: &mut CommonMarkCache,
    index: &mut RenderCache,
    text: &str,
    resolve_embed: &EmbedResolver<'_>,
) {
    let mut run = String::new();
    let mut in_fence = false;
    let flush = |ui: &mut egui::Ui, cache: &mut CommonMarkCache, run: &mut String| {
        if !run.trim().is_empty() {
            CommonMarkViewer::new()
                .enable_scroll_to_heading(true)
                .render_math_fn(Some(&render_math))
                .show(ui, cache, run);
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
            && let Some(EmbedContent::Image(path)) = resolve_embed(name)
        {
            flush(ui, cache, &mut run);
            draw_image_embed(ui, index, &path, name);
            continue;
        }
        run.push_str(line);
        run.push('\n');
    }
    flush(ui, cache, &mut run);
}

/// `egui_commonmark` math hook (§Fase 1.8): `$x^2$` inline as italic
/// Unicode, `$$…$$` centered and larger, via `markdown::math`.
fn render_math(ui: &mut egui::Ui, tex: &str, inline: bool) {
    let text = super::math::to_unicode(tex);
    let p = crate::ui::pal();
    if inline {
        ui.label(egui::RichText::new(text).italics().color(p.text));
    } else {
        egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(8, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(text)
                            .size(crate::ui::theme::TEXT_LG)
                            .italics()
                            .color(p.text),
                    );
                });
            });
    }
}

/// `![[photo.png]]` (optionally with `|width`) alone on a line.
fn image_embed_target(line: &str) -> Option<&str> {
    let inner = line.strip_prefix("![[")?.strip_suffix("]]")?;
    let name = inner.split('|').next()?.trim();
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp").then_some(name)
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
            Some(ui.ctx().load_texture(
                format!("embed:{}", path.display()),
                color,
                egui::TextureOptions::LINEAR,
            ))
        })
        .clone();
    match texture {
        Some(tex) => {
            ui.add_space(4.0);
            ui.add(
                egui::Image::from_texture(&tex)
                    .max_width(ui.available_width())
                    .corner_radius(4.0),
            );
            ui.add_space(4.0);
        }
        None => {
            ui.label(egui::RichText::new(format!("🖼 {name}")).italics());
        }
    }
}

/// Replaces `![[Note]]` / `![[Note#Heading]]` lines with the target's
/// text as a quote-style callout (Obsidian transclusion), recursively up
/// to `MAX_EMBED_DEPTH`. Unresolved embeds and non-note targets are left
/// for `transform_wikilinks` / `render_with_images`.
fn transform_note_embeds(text: &str, resolve_embed: &EmbedResolver<'_>, depth: usize) -> String {
    if !text.contains("![[") || depth >= MAX_EMBED_DEPTH {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut in_fence = false;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        let t = content.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
        }
        let embedded = if in_fence {
            None
        } else {
            t.strip_prefix("![[")
                .and_then(|r| r.strip_suffix("]]"))
                .and_then(|inner| {
                    let link = wikilink::WikiLink::parse(inner);
                    if link.is_pdf() || image_embed_target(t).is_some() {
                        return None;
                    }
                    match resolve_embed(&link.target)? {
                        EmbedContent::Note { title, body } => {
                            let section = match &link.heading {
                                Some(h) => heading_section(&body, h).unwrap_or(body),
                                None => body,
                            };
                            let nested = transform_note_embeds(
                                &blocks::strip_anchors(&section),
                                resolve_embed,
                                depth + 1,
                            );
                            let mut quoted = format!("> [!quote] [[{title}]]\n");
                            for l in nested.lines() {
                                quoted.push_str("> ");
                                quoted.push_str(l);
                                quoted.push('\n');
                            }
                            Some(quoted)
                        }
                        EmbedContent::Sheet(path) => {
                            Some(super::sheet_embed::preview_markdown(&path, &link.target))
                        }
                        EmbedContent::Image(_) => None,
                    }
                })
        };
        match embedded {
            Some(q) => out.push_str(&q),
            None => out.push_str(line),
        }
    }
    out
}

/// The lines under heading `title` (until the next heading of the same
/// or higher level), for `![[Note#Heading]]`.
fn heading_section(body: &str, title: &str) -> Option<String> {
    let wanted = slugify(title);
    let mut level = 0;
    let mut out: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in body.lines() {
        if let Some((l, t)) = parse_heading(line.trim_start()) {
            if inside && l <= level {
                break;
            }
            if !inside && slugify(&t) == wanted {
                inside = true;
                level = l;
                out.push(line);
                continue;
            }
        }
        if inside {
            out.push(line);
        }
    }
    inside.then(|| out.join("\n"))
}

fn segment_has_anchor(segment: &Segment, id: &str) -> bool {
    let text = match segment {
        Segment::Markdown(t) => t.as_str(),
        Segment::Checklist { text, .. } => text.as_str(),
        Segment::Mermaid { .. } => return false,
    };
    text.lines().any(|l| blocks::line_has_anchor(l, id))
}

/// Draws a Mermaid diagram (or, when it can't be drawn, its source as a
/// code block) followed by its error diagnostics, and routes clicks:
/// `[[Note]]`/bare targets become wikilink navigation, URLs open.
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

fn capture_clicked_tag(cache: &CommonMarkCache, tags: &[String], outcome: &mut RenderOutcome) {
    if outcome.clicked_tag.is_some() {
        return;
    }
    for tag in tags {
        if cache.get_link_hook(&tag_destination(tag)) == Some(true) {
            outcome.clicked_tag = Some(tag.clone());
            return;
        }
    }
}

fn tag_destination(tag: &str) -> String {
    format!("tag:{tag}")
}

/// Everything inline that CommonMark doesn't know: wikilinks, `#tag`s and
/// block anchors (hidden, as in Obsidian).
fn transform_inline(text: &str, is_resolved: &dyn Fn(&str) -> bool) -> String {
    let stripped = blocks::strip_anchors(text);
    let linked = transform_wikilinks(&stripped, is_resolved);
    transform_tags(&linked)
}

/// Rewrites inline `#tag`s into `[#tag](<tag:tag>)` links (fence- and
/// heading-aware, mirroring `notes::tags::inline_tags`).
fn transform_tags(text: &str) -> String {
    if !text.contains('#') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 32);
    let mut in_fence = false;
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            out.push_str(line);
            continue;
        }
        if in_fence || (t.starts_with('#') && t.chars().find(|c| *c != '#') == Some(' ')) {
            out.push_str(line);
            continue;
        }
        let tags = crate::notes::tags::inline_tags(line);
        if tags.is_empty() {
            out.push_str(line);
            continue;
        }
        // Replace longest tags first so `#a/b` isn't clobbered by `#a`.
        let mut sorted = tags.clone();
        sorted.sort_by_key(|t| std::cmp::Reverse(t.len()));
        let mut rewritten = line.to_string();
        for tag in sorted {
            let needle = format!("#{tag}");
            let mut result = String::with_capacity(rewritten.len());
            let mut rest = rewritten.as_str();
            let mut in_code = false;
            let mut in_link = false;
            while let Some(pos) = rest.find(&needle) {
                let (before, after) = rest.split_at(pos);
                in_code ^= before.matches('`').count() % 2 == 1;
                in_link ^= before.matches("](<").count() != before.matches(">)").count();
                result.push_str(before);
                let prev = before.chars().next_back();
                let next = after[needle.len()..].chars().next();
                let boundary_before = prev.is_none_or(|p| p.is_whitespace() || "([{\"'".contains(p));
                let boundary_after = next.is_none_or(|n| !(n.is_alphanumeric() || matches!(n, '_' | '-' | '/')));
                if boundary_before && boundary_after && !in_code && !in_link {
                    result.push_str(&format!("[{needle}](<{}>)", tag_destination(&tag)));
                } else {
                    result.push_str(&needle);
                }
                rest = &after[needle.len()..];
            }
            result.push_str(rest);
            rewritten = result;
        }
        out.push_str(&rewritten);
    }
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn capture_clicked_wikilink(cache: &CommonMarkCache, targets: &[String], outcome: &mut RenderOutcome) {
    if outcome.clicked_wikilink.is_some() {
        return;
    }
    for title in targets {
        if cache.get_link_hook(&wikilink_destination(title)) == Some(true) {
            outcome.clicked_wikilink = Some(title.clone());
            return;
        }
    }
}

fn wikilink_destination(title: &str) -> String {
    format!("wikilink:{title}")
}

/// Extract the heading outline of `body`, assigning each heading a unique
/// slug (for scroll-to-heading navigation). Pure & fence-aware, mirroring
/// `wikilink::extract_wikilinks`.
pub fn headings(body: &str) -> Vec<Heading> {
    let mut out = Vec::new();
    let mut in_fence = false;
    let mut used_slugs: HashMap<String, u32> = HashMap::new();

    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some((level, title)) = parse_heading(trimmed) {
            let slug = dedupe_slug(&slugify(&title), &mut used_slugs);
            out.push(Heading { level, title, slug });
        }
    }
    out
}

fn parse_heading(trimmed: &str) -> Option<(u8, String)> {
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !rest.starts_with(' ') {
        return None;
    }
    let title = rest.trim().to_string();
    if title.is_empty() {
        return None;
    }
    Some((hashes as u8, title))
}

/// URL-style anchor for a heading title (`## Bahan Utama` → `bahan-utama`),
/// used for scroll-to-heading and `[[Note#Heading]]` links.
pub fn slugify(title: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for c in title.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

fn dedupe_slug(base: &str, used: &mut HashMap<String, u32>) -> String {
    let base = if base.is_empty() { "section" } else { base };
    let count = used.entry(base.to_string()).or_insert(0);
    *count += 1;
    if *count == 1 {
        base.to_string()
    } else {
        format!("{base}-{}", *count)
    }
}

/// One rendering unit: either a run of ordinary markdown lines, or a single
/// interactive checklist item. Splitting checklist lines out lets us toggle
/// them directly (native `ui.checkbox`) without depending on
/// `egui_commonmark`'s text-mutation span bookkeeping, which would break
/// once wikilink/heading preprocessing changes line lengths.
#[derive(Debug, PartialEq, Eq)]
enum Segment {
    Markdown(String),
    Checklist { line_idx: usize, checked: bool, text: String },
    /// A closed ```` ```mermaid ```` fence; `line_idx` is the opening fence.
    Mermaid { line_idx: usize, source: String },
}

fn segment_lines(body: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut in_fence = false;
    let lines: Vec<&str> = body.lines().collect();
    let mut skip_until = 0;

    for (idx, &line) in lines.iter().enumerate() {
        if idx < skip_until {
            continue;
        }
        if !in_fence
            && let Some(marker) = crate::mermaid::fence_open(line)
            && let Some(end) = (idx + 1..lines.len()).find(|&j| crate::mermaid::fence_close(lines[j], marker))
        {
            flush_markdown_segment(&mut current, &mut segments);
            segments.push(Segment::Mermaid { line_idx: idx, source: lines[idx + 1..end].join("\n") });
            skip_until = end + 1;
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            current.push(line);
            continue;
        }
        if !in_fence
            && let Some((checked, text)) = parse_checklist_line(line)
        {
            flush_markdown_segment(&mut current, &mut segments);
            segments.push(Segment::Checklist { line_idx: idx, checked, text });
            continue;
        }
        current.push(line);
    }
    flush_markdown_segment(&mut current, &mut segments);
    segments
}

fn flush_markdown_segment(current: &mut Vec<&str>, segments: &mut Vec<Segment>) {
    if current.is_empty() {
        return;
    }
    let joined = current.join("\n");
    if !joined.trim().is_empty() {
        segments.push(Segment::Markdown(joined));
    }
    current.clear();
}

fn parse_checklist_line(line: &str) -> Option<(bool, String)> {
    let trimmed = line.trim_start();
    let rest = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))?;

    if let Some(after) = strip_marker(rest, "[ ]") {
        Some((false, after.to_string()))
    } else {
        strip_marker(rest, "[x]")
            .or_else(|| strip_marker(rest, "[X]"))
            .map(|after| (true, after.to_string()))
    }
}

fn strip_marker<'a>(rest: &'a str, marker: &str) -> Option<&'a str> {
    let after = rest.strip_prefix(marker)?;
    Some(after.strip_prefix(' ').unwrap_or(after))
}

fn toggle_checklist_line(body: &str, line_idx: usize) -> String {
    let had_trailing_newline = body.ends_with('\n');
    let lines: Vec<String> = body
        .lines()
        .enumerate()
        .map(|(idx, line)| {
            if idx == line_idx {
                flip_checkbox_marker(line)
            } else {
                line.to_string()
            }
        })
        .collect();
    let mut joined = lines.join("\n");
    if had_trailing_newline {
        joined.push('\n');
    }
    joined
}

fn flip_checkbox_marker(line: &str) -> String {
    if let Some(pos) = line.find("[ ]") {
        format!("{}[x]{}", &line[..pos], &line[pos + 3..])
    } else if let Some(pos) = line.find("[x]").or_else(|| line.find("[X]")) {
        format!("{}[ ]{}", &line[..pos], &line[pos + 3..])
    } else {
        line.to_string()
    }
}

/// Transform raw ```canvas ... ``` blocks into clean, beautiful visual callout alerts with human-readable text.
fn transform_canvas_code_blocks(text: &str) -> String {
    if !text.contains("```canvas") {
        return text.to_string();
    }

    let mut result = String::new();
    let mut remaining = text;

    while let Some(start) = remaining.find("```canvas") {
        result.push_str(&remaining[..start]);
        let after_start = &remaining[start + 9..];
        if let Some(end) = after_start.find("```") {
            let json_body = after_start[..end].trim();
            let doc = crate::canvas::CanvasDocument::from_markdown_body("", json_body);
            let summary = doc.summary_text();
            let readable = doc.to_readable_markdown();

            result.push_str("> [!note] 🎨 **Papan Tulis Kanvas (Edgeless)**\n");
            result.push_str(&format!("> *{}*\n", summary));
            if !readable.is_empty() {
                result.push_str(">\n");
                for line in readable.lines() {
                    result.push_str(&format!("> {}\n", line));
                }
            } else {
                result.push_str(">\n> *Kanvas masih kosong. Buka mode 🎨 Edgeless di atas untuk mulai menggambar visual.*\n");
            }
            result.push('\n');

            remaining = &after_start[end + 3..];
        } else {
            result.push_str(&remaining[start..]);
            remaining = "";
            break;
        }
    }
    result.push_str(remaining);
    result
}

/// Rewrite `[[Title#Heading|Alias]]` into a real CommonMark link
/// (`[Alias](<wikilink:Title#Heading>)`, angle-bracketed since titles may
/// contain spaces) and `![[name]]` image embeds into a plain placeholder —
/// full attachment embedding is deferred past this phase. Links whose
/// target `is_resolved` rejects get italic link text, Obsidian's cue for
/// "this note doesn't exist yet". Fence-aware, so code blocks are left
/// untouched.
fn transform_wikilinks(text: &str, is_resolved: &dyn Fn(&str) -> bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_fence = false;
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            out.push_str(line);
            continue;
        }
        if in_fence {
            out.push_str(line);
            continue;
        }
        out.push_str(&transform_wikilinks_in_line(line, is_resolved));
    }
    out
}

fn transform_wikilinks_in_line(line: &str, is_resolved: &dyn Fn(&str) -> bool) -> String {
    let mut out = String::new();
    let mut rest = line;
    loop {
        let Some(start) = rest.find("[[") else {
            out.push_str(rest);
            break;
        };
        let is_embed = start > 0 && rest.as_bytes()[start - 1] == b'!';
        // Copy everything up to (but not including) the wikilink marker.
        // For an embed, also drop the '!' we would otherwise have copied.
        out.push_str(&rest[..if is_embed { start - 1 } else { start }]);

        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else {
            // Unterminated `[[`: treat the rest of the line as plain text.
            out.push_str(&rest[start..]);
            break;
        };
        let link = wikilink::WikiLink::parse(&after[..end]);

        if is_embed {
            out.push_str("📎 ");
            out.push_str(&link.target);
        } else {
            let text = link.alias.clone().unwrap_or_else(|| match &link.heading {
                Some(h) if !link.is_pdf() => format!("{} › {h}", link.target),
                _ => link.target.clone(),
            });
            let marker = if is_resolved(&link.target) { "" } else { "_" };
            out.push('[');
            out.push_str(marker);
            out.push_str(&text);
            out.push_str(marker);
            out.push_str("](<");
            out.push_str(&wikilink_destination(&link.reference()));
            out.push_str(">)");
        }
        rest = &after[end + 2..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_resolver(target: &str) -> Option<EmbedContent> {
        match target {
            "Anak" => Some(EmbedContent::Note {
                title: "Anak".into(),
                body: "# Bagian A\nisi a ^x1\n\n# Bagian B\nisi b\n![[Cucu]]\n".into(),
            }),
            "Cucu" => Some(EmbedContent::Note {
                title: "Cucu".into(),
                body: "cucu ![[Anak]]".into(),
            }),
            "foto.png" => Some(EmbedContent::Image("/v/foto.png".into())),
            _ => None,
        }
    }

    #[test]
    fn sheet_embeds_become_preview_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Kas.csv");
        std::fs::write(&path, "Item,Harga\nKopi,12000\n").unwrap();
        let resolver = |t: &str| (t == "Kas.csv").then(|| EmbedContent::Sheet(path.clone()));
        let out = transform_note_embeds("awal\n![[Kas.csv]]\nakhir\n", &resolver, 0);
        assert!(out.contains("| Item | Harga |\n| --- | ---: |\n| Kopi | 12000 |\n"), "{out}");
        assert!(out.contains("*[[Kas.csv]] · 1/1 rows · 2 columns*"));
        assert!(out.starts_with("awal\n") && out.ends_with("akhir\n"));
    }

    #[test]
    fn note_embeds_become_quote_callouts_with_depth_limit() {
        let out = transform_note_embeds("awal\n![[Anak#Bagian B]]\n![[Hilang]]\n![[foto.png]]\n", &fake_resolver, 0);
        assert!(out.starts_with("awal\n> [!quote] [[Anak]]\n> # Bagian B\n> isi b\n"), "{out}");
        // Nested embed rendered one level down, then stops recursing.
        assert!(out.contains("> > [!quote] [[Cucu]]"), "{out}");
        assert!(out.contains("![[Hilang]]"), "unresolved embeds are left alone");
        assert!(out.contains("![[foto.png]]"), "images are drawn separately");
        assert!(!out.contains("^x1"));
    }

    #[test]
    fn heading_section_and_image_targets() {
        let body = "intro\n# A\na1\n## A2\na2\n# B\nb1";
        assert_eq!(heading_section(body, "A").as_deref(), Some("# A\na1\n## A2\na2"));
        assert_eq!(heading_section(body, "a2").as_deref(), Some("## A2\na2"));
        assert!(heading_section(body, "Z").is_none());
        assert_eq!(image_embed_target("![[Foto Liburan.JPG|300]]"), Some("Foto Liburan.JPG"));
        assert_eq!(image_embed_target("![[Catatan]]"), None);
    }

    #[test]
    fn inline_tags_render_as_tag_links_outside_code_and_headings() {
        let out = transform_tags("# #bukan heading\nteks #projek/web dan `#kode`\n");
        assert!(out.contains("[#projek/web](<tag:projek/web>)"), "{out}");
        assert!(out.contains("`#kode`"));
        assert!(out.starts_with("# #bukan heading"));
    }

    #[test]
    fn headings_assigns_unique_slugs() {
        let body = "# Judul\n## Judul\ntext\n### Judul Lain!";
        let hs = headings(body);
        assert_eq!(
            hs,
            vec![
                Heading { level: 1, title: "Judul".into(), slug: "judul".into() },
                Heading { level: 2, title: "Judul".into(), slug: "judul-2".into() },
                Heading { level: 3, title: "Judul Lain!".into(), slug: "judul-lain".into() },
            ]
        );
    }

    #[test]
    fn headings_ignores_fenced_code_blocks() {
        let body = "```\n# Bukan heading\n```\n# Heading Asli";
        let hs = headings(body);
        assert_eq!(hs.len(), 1);
        assert_eq!(hs[0].title, "Heading Asli");
    }

    #[test]
    fn headings_requires_space_after_hashes() {
        assert!(headings("#tag-bukan-heading").is_empty());
    }

    #[test]
    fn segment_lines_splits_checklist_from_prose() {
        let body = "Intro\n- [ ] Beli beras\n- [x] Bayar listrik\nPenutup";
        let segments = segment_lines(body);
        assert_eq!(
            segments,
            vec![
                Segment::Markdown("Intro".into()),
                Segment::Checklist { line_idx: 1, checked: false, text: "Beli beras".into() },
                Segment::Checklist { line_idx: 2, checked: true, text: "Bayar listrik".into() },
                Segment::Markdown("Penutup".into()),
            ]
        );
    }

    #[test]
    fn segment_lines_ignores_checklist_syntax_inside_code_fence() {
        let body = "```\n- [ ] bukan checklist\n```";
        let segments = segment_lines(body);
        assert_eq!(segments, vec![Segment::Markdown(body.to_string())]);
    }

    #[test]
    fn toggle_checklist_line_flips_unchecked_to_checked() {
        let body = "- [ ] Beli beras\n- [x] Bayar listrik";
        let updated = toggle_checklist_line(body, 0);
        assert_eq!(updated, "- [x] Beli beras\n- [x] Bayar listrik");
    }

    #[test]
    fn toggle_checklist_line_preserves_trailing_newline() {
        let body = "- [ ] Beli beras\n";
        let updated = toggle_checklist_line(body, 0);
        assert_eq!(updated, "- [x] Beli beras\n");
    }

    fn all_resolved(_: &str) -> bool {
        true
    }

    #[test]
    fn transform_wikilinks_rewrites_plain_link() {
        let out = transform_wikilinks("Lihat [[Belanja Mingguan]] ya.", &all_resolved);
        assert_eq!(out, "Lihat [Belanja Mingguan](<wikilink:Belanja Mingguan>) ya.");
    }

    #[test]
    fn transform_wikilinks_uses_alias_as_link_text() {
        let out = transform_wikilinks("[[Belanja Mingguan|daftar belanja]]", &all_resolved);
        assert_eq!(out, "[daftar belanja](<wikilink:Belanja Mingguan>)");
    }

    #[test]
    fn transform_wikilinks_keeps_heading_in_destination() {
        let out = transform_wikilinks("[[Resep#Bahan]] [[a.pdf#page=2]]", &all_resolved);
        assert_eq!(
            out,
            "[Resep › Bahan](<wikilink:Resep#Bahan>) [a.pdf](<wikilink:a.pdf#page=2>)"
        );
    }

    #[test]
    fn transform_wikilinks_italicizes_unresolved_links() {
        let out = transform_wikilinks("[[Ada]] [[Belum Ada]]", &|t| t == "Ada");
        assert_eq!(
            out,
            "[Ada](<wikilink:Ada>) [_Belum Ada_](<wikilink:Belum Ada>)"
        );
    }

    #[test]
    fn transform_wikilinks_replaces_embed_with_placeholder() {
        let out = transform_wikilinks("![[foto.png]]", &all_resolved);
        assert_eq!(out, "📎 foto.png");
    }

    #[test]
    fn transform_wikilinks_leaves_code_fences_untouched() {
        let body = "```\n[[Bukan Link]]\n```";
        assert_eq!(transform_wikilinks(body, &all_resolved), body);
    }

    // --- §Fase 10: RenderCache / virtualization ---------------------------

    #[test]
    fn visible_segment_range_selects_only_overlapping_segments() {
        let heights = [10.0, 10.0, 10.0, 10.0, 10.0]; // rows at [0,10) [10,20) [20,30) [30,40) [40,50)
        assert_eq!(visible_segment_range(&heights, 15.0, 25.0, 0.0), 1..3);
    }

    #[test]
    fn visible_segment_range_widens_with_buffer() {
        // viewport [15,25] padded by 12 -> [3,37]; rows at
        // [0,10) [10,20) [20,30) [30,40) [40,50) — the last row starts at
        // 40, past the padded window, so it's still excluded.
        let heights = [10.0, 10.0, 10.0, 10.0, 10.0];
        assert_eq!(visible_segment_range(&heights, 15.0, 25.0, 12.0), 0..4);
    }

    #[test]
    fn visible_segment_range_empty_for_empty_input() {
        assert_eq!(visible_segment_range(&[], 0.0, 100.0, 0.0), 0..0);
    }

    #[test]
    fn visible_segment_range_covers_everything_for_an_unbounded_viewport() {
        let heights = [10.0, 10.0, 10.0];
        assert_eq!(
            visible_segment_range(&heights, f32::NEG_INFINITY, f32::INFINITY, 0.0),
            0..3
        );
    }

    #[test]
    fn render_cache_reparses_only_when_body_changes() {
        let mut cache = RenderCache::default();
        let body_a = "Intro\n- [ ] Beli beras";
        let body_b = "Intro\n- [ ] Beli beras\n- [x] Bayar listrik";

        cache.ensure_fresh(body_a);
        assert_eq!(cache.segments.len(), 2);
        let hash_after_a = cache.body_hash;

        // Re-running with the same body is a no-op: hash unchanged.
        cache.ensure_fresh(body_a);
        assert_eq!(cache.body_hash, hash_after_a);

        cache.ensure_fresh(body_b);
        assert_eq!(cache.segments.len(), 3);
        assert_ne!(cache.body_hash, hash_after_a);
    }

    #[test]
    fn render_cache_invalidate_forces_reparse_of_same_body() {
        let mut cache = RenderCache::default();
        let body = "# Judul\nIsi";
        cache.ensure_fresh(body);
        let hash = cache.body_hash;

        cache.invalidate();
        assert_eq!(cache.body_hash, None);

        cache.ensure_fresh(body);
        assert_eq!(cache.body_hash, hash); // same content -> same hash again
    }

    #[test]
    fn render_cache_outline_matches_headings() {
        let mut cache = RenderCache::default();
        let body = "# Judul\ntext\n## Sub";
        assert_eq!(cache.outline(body), headings(body).as_slice());
    }

    #[test]
    fn estimate_height_scales_with_line_count() {
        let one_line = Segment::Markdown("satu baris".to_string());
        let three_lines = Segment::Markdown("a\nb\nc".to_string());
        assert!(estimate_height(&three_lines) > estimate_height(&one_line));

        let checklist = Segment::Checklist { line_idx: 0, checked: false, text: "x".into() };
        assert_eq!(estimate_height(&checklist), estimate_height(&one_line));
    }

    #[test]
    fn mermaid_fences_become_their_own_segment() {
        let segs = segment_lines("a\n```mermaid\nflowchart LR\nA-->B\n```\nb\n```mermaid\nunclosed");
        assert_eq!(segs[0], Segment::Markdown("a".into()));
        assert_eq!(segs[1], Segment::Mermaid { line_idx: 1, source: "flowchart LR\nA-->B".into() });
        // An unclosed fence stays ordinary markdown.
        assert!(matches!(&segs[2], Segment::Markdown(t) if t.starts_with('b') && t.contains("unclosed")));
        assert_eq!(estimate_height(&segs[1]), 320.0);
    }

    #[test]
    fn transform_canvas_code_blocks_renders_alert_and_prose() {
        let raw_canvas_body = "```canvas\n{\n  \"id\": \"00000000-0000-0000-0000-000000000000\",\n  \"title\": \"Diagram\",\n  \"elements\": [],\n  \"viewport\": {\"pan\": [0.0, 0.0], \"zoom\": 1.0}\n}\n```";
        let out = transform_canvas_code_blocks(&raw_canvas_body);
        assert!(out.contains("> [!note] 🎨 **Papan Tulis Kanvas (Edgeless)**"));
        assert!(out.contains("Kanvas masih kosong"));
        assert!(!out.contains("```canvas"));
    }
}
