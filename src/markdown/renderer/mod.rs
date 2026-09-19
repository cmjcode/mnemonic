//! The Live note view (§3.2.1 "Live Preview", the only note mode besides
//! the canvas ones): the body is always rendered, block by block
//! (`markdown::live_blocks`), and the block the user clicks is handed back
//! as an `EditRequest` so the caller can show *just those lines* as raw
//! Markdown in their place (`LiveParams::active` + the `draw_editor`
//! callback). Rendering goes through `egui_commonmark` (§3.2.2): wikilinks
//! and `#tags` become clickable, checklists toggle in place, callouts and
//! headings take their colours from the reading theme (§3.2.5).
//!
//! `RenderCache` + virtualization (§Fase 10, §6 risk 5): the body→blocks
//! split, link targets and outline are memoized behind a content hash,
//! measured block heights are remembered per block content (so typing in
//! one line doesn't reset every other block's height and make the page
//! jump), and blocks outside the viewport (± a buffer) only reserve their
//! height instead of being laid out.
//!
//! Callers: `markdown::editor::MarkdownEditor::render` (which owns a
//! `RenderCache` per open note), `export` (via `transform`).

mod draw;
pub(crate) mod transform;

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use egui_commonmark::CommonMarkCache;

use super::blocks;
use super::live_blocks::{self, BlockKind, LiveBlock};
use super::wikilink;
use crate::reading_theme::ThemeColors;

/// A heading extracted from a note body, with a slug for scroll-to-heading
/// navigation — backs the Outline/TOC panel (§3.2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub title: String,
    pub slug: String,
}

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

/// The user clicked a rendered block: show `lines` as raw Markdown.
#[derive(Debug, Clone, PartialEq)]
pub struct EditRequest {
    pub lines: Range<usize>,
    /// Where the click landed (screen space), to place the text cursor;
    /// `None` when the block's edit button was used.
    pub pos: Option<egui::Pos2>,
}

/// What happened during a render that the caller needs to act on.
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
    /// A rendered block was clicked (to edit it).
    pub edit: Option<EditRequest>,
    /// The empty space below the note was clicked (continue writing there).
    pub clicked_after_end: bool,
    /// The lines the raw editor covered this frame: `LiveParams::active`
    /// widened to whole blocks (a line that became part of a table or a
    /// closed fence pulls the whole block in).
    pub active_lines: Option<Range<usize>>,
}

/// Inputs of one Live render besides the body itself.
pub struct LiveParams<'a> {
    /// Visible region in the renderer's content space (from
    /// `ScrollArea::show_viewport`, shifted to the body's first line).
    pub viewport: egui::Rect,
    /// Whether a link target exists (unresolved links render in italics).
    pub is_resolved: &'a dyn Fn(&str) -> bool,
    pub resolve_embed: &'a EmbedResolver<'a>,
    /// Reading-theme colours for the current light/dark mode.
    pub colors: &'a ThemeColors,
    /// Lines currently being edited as raw Markdown. An empty range at or
    /// past the last line is a new line being started at the end.
    pub active: Option<Range<usize>>,
    /// Shown when the note is empty.
    pub placeholder: &'a str,
}

/// Extra content-space padding rendered above/below the visible viewport
/// (§6 risk 5 "virtualized scrolling"), so a block already has its real
/// layout by the time it scrolls fully into view.
const VIRTUALIZATION_BUFFER: f32 = 600.0;
/// Clickable space under the last block ("click to keep writing").
const TAIL_HEIGHT: f32 = 160.0;

enum ScrollTarget {
    /// A `^block-id` anchor.
    Block(String),
    /// A heading slug.
    Heading(String),
}

/// Memoized parse of a note body, owned by the caller (`MarkdownEditor`)
/// across frames and invalidated (`invalidate`) whenever the body actually
/// changes. See the module doc comment for why this exists.
#[derive(Default)]
pub struct RenderCache {
    body_hash: Option<u64>,
    blocks: Vec<LiveBlock>,
    /// Heading slug per block (`None` for non-headings), parallel to `blocks`.
    slugs: Vec<Option<String>>,
    /// Content key per block (`block_key`), parallel to `blocks`.
    keys: Vec<u64>,
    wikilink_targets: Vec<String>,
    /// Inline `#tag`s, for click hooks.
    tag_targets: Vec<String>,
    scroll_to: Option<ScrollTarget>,
    /// Decoded image embeds, by file path (`None` = failed to decode).
    textures: HashMap<std::path::PathBuf, Option<egui::TextureHandle>>,
    /// Estimated-then-measured height per block, content-space pixels,
    /// parallel to `blocks`. Drives `visible_block_range`.
    heights: Vec<f32>,
    /// Last measured height by block content, surviving re-parses.
    height_memo: HashMap<u64, f32>,
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

    /// Scroll to the block anchored `^id` the next time the view is drawn.
    pub fn scroll_to_block(&mut self, id: &str) {
        self.scroll_to = Some(ScrollTarget::Block(id.to_string()));
    }

    /// Scroll to the heading with this slug (`slugify`) the next time the
    /// view is drawn.
    pub fn scroll_to_heading(&mut self, slug: &str) {
        self.scroll_to = Some(ScrollTarget::Heading(slug.to_string()));
    }

    /// Re-parses `body` only if it differs from the last body this cache
    /// was built from — otherwise a no-op, so a frame where nothing
    /// changed doesn't re-walk the whole document.
    fn ensure_fresh(&mut self, body: &str) {
        let hash = hash_str(body);
        if self.body_hash == Some(hash) {
            return;
        }
        let lines: Vec<&str> = body.lines().collect();
        self.blocks = live_blocks::split_blocks(body);
        self.keys = self.blocks.iter().map(|b| block_key(&lines, b)).collect();
        let mut used = HashMap::new();
        self.slugs = self
            .blocks
            .iter()
            .map(|b| match b.kind {
                BlockKind::Heading(_) => parse_heading(lines[b.lines.start].trim_start())
                    .map(|(_, title)| dedupe_slug(&slugify(&title), &mut used)),
                _ => None,
            })
            .collect();
        self.height_memo.retain(|k, _| self.keys.contains(k));
        self.heights = self
            .blocks
            .iter()
            .zip(&self.keys)
            .map(|(b, k)| self.height_memo.get(k).copied().unwrap_or_else(|| estimate_height(b)))
            .collect();
        let mut references: Vec<String> =
            wikilink::parse_wikilinks(body).into_iter().map(|o| o.link.reference()).collect();
        references.sort_unstable();
        references.dedup();
        self.wikilink_targets = references;
        self.tag_targets = crate::notes::tags::inline_tags(body);
        self.outline = headings(body);
        // Keep only diagrams still present (either theme).
        let live: Vec<u64> = self
            .blocks
            .iter()
            .filter(|b| b.kind == BlockKind::Mermaid)
            .flat_map(|b| {
                let source = lines[b.lines.start + 1..b.lines.end - 1].join("\n");
                [diagram_key(&source, false), diagram_key(&source, true)]
            })
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

    /// The Live blocks of `body`.
    pub fn blocks(&mut self, body: &str) -> &[LiveBlock] {
        self.ensure_fresh(body);
        &self.blocks
    }
}

fn diagram_key(source: &str, dark: bool) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    dark.hash(&mut hasher);
    hasher.finish()
}

fn hash_str(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Identity of a block's rendered content, for the height memo.
fn block_key(lines: &[&str], block: &LiveBlock) -> u64 {
    let mut hasher = DefaultHasher::new();
    lines[block.lines.clone()].hash(&mut hasher);
    std::mem::discriminant(&block.kind).hash(&mut hasher);
    hasher.finish()
}

/// Rough pre-render height of a block that hasn't been measured yet, so a
/// freshly opened long note can virtualize from its first frame. The real
/// height replaces it as soon as the block is drawn.
fn estimate_height(block: &LiveBlock) -> f32 {
    const LINE_HEIGHT: f32 = 24.0;
    let lines = block.lines.len() as f32;
    match block.kind {
        BlockKind::Blank => draw::BLANK_HEIGHT + 4.0,
        BlockKind::Heading(1 | 2) => 48.0,
        BlockKind::Heading(_) => 36.0,
        BlockKind::Mermaid => 320.0,
        BlockKind::Code | BlockKind::Table | BlockKind::Callout | BlockKind::Math => lines * LINE_HEIGHT + 24.0,
        _ => LINE_HEIGHT + 4.0,
    }
}

/// Given block `heights` (content-space) and the visible viewport
/// `[viewport_top, viewport_bottom]`, returns the index range of blocks
/// that overlap the viewport padded by `buffer` on both sides. Pure and
/// independent of `egui` so it's directly unit-testable.
fn visible_block_range(heights: &[f32], viewport_top: f32, viewport_bottom: f32, buffer: f32) -> Range<usize> {
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

/// `active` widened to the whole blocks it touches.
fn widen_to_blocks(blocks: &[LiveBlock], active: &Range<usize>) -> Range<usize> {
    let mut span = active.clone();
    for b in blocks.iter().filter(|b| b.lines.start < active.end && active.start < b.lines.end) {
        span.start = span.start.min(b.lines.start);
        span.end = span.end.max(b.lines.end);
    }
    span
}

/// Renders `body` in Live mode, memoized against `index` and virtualized
/// against `params.viewport`. The lines in `params.active` are not
/// rendered: `draw_editor` is called once, in their place, to draw the raw
/// Markdown editor. Returns clicks the caller must act on.
pub fn render_cached(
    ui: &mut egui::Ui,
    cache: &mut CommonMarkCache,
    index: &mut RenderCache,
    body: &str,
    params: &LiveParams<'_>,
    draw_editor: &mut dyn FnMut(&mut egui::Ui),
) -> RenderOutcome {
    index.ensure_fresh(body);
    let mut outcome = RenderOutcome::default();
    let lines: Vec<&str> = body.lines().collect();

    // Re-register link hooks for this frame's wikilinks and tags so
    // egui_commonmark intercepts clicks instead of opening them as URLs.
    // Cleared every frame to avoid unbounded growth across notes.
    cache.link_hooks_clear();
    for title in &index.wikilink_targets {
        cache.add_link_hook(transform::wikilink_destination(title));
    }
    for tag in &index.tag_targets {
        cache.add_link_hook(transform::tag_destination(tag));
    }
    let scroll_target = index.scroll_to.take();
    let env = draw::BlockEnv {
        colors: params.colors,
        is_resolved: params.is_resolved,
        resolve_embed: params.resolve_embed,
    };

    let span = params.active.as_ref().map(|a| widen_to_blocks(&index.blocks, a));
    let mut editor_drawn = false;
    let mut draw_editor_here = |ui: &mut egui::Ui, outcome: &mut RenderOutcome| {
        ui.add_space(2.0);
        draw_editor(ui);
        ui.add_space(2.0);
        outcome.active_lines = span.clone();
    };

    if index.blocks.is_empty() && span.is_none() {
        ui.label(
            egui::RichText::new(params.placeholder)
                .size(draw::BODY_SIZE)
                .color(egui::Color32::from(params.colors.muted)),
        );
    }

    let visible = visible_block_range(&index.heights, params.viewport.top(), params.viewport.bottom(), VIRTUALIZATION_BUFFER);
    ui.spacing_mut().item_spacing.y = 0.0;

    for i in 0..index.blocks.len() {
        let block = index.blocks[i].clone();
        if let Some(span) = &span {
            if !editor_drawn && block.lines.start >= span.start {
                draw_editor_here(ui, &mut outcome);
                editor_drawn = true;
            }
            if block.lines.start < span.end && span.start < block.lines.end {
                continue;
            }
        }

        let top = ui.cursor().top();
        let left = ui.cursor().left();
        let width = ui.available_width();
        if visible.contains(&i) {
            // Registered *before* the content, so links, checkboxes and
            // diagrams drawn on top of it keep their own clicks.
            let sensor_rect = egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(width, index.heights[i]));
            let sensor = ui
                .interact(sensor_rect, ui.id().with(("live_block", block.lines.start)), egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::Text);
            let source = live_blocks::block_source(&lines, &block);
            let edit_button = ui
                .scope(|ui| draw::draw_block(ui, cache, index, body, &block, &source, &env, &mut outcome))
                .inner;
            // `CommonMarkViewer::show` resets all link hooks at the start of
            // the next call, so clicks must be read back per block.
            capture_clicked_wikilink(cache, &index.wikilink_targets, &mut outcome);
            capture_clicked_tag(cache, &index.tag_targets, &mut outcome);
            if sensor.clicked() {
                outcome.edit = Some(EditRequest { lines: block.lines.clone(), pos: sensor.interact_pointer_pos() });
            } else if edit_button {
                outcome.edit = Some(EditRequest { lines: block.lines.clone(), pos: None });
            }
        } else {
            ui.allocate_space(egui::vec2(width, index.heights[i]));
        }
        let bottom = ui.cursor().top();
        let measured = (bottom - top).max(1.0);
        index.heights[i] = measured;
        if visible.contains(&i) {
            index.height_memo.insert(index.keys[i], measured);
        }

        let is_target = match &scroll_target {
            Some(ScrollTarget::Block(id)) => lines[block.lines.clone()].iter().any(|l| blocks::line_has_anchor(l, id)),
            Some(ScrollTarget::Heading(slug)) => index.slugs[i].as_deref() == Some(slug.as_str()),
            None => false,
        };
        if is_target {
            let rect = egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(left + width, bottom));
            ui.scroll_to_rect(rect, Some(egui::Align::TOP));
        }
    }
    if span.is_some() && !editor_drawn {
        draw_editor_here(ui, &mut outcome);
    }

    let (_, tail) = ui.allocate_exact_size(egui::vec2(ui.available_width(), TAIL_HEIGHT), egui::Sense::click());
    if tail.on_hover_cursor(egui::CursorIcon::Text).clicked() {
        outcome.clicked_after_end = true;
    }
    outcome
}

fn capture_clicked_tag(cache: &CommonMarkCache, tags: &[String], outcome: &mut RenderOutcome) {
    if outcome.clicked_tag.is_some() {
        return;
    }
    if let Some(tag) = tags.iter().find(|t| cache.get_link_hook(&transform::tag_destination(t)) == Some(true)) {
        outcome.clicked_tag = Some(tag.clone());
    }
}

fn capture_clicked_wikilink(cache: &CommonMarkCache, targets: &[String], outcome: &mut RenderOutcome) {
    if outcome.clicked_wikilink.is_some() {
        return;
    }
    if let Some(title) =
        targets.iter().find(|t| cache.get_link_hook(&transform::wikilink_destination(t)) == Some(true))
    {
        outcome.clicked_wikilink = Some(title.clone());
    }
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
    let title = blocks::strip_anchors(rest.trim()).trim().to_string();
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
    if *count == 1 { base.to_string() } else { format!("{base}-{}", *count) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_assigns_unique_slugs() {
        let body = "# Judul\n## Judul\ntext\n### Judul Lain!";
        assert_eq!(
            headings(body),
            vec![
                Heading { level: 1, title: "Judul".into(), slug: "judul".into() },
                Heading { level: 2, title: "Judul".into(), slug: "judul-2".into() },
                Heading { level: 3, title: "Judul Lain!".into(), slug: "judul-lain".into() },
            ]
        );
    }

    #[test]
    fn headings_ignores_fences_and_needs_a_space() {
        let hs = headings("```\n# Bukan heading\n```\n# Heading Asli");
        assert_eq!(hs.len(), 1);
        assert_eq!(hs[0].title, "Heading Asli");
        assert!(headings("#tag-bukan-heading").is_empty());
    }

    #[test]
    fn visible_block_range_selects_overlapping_blocks() {
        let heights = [10.0, 10.0, 10.0, 10.0, 10.0]; // [0,10) [10,20) … [40,50)
        assert_eq!(visible_block_range(&heights, 15.0, 25.0, 0.0), 1..3);
        // Padded by 12 -> [3,37]: the last block starts at 40, excluded.
        assert_eq!(visible_block_range(&heights, 15.0, 25.0, 12.0), 0..4);
        assert_eq!(visible_block_range(&[], 0.0, 100.0, 0.0), 0..0);
        assert_eq!(visible_block_range(&heights[..3], f32::NEG_INFINITY, f32::INFINITY, 0.0), 0..3);
    }

    #[test]
    fn render_cache_reparses_only_when_body_changes() {
        let mut cache = RenderCache::default();
        cache.ensure_fresh("Intro\n- [ ] Beli beras");
        assert_eq!(cache.blocks.len(), 2);
        let hash = cache.body_hash;
        cache.ensure_fresh("Intro\n- [ ] Beli beras");
        assert_eq!(cache.body_hash, hash);
        cache.ensure_fresh("Intro\n- [ ] Beli beras\n- [x] Bayar listrik");
        assert_eq!(cache.blocks.len(), 3);
        cache.invalidate();
        assert_eq!(cache.body_hash, None);
    }

    #[test]
    fn measured_heights_survive_edits_elsewhere() {
        let mut cache = RenderCache::default();
        cache.ensure_fresh("# Judul\nsatu\ndua");
        let key = cache.keys[2];
        cache.height_memo.insert(key, 99.0);
        // Editing line 1 re-parses, but "dua" keeps its measured height.
        cache.ensure_fresh("# Judul\nsatu diubah\ndua");
        assert_eq!(cache.heights[2], 99.0);
        assert_eq!(cache.heights[1], estimate_height(&cache.blocks[1]));
    }

    #[test]
    fn heading_slugs_match_the_outline() {
        let mut cache = RenderCache::default();
        let body = "# A\ntext\n## A ^id\n```\n# kode\n```";
        cache.ensure_fresh(body);
        let slugs: Vec<&str> = cache.slugs.iter().flatten().map(String::as_str).collect();
        let outline: Vec<String> = headings(body).into_iter().map(|h| h.slug).collect();
        assert_eq!(slugs, outline);
        assert_eq!(slugs, ["a", "a-2"]);
    }

    #[test]
    fn active_range_widens_to_whole_blocks() {
        let blocks = live_blocks::split_blocks("a\n| x |\n| --- |\n| 1 |\nb");
        assert_eq!(widen_to_blocks(&blocks, &(2..3)), 1..4);
        assert_eq!(widen_to_blocks(&blocks, &(0..1)), 0..1);
        // An insertion point (empty range) touches nothing.
        assert_eq!(widen_to_blocks(&blocks, &(5..5)), 5..5);
    }

    #[test]
    fn estimates_scale_with_block_kind() {
        let blocks = live_blocks::split_blocks("# H\n\ntext\n```mermaid\nflowchart LR\n```");
        assert!(estimate_height(&blocks[0]) > estimate_height(&blocks[2]));
        assert!(estimate_height(&blocks[1]) < estimate_height(&blocks[2]));
        assert_eq!(estimate_height(&blocks[3]), 320.0);
    }
}
