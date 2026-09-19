//! Section segments (§3.9.1): a note body cut into non-overlapping,
//! anchored pieces that each become one box on the canvas —
//! a heading with its own prose up to the next heading or component
//! (`Section`), a table, a ```` ```mermaid ```` / code fence, and prose
//! that follows a component inside a section (`Text`). Heading nesting
//! gives every segment a parent, which is the mind-map outline.
//!
//! Identity is a `^id` anchor (Obsidian block id): on the heading line
//! for a section, on the last line for text, on its own line right after
//! a table/fence. Ranges never overlap, so editing one box can never
//! overwrite another box's text. Pure text logic — no `egui`, no IO.
//! Callers: `markdown::editor` (canvas sync), `canvas::outline_layout`,
//! `canvas::mermaid_export`, `api`.

use super::blocks::{generate_id, split_anchor};

/// What a segment holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentKind {
    /// `#`..`######` heading plus the prose under it.
    Section { level: u8 },
    Table,
    /// ```` ```mermaid ```` fence.
    Mermaid,
    /// Any other fence; `lang` is its info string (may be empty).
    Code { lang: String },
    /// Prose outside a heading's own run (before the first heading, or
    /// after a table/fence).
    Text,
}

impl SegmentKind {
    /// Tables and fences: kept verbatim, anchored on the line below.
    pub fn is_component(&self) -> bool {
        matches!(self, SegmentKind::Table | SegmentKind::Mermaid | SegmentKind::Code { .. })
    }
}

/// One anchored (or not yet anchored) piece of a note body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// Anchor id without `^`; `None` until [`anchor_all_segments`] runs.
    pub id: Option<String>,
    pub kind: SegmentKind,
    /// Index (into the same `Vec`) of the enclosing section.
    pub parent: Option<usize>,
    /// 0-based inclusive line range of the content (anchor-only lines of
    /// components excluded, trailing blank lines excluded).
    pub start_line: usize,
    pub end_line: usize,
    /// Line carrying the anchor, if any.
    pub anchor_line: Option<usize>,
    /// Content with the segment's own anchor removed.
    pub text: String,
}

impl Segment {
    /// A one-line label: heading text, first table row, diagram type,
    /// or the first prose line without list/quote markers.
    pub fn summary(&self) -> String {
        summary_of(&self.kind, &self.text)
    }
}

/// Label for a segment of `kind` holding `text` (see [`Segment::summary`]).
pub fn summary_of(kind: &SegmentKind, text: &str) -> String {
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let s = match kind {
        SegmentKind::Section { .. } => first.trim_start_matches('#').trim().to_string(),
        SegmentKind::Table => first
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .collect::<Vec<_>>()
            .join(" · "),
        SegmentKind::Mermaid => {
            let header = text.lines().skip(1).map(str::trim).find(|l| !l.is_empty() && !l.starts_with("%%"));
            format!("mermaid: {}", header.unwrap_or(""))
        }
        SegmentKind::Code { lang } => format!("code: {lang}"),
        SegmentKind::Text => strip_line_marker(first).to_string(),
    };
    truncate_chars(&s, 80)
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

fn strip_line_marker(line: &str) -> &str {
    let t = line.trim_start();
    for m in ["- [ ] ", "- [x] ", "- [X] ", "- ", "* ", "+ ", "> "] {
        if let Some(rest) = t.strip_prefix(m) {
            return rest;
        }
    }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && t[digits..].starts_with(". ") {
        return &t[digits + 2..];
    }
    t
}

/// Heading level of `line` (`# x` → 1), ignoring up to 3 spaces of indent.
pub fn heading_level(line: &str) -> Option<u8> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let t = &line[indent..];
    let hashes = t.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &t[hashes..];
    (rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')).then_some(hashes as u8)
}

/// `(marker, info)` when `line` opens a fence (```` ``` ```` / `~~~`, any length ≥ 3).
fn fence_open(line: &str) -> Option<(String, String)> {
    let t = line.trim_start();
    let ch = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let len = t.chars().take_while(|c| *c == ch).count();
    if len < 3 {
        return None;
    }
    let info = t[len..].trim();
    if ch == '`' && info.contains('`') {
        return None;
    }
    Some((t[..len].to_string(), info.split_whitespace().next().unwrap_or("").to_string()))
}

fn fence_closes(line: &str, marker: &str) -> bool {
    let t = line.trim();
    let ch = marker.chars().next().unwrap_or('`');
    t.len() >= marker.len() && t.chars().all(|c| c == ch)
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

fn is_table_row(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

/// A line that is only `^id`.
fn standalone_anchor(line: &str) -> Option<&str> {
    match split_anchor(line) {
        (text, Some(id)) if text.trim().is_empty() => Some(id),
        _ => None,
    }
}

/// Every segment of `body`, in document order.
pub fn segments(body: &str) -> Vec<Segment> {
    let lines: Vec<&str> = body.lines().collect();
    let mut out: Vec<Segment> = Vec::new();
    // Section/Text segment currently absorbing prose lines.
    let mut open: Option<usize> = None;
    // (level, index) of the enclosing headings.
    let mut stack: Vec<(u8, usize)> = Vec::new();
    let push = |out: &mut Vec<Segment>, kind: SegmentKind, parent: Option<usize>, start: usize, end: usize| {
        out.push(Segment {
            id: None,
            kind,
            parent,
            start_line: start,
            end_line: end,
            anchor_line: None,
            text: String::new(),
        });
        out.len() - 1
    };

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let section = stack.last().map(|(_, idx)| *idx);
        if let Some(level) = heading_level(line) {
            while stack.last().is_some_and(|(l, _)| *l >= level) {
                stack.pop();
            }
            let parent = stack.last().map(|(_, idx)| *idx);
            let idx = push(&mut out, SegmentKind::Section { level }, parent, i, i);
            out[idx].anchor_line = Some(i);
            stack.push((level, idx));
            open = Some(idx);
            i += 1;
            continue;
        }
        let component = if let Some((marker, lang)) = fence_open(line) {
            let close = (i + 1..lines.len()).find(|&j| fence_closes(lines[j], &marker));
            let end = close.unwrap_or(lines.len() - 1);
            let kind = if lang == "mermaid" { SegmentKind::Mermaid } else { SegmentKind::Code { lang } };
            Some((kind, end))
        } else if is_table_row(line) {
            let mut end = i;
            while end + 1 < lines.len() && is_table_row(lines[end + 1]) {
                end += 1;
            }
            Some((SegmentKind::Table, end))
        } else {
            None
        };
        if let Some((kind, end)) = component {
            let idx = push(&mut out, kind, section, i, end);
            let mut next = end + 1;
            if let Some(l) = lines.get(end + 1)
                && standalone_anchor(l).is_some()
            {
                out[idx].anchor_line = Some(end + 1);
                next = end + 2;
            }
            open = None;
            i = next;
            continue;
        }
        if !is_blank(line) {
            match open {
                Some(idx) => out[idx].end_line = i,
                None => {
                    let idx = push(&mut out, SegmentKind::Text, section, i, i);
                    open = Some(idx);
                }
            }
        }
        i += 1;
    }

    for seg in &mut out {
        if seg.kind == SegmentKind::Text {
            seg.anchor_line = [seg.end_line, seg.start_line]
                .into_iter()
                .find(|&l| split_anchor(lines[l]).1.is_some());
        }
        seg.id = seg.anchor_line.and_then(|l| split_anchor(lines[l]).1).map(str::to_string);
        let mut text: Vec<&str> = Vec::new();
        for (l, line) in lines.iter().enumerate().take(seg.end_line + 1).skip(seg.start_line) {
            if Some(l) == seg.anchor_line && seg.id.is_some() {
                text.push(split_anchor(line).0);
            } else {
                text.push(line);
            }
        }
        seg.text = text.join("\n");
    }
    out
}

/// The segment anchored `^id`.
pub fn find_segment(body: &str, id: &str) -> Option<Segment> {
    segments(body).into_iter().find(|s| s.id.as_deref() == Some(id))
}

/// Content of the segment anchored `^id`.
pub fn segment_text(body: &str, id: &str) -> Option<String> {
    find_segment(body, id).map(|s| s.text)
}

/// The anchored segment whose range (anchor line included) holds 0-based `line`.
pub fn segment_at_line(body: &str, line: usize) -> Option<Segment> {
    segments(body).into_iter().find(|s| {
        s.id.is_some() && s.start_line <= line && line <= s.anchor_line.unwrap_or(s.end_line).max(s.end_line)
    })
}

fn join_lines(lines: Vec<String>, body: &str) -> String {
    let mut joined = lines.join("\n");
    if body.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

/// Replaces the content of the segment `^id` with `new_text`, keeping its
/// anchor (heading line, last prose line, or the line under a component).
/// An unclosed fence gets its closing marker back. `None` when the anchor
/// doesn't exist, the text is unchanged, or `new_text` is blank (removing a
/// segment is an explicit action, never a side effect of an edit).
pub fn replace_segment_text(body: &str, id: &str, new_text: &str) -> Option<String> {
    let seg = find_segment(body, id)?;
    let new_text = new_text.trim_end().trim_start_matches(['\n', '\r']);
    if new_text.trim().is_empty() || seg.text == new_text {
        return None;
    }
    let mut replacement: Vec<String> = new_text.lines().map(str::to_string).collect();
    match &seg.kind {
        SegmentKind::Section { .. } => {
            replacement[0] = format!("{} ^{id}", replacement[0].trim_end());
        }
        SegmentKind::Text => {
            let last = replacement.len() - 1;
            replacement[last] = format!("{} ^{id}", replacement[last].trim_end());
        }
        SegmentKind::Mermaid | SegmentKind::Code { .. } => {
            if let Some((marker, _)) = fence_open(&replacement[0])
                && !replacement[1..].iter().any(|l| fence_closes(l, &marker))
            {
                replacement.push(marker);
            }
        }
        SegmentKind::Table => {}
    }
    let lines: Vec<&str> = body.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + replacement.len());
    out.extend(lines[..seg.start_line].iter().map(|l| l.to_string()));
    out.extend(replacement);
    out.extend(lines[seg.end_line + 1..].iter().map(|l| l.to_string()));
    Some(join_lines(out, body))
}

/// Every `^id` used anywhere in `body`.
fn taken_ids(body: &str) -> Vec<String> {
    body.lines().filter_map(|l| split_anchor(l).1.map(str::to_string)).collect()
}

/// Gives every segment without an anchor one, returning the new body and
/// the (all anchored) segments. Idempotent.
pub fn anchor_all_segments(body: &str) -> (String, Vec<Segment>) {
    let mut body = body.to_string();
    // Each round anchors one segment; the bound keeps a bug from looping.
    for _ in 0..=body.lines().count() {
        let segs = segments(&body);
        let Some(seg) = segs.iter().find(|s| s.id.is_none()) else {
            return (body, segs);
        };
        let id = generate_id(&taken_ids(&body));
        let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
        match seg.kind {
            SegmentKind::Section { .. } => {
                let l = seg.start_line;
                lines[l] = format!("{} ^{id}", lines[l].trim_end());
            }
            SegmentKind::Text => {
                let l = seg.end_line;
                lines[l] = format!("{} ^{id}", lines[l].trim_end());
            }
            _ => lines.insert(seg.end_line + 1, format!("^{id}")),
        }
        body = join_lines(lines, &body);
    }
    log::warn!("sections: gave up anchoring segments");
    let segs = segments(&body);
    (body, segs)
}

/// Removes the segment `^id` (content and anchor line) from `body`.
pub fn remove_segment(body: &str, id: &str) -> Option<String> {
    let seg = find_segment(body, id)?;
    let last = seg.anchor_line.unwrap_or(seg.end_line).max(seg.end_line);
    let lines: Vec<&str> = body.lines().collect();
    let mut out: Vec<String> = lines[..seg.start_line].iter().map(|l| l.to_string()).collect();
    let mut rest = last + 1;
    // Drop one blank separator so removal doesn't leave a double gap.
    if rest < lines.len() && is_blank(lines[rest]) && out.last().is_none_or(|l| is_blank(l)) {
        rest += 1;
    }
    out.extend(lines[rest..].iter().map(|l| l.to_string()));
    while out.last().is_some_and(|l| is_blank(l)) {
        out.pop();
    }
    Some(join_lines(out, body))
}

/// Appends `text` as a new anchored segment at the end of `body` and
/// returns the new body and id. A text starting with `#` becomes a section.
pub fn append_segment(body: &str, text: &str) -> (String, String) {
    let id = generate_id(&taken_ids(body));
    let mut out = body.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    let text = text.trim();
    let text = if text.is_empty() { "…" } else { text };
    let first = text.lines().next().unwrap_or("");
    if fence_open(first).is_some() || is_table_row(first) {
        out.push_str(text);
        out.push_str(&format!("\n^{id}\n"));
    } else if heading_level(first).is_some() {
        out.push_str(&format!("{first} ^{id}"));
        for l in text.lines().skip(1) {
            out.push('\n');
            out.push_str(l);
        }
        out.push('\n');
    } else {
        out.push_str(&format!("{text} ^{id}\n"));
    }
    (out, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "Pembuka catatan.\n\n# Produk ^p1\nVisi singkat.\n\n- poin a\n- poin b\n\n## Data ^d1\nPenjelasan data.\n\n| Tabel | Kolom |\n|---|---|\n| a | b |\n^t1\n\nSesudah tabel. ^x1\n\n```mermaid\nerDiagram\n  A ||--o{ B : has\n```\n^m1\n\n## Tim ^tm\nOrang.\n";

    #[test]
    fn segments_split_sections_components_and_trailing_text() {
        let segs = segments(DOC);
        let kinds: Vec<_> = segs.iter().map(|s| (s.id.clone(), s.kind.clone(), s.parent)).collect();
        assert_eq!(
            kinds,
            vec![
                (None, SegmentKind::Text, None),
                (Some("p1".into()), SegmentKind::Section { level: 1 }, None),
                (Some("d1".into()), SegmentKind::Section { level: 2 }, Some(1)),
                (Some("t1".into()), SegmentKind::Table, Some(2)),
                (Some("x1".into()), SegmentKind::Text, Some(2)),
                (Some("m1".into()), SegmentKind::Mermaid, Some(2)),
                (Some("tm".into()), SegmentKind::Section { level: 2 }, Some(1)),
            ]
        );
        assert_eq!(segs[1].text, "# Produk\nVisi singkat.\n\n- poin a\n- poin b");
        assert_eq!(segs[3].text, "| Tabel | Kolom |\n|---|---|\n| a | b |");
        assert_eq!(segs[4].text, "Sesudah tabel.");
        assert!(segs[5].text.starts_with("```mermaid\nerDiagram"));
        assert_eq!(segs[1].summary(), "Produk");
        assert_eq!(segs[3].summary(), "Tabel · Kolom");
        assert_eq!(segs[5].summary(), "mermaid: erDiagram");
    }

    #[test]
    fn replace_keeps_anchor_and_other_segments() {
        let out = replace_segment_text(DOC, "p1", "# Produk Baru\nVisi baru.").unwrap();
        assert!(out.contains("# Produk Baru ^p1\nVisi baru.\n\n## Data ^d1"), "{out}");
        assert_eq!(segment_text(&out, "d1").as_deref(), Some("## Data\nPenjelasan data."));
        let out = replace_segment_text(&out, "t1", "| X |\n|---|").unwrap();
        assert!(out.contains("| X |\n|---|\n^t1\n"), "{out}");
        let out = replace_segment_text(&out, "x1", "Dua\nbaris").unwrap();
        assert!(out.contains("Dua\nbaris ^x1\n"), "{out}");
        // Unclosed fence gets closed again.
        let out = replace_segment_text(&out, "m1", "```mermaid\nflowchart LR\n  A-->B").unwrap();
        assert!(out.contains("  A-->B\n```\n^m1"), "{out}");
        assert!(replace_segment_text(&out, "p1", "   ").is_none());
        assert!(replace_segment_text(&out, "nope", "x").is_none());
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn anchor_all_is_idempotent_and_anchors_each_kind() {
        let body = "Intro\n\n# A\nisi\n\n| t |\n|---|\n\n```rust\nfn x() {}\n```\nlanjut\n";
        let (out, segs) = anchor_all_segments(body);
        assert_eq!(segs.len(), 5, "{out}");
        assert!(segs.iter().all(|s| s.id.is_some()));
        let (again, _) = anchor_all_segments(&out);
        assert_eq!(again, out);
        // Code fence anchored on its own line after the closing marker.
        let code = segs.iter().find(|s| matches!(s.kind, SegmentKind::Code { .. })).unwrap();
        assert_eq!(code.text, "```rust\nfn x() {}\n```");
    }

    #[test]
    fn heading_nesting_and_level_skips() {
        let segs = segments("# A ^a\n### C ^c\n## B ^b\n# D ^d\n");
        let parents: Vec<_> = segs.iter().map(|s| s.parent).collect();
        assert_eq!(parents, vec![None, Some(0), Some(0), None]);
        assert_eq!(heading_level("#tag"), None);
        assert_eq!(heading_level("#"), Some(1));
    }

    #[test]
    fn segment_at_line_includes_component_anchor_line() {
        let s = segment_at_line(DOC, 14).unwrap();
        assert_eq!(s.id.as_deref(), Some("t1"));
        assert_eq!(segment_at_line(DOC, 3).unwrap().id.as_deref(), Some("p1"));
    }

    #[test]
    fn append_and_remove_round_trip() {
        let (body, id) = append_segment("# A ^a\nisi\n", "## Baru\nteks");
        assert_eq!(segment_text(&body, &id).as_deref(), Some("## Baru\nteks"));
        let removed = remove_segment(&body, &id).unwrap();
        assert_eq!(removed, "# A ^a\nisi\n");
        let (body, id) = append_segment("x ^x", "| a |\n|---|");
        assert_eq!(segment_text(&body, &id).as_deref(), Some("| a |\n|---|"));
    }
}
