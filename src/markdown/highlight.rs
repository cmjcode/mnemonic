//! Obsidian-style styling of the *source* editor (§Fase 1.7 "live preview
//! ringan"): headings render large, `**bold**`/`*italic*` show as such,
//! `[[links]]` and `#tags` take the accent color, inline/fenced code is
//! monospace on a tinted background, block anchors `^id` fade out — all
//! while the raw Markdown stays editable character for character. Builds
//! an `epaint::text::LayoutJob` for `egui::TextEdit::layouter`; pure
//! (no `Ui`), so the byte-range bookkeeping is unit-testable.
//! Callers: `app::editor`.

use egui::text::{ByteIndex, LayoutJob, LayoutSection, TextFormat};
use egui::{Color32, FontFamily, FontId, Stroke};

/// Colors and fonts the highlighter draws with — the caller derives them
/// from the active palette so this module stays theme-agnostic.
#[derive(Clone)]
pub struct HighlightStyle {
    pub base_size: f32,
    pub text: Color32,
    pub dim: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub code_bg: Color32,
    pub highlight_bg: Color32,
    /// Family used for bold runs and headings.
    pub semibold: FontFamily,
    pub line_height: f32,
}

#[derive(Clone, Copy, Default)]
struct Inline {
    bold: bool,
    italic: bool,
    code: bool,
    link: bool,
    math: bool,
    mark: bool,
    strike: bool,
}

/// Heading font size for `#`…`######`.
fn heading_size(level: usize, base: f32) -> f32 {
    match level {
        1 => base * 1.9,
        2 => base * 1.55,
        3 => base * 1.3,
        4 => base * 1.15,
        _ => base * 1.05,
    }
}

/// Lays out `text` with Markdown-aware styling, wrapped at `wrap_width`.
pub fn layout_job(text: &str, wrap_width: f32, style: &HighlightStyle) -> LayoutJob {
    let mut job = LayoutJob {
        text: text.to_string(),
        ..Default::default()
    };
    job.wrap.max_width = wrap_width;

    let mut in_fence = false;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let content = line.trim_end_matches(['\n', '\r']);
        let trimmed = content.trim_start();
        let indent = content.len() - trimmed.len();

        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            push(&mut job, start, start + line.len(), code_format(style, style.faint));
            continue;
        }
        if in_fence {
            push(&mut job, start, start + line.len(), code_format(style, style.text));
            continue;
        }

        // Headings: `#` marks faint, title large & semibold. Marks and title
        // share one line height, or the marks sit on a different baseline.
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
            let size = heading_size(hashes, style.base_size);
            let marks_end = start + indent + hashes + 1;
            let heading = TextFormat {
                font_id: FontId::new(size, style.semibold.clone()),
                color: style.text,
                line_height: Some(size * 1.35),
                ..base_format(style)
            };
            push(&mut job, start, marks_end, TextFormat { color: style.faint, ..heading.clone() });
            inline_runs(&mut job, style, &line[indent + hashes + 1..], marks_end, heading);
            continue;
        }

        // Block prefix: list bullets, task boxes, numbered items, quotes.
        let mut cursor = start + indent;
        if indent > 0 {
            push(&mut job, start, cursor, base_format(style));
        }
        let rest = trimmed;
        let prefix_len = block_prefix_len(rest);
        if prefix_len > 0 {
            push(
                &mut job,
                cursor,
                cursor + prefix_len,
                TextFormat {
                    color: style.accent,
                    ..base_format(style)
                },
            );
            cursor += prefix_len;
        }
        let mut body_format = base_format(style);
        if rest.starts_with('>') {
            body_format.color = style.dim;
            body_format.italics = true;
        }
        inline_runs(&mut job, style, &line[cursor - start..], cursor, body_format);
    }
    job
}

/// Bytes of a leading `- `, `* `, `+ `, `- [ ] `, `1. `, `> ` marker.
fn block_prefix_len(rest: &str) -> usize {
    if let Some(after) = rest.strip_prefix("- ").or_else(|| rest.strip_prefix("* ")).or_else(|| rest.strip_prefix("+ ")) {
        let task = ["[ ] ", "[x] ", "[X] "]
            .iter()
            .find(|m| after.starts_with(*m))
            .map(|m| m.len())
            .unwrap_or(0);
        return 2 + task;
    }
    if rest.starts_with("> ") {
        return 2;
    }
    if rest == ">" {
        return 1;
    }
    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && rest[digits..].starts_with(". ") {
        return digits + 2;
    }
    0
}

fn base_format(style: &HighlightStyle) -> TextFormat {
    TextFormat {
        font_id: FontId::proportional(style.base_size),
        color: style.text,
        line_height: Some(style.line_height),
        ..Default::default()
    }
}

fn code_format(style: &HighlightStyle, color: Color32) -> TextFormat {
    TextFormat {
        font_id: FontId::monospace(style.base_size - 1.0),
        color,
        background: style.code_bg,
        line_height: Some(style.line_height),
        ..Default::default()
    }
}

fn format_for(style: &HighlightStyle, base: &TextFormat, st: Inline) -> TextFormat {
    let mut f = base.clone();
    if st.code {
        f.font_id = FontId::monospace(base.font_id.size - 1.0);
        f.background = style.code_bg;
        return f;
    }
    if st.math {
        f.italics = true;
        f.color = style.dim;
        return f;
    }
    if st.bold {
        f.font_id = FontId::new(base.font_id.size, style.semibold.clone());
    }
    if st.italic {
        f.italics = true;
    }
    if st.link {
        f.color = style.accent;
        f.underline = Stroke::new(1.0, style.accent);
    }
    if st.mark {
        f.background = style.highlight_bg;
    }
    if st.strike {
        f.strikethrough = Stroke::new(1.0, style.dim);
    }
    f
}

/// Styles the inline content of one line (`segment`, which starts at byte
/// `base_offset` of the job text) on top of `base`.
fn inline_runs(job: &mut LayoutJob, style: &HighlightStyle, segment: &str, base_offset: usize, base: TextFormat) {
    let bytes = segment.as_bytes();
    let mut st = Inline::default();
    let mut run_start = 0;
    let mut i = 0;
    let mut pending: Vec<(usize, usize, TextFormat)> = Vec::new();

    // Block anchor ` ^id` at the end of the line fades out.
    let content_end = segment.trim_end_matches(['\n', '\r']).len();
    let anchor_start = super::blocks::split_anchor(&segment[..content_end])
        .1
        .map(|id| content_end - id.len() - 1);

    let flush = |pending: &mut Vec<(usize, usize, TextFormat)>, from: usize, to: usize, st: Inline| {
        if to > from {
            pending.push((from, to, format_for(style, &base, st)));
        }
    };

    while i < content_end {
        if let Some(a) = anchor_start
            && i >= a
        {
            break;
        }
        let rest = &segment[i..content_end];
        // Inline code swallows everything until the closing backtick.
        if st.code {
            if rest.starts_with('`') {
                flush(&mut pending, run_start, i + 1, st);
                st.code = false;
                i += 1;
                run_start = i;
            } else {
                i += rest.chars().next().map(char::len_utf8).unwrap_or(1);
            }
            continue;
        }
        if st.math {
            if rest.starts_with('$') {
                let n = if rest.starts_with("$$") { 2 } else { 1 };
                flush(&mut pending, run_start, i + n, st);
                st.math = false;
                i += n;
                run_start = i;
            } else {
                i += rest.chars().next().map(char::len_utf8).unwrap_or(1);
            }
            continue;
        }
        if st.link {
            if rest.starts_with("]]") {
                flush(&mut pending, run_start, i + 2, st);
                st.link = false;
                i += 2;
                run_start = i;
            } else {
                i += rest.chars().next().map(char::len_utf8).unwrap_or(1);
            }
            continue;
        }
        let toggled = if rest.starts_with('`') {
            flush(&mut pending, run_start, i, st);
            st.code = true;
            run_start = i;
            1
        } else if rest.starts_with('$') && rest.len() > 1 && !rest[1..].starts_with(char::is_whitespace) {
            flush(&mut pending, run_start, i, st);
            st.math = true;
            run_start = i;
            if rest.starts_with("$$") { 2 } else { 1 }
        } else if rest.starts_with("[[") {
            flush(&mut pending, run_start, i, st);
            st.link = true;
            run_start = i;
            2
        } else if rest.starts_with("**") || rest.starts_with("__") {
            flush(&mut pending, run_start, i, st);
            st.bold = !st.bold;
            run_start = i;
            2
        } else if rest.starts_with("==") {
            flush(&mut pending, run_start, i, st);
            st.mark = !st.mark;
            run_start = i;
            2
        } else if rest.starts_with("~~") {
            flush(&mut pending, run_start, i, st);
            st.strike = !st.strike;
            run_start = i;
            2
        } else if (rest.starts_with('*') || rest.starts_with('_')) && emphasis_boundary(bytes, i, st.italic) {
            flush(&mut pending, run_start, i, st);
            st.italic = !st.italic;
            run_start = i;
            1
        } else if rest.starts_with('#') && tag_boundary(bytes, i) {
            let len = tag_len(rest);
            if len > 0 {
                flush(&mut pending, run_start, i, st);
                let mut tag = format_for(style, &base, st);
                tag.color = style.accent;
                pending.push((i, i + 1 + len, tag));
                i += 1 + len;
                run_start = i;
                continue;
            }
            0
        } else {
            0
        };
        if toggled == 0 {
            i += rest.chars().next().map(char::len_utf8).unwrap_or(1);
        } else {
            i += toggled;
        }
    }
    let stop = anchor_start.unwrap_or(content_end).max(run_start);
    flush(&mut pending, run_start, stop, st);
    if let Some(a) = anchor_start {
        let mut faint = base.clone();
        faint.color = style.faint;
        pending.push((a, content_end, faint));
    }
    if content_end < segment.len() {
        pending.push((content_end, segment.len(), base.clone()));
    }
    for (from, to, format) in pending {
        push(job, base_offset + from, base_offset + to, format);
    }
}

/// `*`/`_` counts as emphasis when opening before a non-space or closing
/// after a non-space — so `2 * 3` and snake_case stay plain.
fn emphasis_boundary(bytes: &[u8], i: usize, currently_italic: bool) -> bool {
    let next = bytes.get(i + 1).copied();
    let prev = if i == 0 { None } else { bytes.get(i - 1).copied() };
    if bytes[i] == b'_' {
        let prev_word = prev.is_some_and(|c| c.is_ascii_alphanumeric());
        let next_word = next.is_some_and(|c| c.is_ascii_alphanumeric());
        if prev_word && next_word {
            return false;
        }
    }
    if currently_italic {
        prev.is_some_and(|c| !c.is_ascii_whitespace())
    } else {
        next.is_some_and(|c| !c.is_ascii_whitespace() && c != b'*' && c != b'_')
    }
}

fn tag_boundary(bytes: &[u8], i: usize) -> bool {
    i == 0 || bytes[i - 1].is_ascii_whitespace() || b"([{\"'".contains(&bytes[i - 1])
}

fn tag_len(rest: &str) -> usize {
    let body = &rest[1..];
    let len: usize = body
        .chars()
        .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/'))
        .map(char::len_utf8)
        .sum();
    if len == 0 || body[..len].chars().all(|c| c.is_ascii_digit()) {
        0
    } else {
        len
    }
}

fn push(job: &mut LayoutJob, from: usize, to: usize, format: TextFormat) {
    if to <= from {
        return;
    }
    job.sections.push(LayoutSection {
        leading_space: 0.0,
        byte_range: ByteIndex(from)..ByteIndex(to),
        format,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> HighlightStyle {
        HighlightStyle {
            base_size: 15.0,
            text: Color32::WHITE,
            dim: Color32::GRAY,
            faint: Color32::DARK_GRAY,
            accent: Color32::LIGHT_BLUE,
            code_bg: Color32::BLACK,
            highlight_bg: Color32::YELLOW,
            semibold: FontFamily::Proportional,
            line_height: 24.0,
        }
    }

    fn covers_all(job: &LayoutJob) -> bool {
        let mut pos = 0;
        for s in &job.sections {
            if s.byte_range.start.0 != pos {
                return false;
            }
            pos = s.byte_range.end.0;
        }
        pos == job.text.len()
    }

    #[test]
    fn sections_tile_the_text_exactly() {
        let text = "# Judul\n\nTeks **tebal** dan *miring* `kode` [[Tautan|alias]] #tag ^abc12\n- [ ] tugas\n> kutipan\n```\nlet x = 1;\n```\n1. satu\n";
        let job = layout_job(text, 400.0, &style());
        assert!(covers_all(&job), "{:?}", job.sections.iter().map(|s| s.byte_range.clone()).collect::<Vec<_>>());
        assert!(job.sections.iter().all(|s| s.byte_range.end.0 <= job.text.len()));
    }

    #[test]
    fn headings_links_and_anchors_get_their_formats() {
        let st = style();
        let text = "## Bab\nlihat [[Catatan]] ^id1\n";
        let job = layout_job(text, 400.0, &st);
        let section_at = |byte: usize| job.sections.iter().find(|s| s.byte_range.contains(&ByteIndex(byte))).unwrap();
        assert!(section_at(3).format.font_id.size > st.base_size * 1.5);
        assert_eq!(section_at(text.find("[[").unwrap() + 2).format.color, st.accent);
        assert_eq!(section_at(text.find("^id1").unwrap()).format.color, st.faint);
        assert!(covers_all(&job));
    }

    #[test]
    fn heading_marks_share_the_title_line_height() {
        let job = layout_job("# 🚀 Judul ^8a6jg2\n", 400.0, &style());
        let heading: Vec<_> = job.sections.iter().filter(|s| s.byte_range.end.0 < job.text.len()).collect();
        let first = heading[0].format.line_height;
        assert!(heading.iter().all(|s| s.format.line_height == first && s.format.font_id.size == heading[0].format.font_id.size));
    }

    #[test]
    fn emphasis_is_not_triggered_inside_words_or_math() {
        let st = style();
        let job = layout_job("snake_case dan 2 * 3 = 6\n", 400.0, &st);
        assert!(job.sections.iter().all(|s| !s.format.italics));
        assert!(covers_all(&job));
    }

    #[test]
    fn unicode_and_unterminated_markup_never_panic() {
        let st = style();
        for text in ["**belum tutup", "`kode", "[[link", "Ünïcödé ^x", "", "\n\n", "> ", "#", "# "] {
            let job = layout_job(text, 100.0, &st);
            assert!(covers_all(&job), "{text:?}");
        }
    }
}
