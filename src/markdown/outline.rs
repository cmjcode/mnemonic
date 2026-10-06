//! Heading outline and addressable parts of a note (§3.10.2): every
//! heading with its full section range (up to the next heading of the
//! same or a higher level, subsections included — what Obsidian's
//! `[[Note#Heading]]` embed shows), plus the pure edits agents use to read
//! or change one part without resending the whole note: select a section
//! (`Heading`, `Parent#Child`, `^anchor`) or an anchored block (`^id`),
//! replace it, append inside it or at the end. Fence-aware; line endings
//! (LF/CRLF) and the trailing newline are kept. Pure text logic — no
//! `egui`, no IO. Callers: `api::memory`.

use anyhow::{Result, bail};

use super::blocks::{self, split_anchor};
use super::sections::heading_level;

/// One Markdown heading and the section it opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    /// Heading text without `#` markers, closing hashes or `^anchor`.
    pub text: String,
    /// `^id` on the heading line, without the caret.
    pub anchor: Option<String>,
    /// 0-based line of the heading.
    pub line: usize,
    /// 0-based exclusive end of the section.
    pub end: usize,
    /// Texts of the enclosing headings, outermost first, then this one.
    pub path: Vec<String>,
}

impl Heading {
    /// `Parent#Child` — accepted back by [`select`].
    pub fn path_string(&self) -> String {
        self.path.join("#")
    }
}

/// What a [`Selection`] points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionKind {
    /// A heading and everything under it.
    Section,
    /// A `^id`-anchored block (paragraph, list item, table…).
    Block,
}

/// A part of a note body addressed by [`select`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub kind: SelectionKind,
    /// 0-based first line (the heading line for a section).
    pub start: usize,
    /// 0-based exclusive end.
    pub end: usize,
    /// Heading path (`A#B`) of a section, `^id` of a block.
    pub label: String,
    /// Anchor id of a block selection.
    pub block_id: Option<String>,
}

/// Heading text without a CommonMark closing sequence (` ##`).
fn strip_closing_hashes(text: &str) -> &str {
    let trimmed = text.trim_end_matches('#');
    if trimmed.len() < text.len() && (trimmed.is_empty() || trimmed.ends_with([' ', '\t'])) {
        trimmed.trim_end()
    } else {
        text
    }
}

/// Every heading outside fenced code, in document order.
pub fn headings(body: &str) -> Vec<Heading> {
    let lines: Vec<&str> = body.lines().collect();
    let mut out: Vec<Heading> = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    let mut in_fence = false;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let Some(level) = heading_level(line) else {
            continue;
        };
        let (text, anchor) = split_anchor(line);
        let text = strip_closing_hashes(text.trim().trim_start_matches('#').trim()).to_string();
        while let Some(&top) = open.last() {
            if out[top].level < level {
                break;
            }
            out[top].end = i;
            open.pop();
        }
        let mut path: Vec<String> = open.iter().map(|&j| out[j].text.clone()).collect();
        path.push(text.clone());
        open.push(out.len());
        out.push(Heading {
            level,
            text,
            anchor: anchor.map(str::to_string),
            line: i,
            end: lines.len(),
            path,
        });
    }
    out
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// `true` when `wanted` appears in `path` in order (not necessarily
/// adjacent), ending with `path`'s last entry.
fn path_matches(path: &[String], wanted: &[String]) -> bool {
    let (Some(last), Some(want_last)) = (path.last(), wanted.last()) else {
        return false;
    };
    if norm(last) != *want_last {
        return false;
    }
    let mut ancestors = path[..path.len() - 1].iter();
    wanted[..wanted.len() - 1]
        .iter()
        .all(|w| ancestors.by_ref().any(|a| norm(a) == *w))
}

/// Resolves `spec` in `body`: `^id` (a heading carrying that anchor, else
/// an anchored block), or a heading given as `Heading`, `## Heading` or a
/// `Parent#Child` path (case- and whitespace-insensitive). Errors say what
/// exists when nothing matches, and list the candidates when several do.
pub fn select(body: &str, spec: &str) -> Result<Selection> {
    let spec = spec.trim();
    let all = headings(body);
    if let Some(id) = spec.strip_prefix('^') {
        if let Some(h) = all.iter().find(|h| h.anchor.as_deref() == Some(id)) {
            return Ok(section_selection(h));
        }
        if let Some(b) = blocks::block_anchors(body).into_iter().find(|b| b.id == id) {
            return Ok(Selection {
                kind: SelectionKind::Block,
                start: b.start_line,
                end: b.end_line + 1,
                label: format!("^{id}"),
                block_id: Some(id.to_string()),
            });
        }
        bail!("no block or heading anchored `^{id}` in this note");
    }
    let wanted: Vec<String> = spec
        .trim_start_matches('#')
        .split('#')
        .map(norm)
        .filter(|s| !s.is_empty())
        .collect();
    if wanted.is_empty() {
        bail!("empty section reference");
    }
    let found: Vec<&Heading> = all.iter().filter(|h| path_matches(&h.path, &wanted)).collect();
    match found.as_slice() {
        [h] => Ok(section_selection(h)),
        [] => {
            let known: Vec<String> = all.iter().take(30).map(Heading::path_string).collect();
            if known.is_empty() {
                bail!("section `{spec}` not found: the note has no headings");
            }
            bail!("section `{spec}` not found; headings: {}", known.join(", "))
        }
        many => bail!(
            "section `{spec}` is ambiguous: {}; use `Parent#Child` or a ^anchor",
            many.iter()
                .map(|h| format!("{} (line {})", h.path_string(), h.line + 1))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn section_selection(h: &Heading) -> Selection {
    Selection {
        kind: SelectionKind::Section,
        start: h.line,
        end: h.end,
        label: h.path_string(),
        block_id: None,
    }
}

/// The selected lines (heading included), trailing blank lines dropped.
pub fn selection_text(body: &str, sel: &Selection) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let end = sel.end.min(lines.len());
    let mut part: Vec<&str> = lines[sel.start.min(end)..end].to_vec();
    while part.last().is_some_and(|l| l.trim().is_empty()) {
        part.pop();
    }
    part.join("\n")
}

fn eol(body: &str) -> &'static str {
    if body.contains("\r\n") { "\r\n" } else { "\n" }
}

fn is_list_item(line: &str) -> bool {
    let t = line.trim_start();
    ["- ", "* ", "+ "].iter().any(|m| t.starts_with(m))
        || t.chars().take_while(char::is_ascii_digit).count() > 0
            && t.trim_start_matches(|c: char| c.is_ascii_digit()).starts_with(". ")
}

fn text_lines(text: &str) -> Vec<String> {
    text.trim_matches(['\n', '\r']).lines().map(str::to_string).collect()
}

/// Rebuilds `body` from `lines`, in its line-ending style and keeping (or
/// adding) the trailing newline.
fn join(body: &str, lines: &[String]) -> String {
    let mut out = lines.join(eol(body));
    if !out.is_empty() && (body.ends_with('\n') || body.is_empty()) {
        out.push_str(eol(body));
    }
    out
}

/// Inserts `text` after line `after` (0-based; `None` = at the top),
/// separated by a blank line unless both sides are list items.
fn insert_after(lines: &mut Vec<String>, after: Option<usize>, text: &str) {
    let new = text_lines(text);
    if new.is_empty() {
        return;
    }
    let at = after.map_or(0, |a| a + 1);
    let mut chunk: Vec<String> = Vec::new();
    let joins_list = |a: &str, b: &str| is_list_item(a) && is_list_item(b);
    if let Some(prev) = after.map(|a| lines[a].as_str())
        && !prev.trim().is_empty()
        && !joins_list(prev, &new[0])
    {
        chunk.push(String::new());
    }
    let last_new = new.last().cloned().unwrap_or_default();
    chunk.extend(new);
    if let Some(next) = lines.get(at)
        && !next.trim().is_empty()
        && !joins_list(&last_new, next)
    {
        chunk.push(String::new());
    }
    lines.splice(at..at, chunk);
}

/// Last non-blank line in `[from, to)`, if any.
fn last_content_line(lines: &[String], from: usize, to: usize) -> Option<usize> {
    (from..to.min(lines.len())).rev().find(|&i| !lines[i].trim().is_empty())
}

/// Replaces what `sel` covers with `text`: a section keeps its heading
/// line (subsections are part of the section and are replaced too); a
/// block keeps its `^id` anchor and list marker.
pub fn replace_selection(body: &str, sel: &Selection, text: &str) -> String {
    if let Some(id) = &sel.block_id {
        return blocks::replace_block_text(body, id, text).unwrap_or_else(|| body.to_string());
    }
    let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
    let end = sel.end.min(lines.len());
    let blank_after_heading = lines.get(sel.start + 1).is_some_and(|l| l.trim().is_empty());
    let mut chunk: Vec<String> = Vec::new();
    let new = text_lines(text);
    if !new.is_empty() {
        if blank_after_heading {
            chunk.push(String::new());
        }
        chunk.extend(new);
    }
    if end < lines.len() {
        chunk.push(String::new());
    }
    lines.splice(sel.start + 1..end, chunk);
    join(body, &lines)
}

/// Appends `text` at the end of what `sel` covers (after a section's last
/// content line, before the next heading; after a block).
pub fn append_to_selection(body: &str, sel: &Selection, text: &str) -> String {
    let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
    let after = match sel.kind {
        SelectionKind::Section => last_content_line(&lines, sel.start, sel.end).or(Some(sel.start)),
        SelectionKind::Block => Some(sel.end.saturating_sub(1).min(lines.len().saturating_sub(1))),
    };
    insert_after(&mut lines, after, text);
    join(body, &lines)
}

/// Appends `text` after the note's last non-blank line.
pub fn append_to_end(body: &str, text: &str) -> String {
    let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
    let after = last_content_line(&lines, 0, lines.len());
    lines.truncate(after.map_or(0, |a| a + 1));
    insert_after(&mut lines, after, text);
    join(body, &lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "Intro.\n\n# Proyek\nVisi.\n\n## Status ^st\n- a\n- b\n\n### Detail\nx\n\n## Tim\nOrang.\n```\n# bukan heading\n```\n";

    #[test]
    fn headings_have_nested_ranges_and_paths() {
        let hs = headings(NOTE);
        let paths: Vec<String> = hs.iter().map(Heading::path_string).collect();
        assert_eq!(paths, vec!["Proyek", "Proyek#Status", "Proyek#Status#Detail", "Proyek#Tim"]);
        assert_eq!((hs[1].line, hs[1].end), (5, 12));
        assert_eq!(hs[1].anchor.as_deref(), Some("st"));
        assert_eq!(hs[0].end, NOTE.lines().count());
        assert_eq!(headings("## C#\n## Judul ##\n")[0].text, "C#");
        assert_eq!(headings("## Judul ##\n")[0].text, "Judul");
    }

    #[test]
    fn select_by_name_path_anchor_and_block() {
        let s = select(NOTE, "status").unwrap();
        assert_eq!(s.kind, SelectionKind::Section);
        assert_eq!(selection_text(NOTE, &s), "## Status ^st\n- a\n- b\n\n### Detail\nx");
        assert_eq!(select(NOTE, "## Proyek#Detail").unwrap().label, "Proyek#Status#Detail");
        assert_eq!(select(NOTE, "^st").unwrap().label, "Proyek#Status");
        let body = "p satu ^p1\n\n- item ^i1\n";
        let b = select(body, "^i1").unwrap();
        assert_eq!((b.kind, b.start, b.end), (SelectionKind::Block, 2, 3));
        let err = select(NOTE, "Nope").unwrap_err().to_string();
        assert!(err.contains("Proyek#Tim"), "{err}");
        let dup = "# A\n## X\n# B\n## X\n";
        assert!(select(dup, "X").unwrap_err().to_string().contains("ambiguous"));
        assert_eq!(select(dup, "B#X").unwrap().start, 3);
    }

    #[test]
    fn replace_section_keeps_heading_and_spacing() {
        let s = select(NOTE, "Status").unwrap();
        let out = replace_selection(NOTE, &s, "- selesai\n");
        assert!(out.contains("## Status ^st\n- selesai\n\n## Tim\n"), "{out}");
        assert!(!out.contains("### Detail"));
        let crlf = "# A\r\nlama\r\n# B\r\nx\r\n";
        let out = replace_selection(crlf, &select(crlf, "A").unwrap(), "baru");
        assert_eq!(out, "# A\r\nbaru\r\n\r\n# B\r\nx\r\n");
    }

    #[test]
    fn replace_block_keeps_anchor() {
        let body = "- lama ^i1\n";
        let out = replace_selection(body, &select(body, "^i1").unwrap(), "baru");
        assert_eq!(out, "- baru ^i1\n");
    }

    #[test]
    fn append_inside_section_list_and_at_end() {
        let s = select(NOTE, "Proyek#Status#Detail").unwrap();
        let out = append_to_selection(NOTE, &s, "y");
        assert!(out.contains("### Detail\nx\n\ny\n\n## Tim"), "{out}");
        let list = "## Todo\n- a\n\n## Lain\n";
        let out = append_to_selection(list, &select(list, "Todo").unwrap(), "- b");
        assert_eq!(out, "## Todo\n- a\n- b\n\n## Lain\n");
        assert_eq!(append_to_end("satu\n\n\n", "dua"), "satu\n\ndua\n");
        assert_eq!(append_to_end("", "dua"), "dua\n");
        assert_eq!(append_to_end("- a", "- b"), "- a\n- b");
    }
}
