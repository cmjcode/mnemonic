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

use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

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
#[derive(Debug, Default)]
pub struct RenderOutcome {
    /// A checklist checkbox was toggled; this is the note body with that
    /// line's `[ ]`/`[x]` marker flipped.
    pub updated_body: Option<String>,
    /// The user clicked a `[[wikilink]]`; this is the target title.
    pub clicked_wikilink: Option<String>,
}

/// Render `body` into `ui`, always fully (no memoization, no
/// virtualization) — a thin wrapper over `render_cached` with a throwaway
/// `RenderCache` and an unbounded viewport. Kept for any caller without a
/// per-note `RenderCache` to reuse across frames; `app.rs`'s note editor
/// calls `render_cached` (via `MarkdownEditor::render`) directly so repeated
/// frames actually benefit from the memoization (§Fase 10).
pub fn render(ui: &mut egui::Ui, cache: &mut CommonMarkCache, body: &str) -> RenderOutcome {
    let mut index = RenderCache::default();
    render_cached(ui, cache, &mut index, body, egui::Rect::EVERYTHING)
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
    /// One estimated-then-measured height per segment, content-space
    /// pixels, parallel to `segments`. Drives `visible_segment_range`.
    heights: Vec<f32>,
    outline: Vec<Heading>,
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
        self.wikilink_targets = wikilink::extract_wikilinks(body);
        self.heights = self.segments.iter().map(estimate_height).collect();
        self.outline = headings(body);
        self.body_hash = Some(hash);
    }

    /// The heading outline for `body` (Outline side panel), recomputed only
    /// when `body` changed since the last call on this cache.
    pub fn outline(&mut self, body: &str) -> &[Heading] {
        self.ensure_fresh(body);
        &self.outline
    }
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
/// `render`.
pub fn render_cached(
    ui: &mut egui::Ui,
    cache: &mut CommonMarkCache,
    index: &mut RenderCache,
    body: &str,
    viewport: egui::Rect,
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

    let visible = visible_segment_range(&index.heights, viewport.top(), viewport.bottom(), VIRTUALIZATION_BUFFER);

    for i in 0..index.segments.len() {
        let top = ui.cursor().top();
        if visible.contains(&i) {
            match &index.segments[i] {
                Segment::Markdown(text) => {
                    let transformed = transform_wikilinks(text);
                    CommonMarkViewer::new()
                        .enable_scroll_to_heading(true)
                        .show(ui, cache, &transformed);
                }
                Segment::Checklist { line_idx, checked, text } => {
                    let line_idx = *line_idx;
                    let is_checked_initially = *checked;
                    let transformed = transform_wikilinks(text);
                    ui.horizontal(|ui| {
                        let mut is_checked = is_checked_initially;
                        if ui.checkbox(&mut is_checked, "").changed() {
                            outcome.updated_body = Some(toggle_checklist_line(body, line_idx));
                        }
                        CommonMarkViewer::new().show(ui, cache, &transformed);
                    });
                }
            }
            // `CommonMarkViewer::show` resets all link hooks to `false` at
            // the *start* of the next call (`prepare_show`), so a click
            // must be read back right after the segment that produced it,
            // not once at the end of the whole document.
            capture_clicked_wikilink(cache, &index.wikilink_targets, &mut outcome);
        } else {
            ui.allocate_space(egui::vec2(ui.available_width(), index.heights[i]));
        }
        let bottom = ui.cursor().top();
        index.heights[i] = (bottom - top).max(1.0);
    }

    outcome
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

fn slugify(title: &str) -> String {
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
}

fn segment_lines(body: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut in_fence = false;

    for (idx, line) in body.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            current.push(line);
            continue;
        }
        if !in_fence {
            if let Some((checked, text)) = parse_checklist_line(line) {
                flush_markdown_segment(&mut current, &mut segments);
                segments.push(Segment::Checklist { line_idx: idx, checked, text });
                continue;
            }
        }
        current.push(line);
    }
    flush_markdown_segment(&mut current, &mut segments);
    segments
}

fn flush_markdown_segment<'a>(current: &mut Vec<&'a str>, segments: &mut Vec<Segment>) {
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
    } else if let Some(after) = strip_marker(rest, "[x]").or_else(|| strip_marker(rest, "[X]")) {
        Some((true, after.to_string()))
    } else {
        None
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

/// Rewrite `[[Title]]` / `[[Title|Alias]]` into a real CommonMark link
/// (`[Alias](<wikilink:Title>)`, angle-bracketed since titles may contain
/// spaces) and `![[name]]` image embeds into a plain placeholder — full
/// attachment embedding is deferred past this phase. Fence-aware, so code
/// blocks are left untouched.
fn transform_wikilinks(text: &str) -> String {
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
        out.push_str(&transform_wikilinks_in_line(line));
    }
    out
}

fn transform_wikilinks_in_line(line: &str) -> String {
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
        let inner = &after[..end];
        let mut parts = inner.splitn(2, '|');
        let title = parts.next().unwrap_or(inner).trim();
        let alias = parts.next().map(str::trim).filter(|a| !a.is_empty()).unwrap_or(title);

        if is_embed {
            out.push_str("📎 ");
            out.push_str(title);
        } else {
            out.push('[');
            out.push_str(alias);
            out.push_str("](<");
            out.push_str(&wikilink_destination(title));
            out.push_str(">)");
        }
        rest = &after[end + 2..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn transform_wikilinks_rewrites_plain_link() {
        let out = transform_wikilinks("Lihat [[Belanja Mingguan]] ya.");
        assert_eq!(out, "Lihat [Belanja Mingguan](<wikilink:Belanja Mingguan>) ya.");
    }

    #[test]
    fn transform_wikilinks_uses_alias_as_link_text() {
        let out = transform_wikilinks("[[Belanja Mingguan|daftar belanja]]");
        assert_eq!(out, "[daftar belanja](<wikilink:Belanja Mingguan>)");
    }

    #[test]
    fn transform_wikilinks_replaces_embed_with_placeholder() {
        let out = transform_wikilinks("![[foto.png]]");
        assert_eq!(out, "📎 foto.png");
    }

    #[test]
    fn transform_wikilinks_leaves_code_fences_untouched() {
        let body = "```\n[[Bukan Link]]\n```";
        assert_eq!(transform_wikilinks(body), body);
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
}
