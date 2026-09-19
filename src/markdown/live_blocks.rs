//! Splits a note body into the blocks of the Live editor (§3.2.1 "Live
//! Preview"): the note is always shown rendered, and clicking a block turns
//! just that block back into raw Markdown. Most blocks are a single source
//! line (headings, paragraph lines, list items, quote lines); constructs
//! that only make sense whole — fenced code, Mermaid, `$$` math, tables and
//! callouts — are one atomic block spanning all their lines.
//!
//! Also holds the pure text operations the Live editor needs: replacing a
//! line range (`replace_lines`, keeping the file's line endings) and list
//! continuation on Enter (`continue_list`). Pure and egui-free.
//! Callers: `markdown::renderer` (drawing), `app::editor::live` (editing).

use std::ops::Range;

/// What a block is, which decides how it is rendered and edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Blank,
    Heading(u8),
    Paragraph,
    ListItem,
    Checklist { checked: bool },
    Quote,
    Rule,
    /// `> [!type] Title` and the `>` lines under it.
    Callout,
    Table,
    /// A closed ```` ``` ```` / `~~~` fence (including `canvas` fences).
    Code,
    /// A closed ```` ```mermaid ```` fence.
    Mermaid,
    /// A `$$` … `$$` display-math block spanning several lines.
    Math,
}

impl BlockKind {
    /// Atomic blocks are edited as a whole instead of line by line.
    pub fn is_atomic(self) -> bool {
        matches!(self, BlockKind::Callout | BlockKind::Table | BlockKind::Code | BlockKind::Mermaid | BlockKind::Math)
    }
}

/// One Live-editor block: a 0-based, end-exclusive range of body lines
/// (as `str::lines` counts them) and its kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveBlock {
    pub lines: Range<usize>,
    pub kind: BlockKind,
    /// Leading whitespace of the first line in columns (tab = 4), used to
    /// indent nested list items and continuation lines.
    pub indent: usize,
}

/// Splits `body` into Live blocks, in order, covering every line.
pub fn split_blocks(body: &str) -> Vec<LiveBlock> {
    let lines: Vec<&str> = body.lines().collect();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let (kind, end) = classify(&lines, i);
        blocks.push(LiveBlock { lines: i..end, kind, indent: indent_of(line) });
        i = end;
    }
    blocks
}

/// Kind of the block starting at line `i` and the line it ends before.
fn classify(lines: &[&str], i: usize) -> (BlockKind, usize) {
    let line = lines[i];
    let t = line.trim_start();

    if let Some(marker) = crate::mermaid::fence_open(line)
        && let Some(end) = (i + 1..lines.len()).find(|&j| crate::mermaid::fence_close(lines[j], marker))
    {
        return (BlockKind::Mermaid, end + 1);
    }
    if let Some((ch, len)) = fence_marker(t)
        && let Some(end) = (i + 1..lines.len()).find(|&j| closes_fence(lines[j], ch, len))
    {
        return (BlockKind::Code, end + 1);
    }
    if t.trim_end() == "$$"
        && let Some(end) = (i + 1..lines.len()).find(|&j| lines[j].trim() == "$$")
    {
        return (BlockKind::Math, end + 1);
    }
    if is_callout_start(t) {
        let end = (i + 1..lines.len()).find(|&j| !lines[j].trim_start().starts_with('>')).unwrap_or(lines.len());
        return (BlockKind::Callout, end);
    }
    if t.contains('|') && lines.get(i + 1).is_some_and(|next| is_table_delimiter(next)) {
        let end = (i + 2..lines.len())
            .find(|&j| lines[j].trim().is_empty() || !lines[j].contains('|'))
            .unwrap_or(lines.len());
        return (BlockKind::Table, end);
    }

    let kind = if t.trim().is_empty() {
        BlockKind::Blank
    } else if let Some(level) = heading_level(t) {
        BlockKind::Heading(level)
    } else if is_rule(t) {
        BlockKind::Rule
    } else if let Some(checked) = checklist_state(t) {
        BlockKind::Checklist { checked }
    } else if list_marker_len(t).is_some() {
        BlockKind::ListItem
    } else if t.starts_with('>') {
        BlockKind::Quote
    } else {
        BlockKind::Paragraph
    };
    (kind, i + 1)
}

/// Opening fence char and length (```` ``` ````/`~~~`, 3 or more).
fn fence_marker(t: &str) -> Option<(char, usize)> {
    let ch = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let len = t.chars().take_while(|c| *c == ch).count();
    (len >= 3).then_some((ch, len))
}

fn closes_fence(line: &str, ch: char, len: usize) -> bool {
    let t = line.trim();
    !t.is_empty() && t.chars().all(|c| c == ch) && t.chars().count() >= len
}

fn is_callout_start(t: &str) -> bool {
    t.strip_prefix('>').is_some_and(|rest| rest.trim_start().starts_with("[!"))
}

/// A GFM table delimiter row such as `| --- | :---: |`.
fn is_table_delimiter(line: &str) -> bool {
    let t = line.trim().trim_matches('|');
    if !t.contains('-') {
        return false;
    }
    t.split('|').all(|cell| {
        let c = cell.trim();
        !c.is_empty() && c.trim_matches(':').chars().all(|ch| ch == '-') && c.contains('-')
    })
}

/// Level of an ATX heading (`#` … `######` followed by a space or the end).
pub fn heading_level(t: &str) -> Option<u8> {
    let hashes = t.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &t[hashes..];
    (rest.is_empty() || rest.starts_with(' ')).then_some(hashes as u8)
}

fn is_rule(t: &str) -> bool {
    let compact: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && ['-', '*', '_'].iter().any(|m| compact.chars().all(|c| c == *m))
}

fn checklist_state(t: &str) -> Option<bool> {
    let rest = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ "))?;
    if rest.starts_with("[ ]") {
        Some(false)
    } else if rest.starts_with("[x]") || rest.starts_with("[X]") {
        Some(true)
    } else {
        None
    }
}

/// Byte length of a list marker plus its space (`- `, `* `, `+ `, `12. `,
/// `3) `) at the start of `t`, or `None` if `t` isn't a list item.
fn list_marker_len(t: &str) -> Option<usize> {
    if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") {
        return Some(2);
    }
    if matches!(t, "-" | "*" | "+") {
        return Some(1);
    }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 9 {
        return None;
    }
    let rest = &t[digits..];
    if rest.starts_with(". ") || rest.starts_with(") ") {
        Some(digits + 2)
    } else if rest == "." || rest == ")" {
        Some(digits + 1)
    } else {
        None
    }
}

/// Leading whitespace of `line` in columns (tab = 4).
pub fn indent_of(line: &str) -> usize {
    line.chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
}

/// Number of lines `str::lines` sees in `body`.
pub fn line_count(body: &str) -> usize {
    body.lines().count()
}

/// The text of `range` (clamped to the body), lines joined by `\n`.
pub fn lines_text(body: &str, range: Range<usize>) -> String {
    body.lines()
        .skip(range.start)
        .take(range.end.saturating_sub(range.start))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Replaces the lines in `range` with `replacement` (lines separated by
/// `\n`). A range past the end appends; an empty range inserts. The body's
/// line ending (`\r\n` or `\n`) and trailing newline are kept.
pub fn replace_lines(body: &str, range: Range<usize>, replacement: &str) -> String {
    let eol = if body.contains("\r\n") { "\r\n" } else { "\n" };
    let ends_with_newline = body.ends_with('\n');
    let mut lines: Vec<&str> = body.lines().collect();
    let start = range.start.min(lines.len());
    let end = range.end.clamp(start, lines.len());
    lines.splice(start..end, replacement.split('\n').map(|l| l.trim_end_matches('\r')));
    let mut out = lines.join(eol);
    if ends_with_newline {
        out.push_str(eol);
    }
    out
}

/// What Enter at the end of a list item should put on the new line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListContinuation {
    /// Not a list item: a plain new line.
    None,
    /// Start the new line with this prefix (indent + marker).
    Continue(String),
    /// The item was empty: drop its marker instead of continuing the list.
    EndList,
}

/// Obsidian-style list continuation for `line` (the line Enter was
/// pressed on): `- a` → `- `, `- [x] a` → `- [ ] `, `3. a` → `4. `,
/// nesting kept; an item with no text ends the list.
pub fn continue_list(line: &str) -> ListContinuation {
    let indent_len = line.len() - line.trim_start().len();
    let (indent, t) = line.split_at(indent_len);
    let Some(marker_len) = list_marker_len(t) else {
        return ListContinuation::None;
    };
    let marker = t[..marker_len].trim_end();
    let mut rest = t[marker_len..].trim_start();
    let is_task = checklist_state(t).is_some();
    if is_task {
        rest = rest[3..].trim_start();
    }
    if rest.is_empty() {
        return ListContinuation::EndList;
    }
    let next_marker = match marker.strip_suffix('.').or_else(|| marker.strip_suffix(')')) {
        Some(num) => {
            let n: u64 = num.parse().unwrap_or(0);
            format!("{}{}", n + 1, &marker[num.len()..])
        }
        None => marker.to_string(),
    };
    let task = if is_task { " [ ]" } else { "" };
    ListContinuation::Continue(format!("{indent}{next_marker}{task} "))
}

/// The block text to render for `block`: lines of a non-atomic block with
/// their indentation removed (the renderer indents them itself, and four
/// leading spaces would otherwise turn a nested item into a code block).
pub fn block_source(body_lines: &[&str], block: &LiveBlock) -> String {
    let lines = &body_lines[block.lines.clone()];
    if block.kind.is_atomic() {
        return lines.join("\n");
    }
    lines.iter().map(|l| l.trim_start()).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(body: &str) -> Vec<(Range<usize>, BlockKind)> {
        split_blocks(body).into_iter().map(|b| (b.lines, b.kind)).collect()
    }

    #[test]
    fn one_block_per_line_for_prose_lists_and_headings() {
        let body = "# Judul\nparagraf satu\nparagraf dua\n\n- a\n  - b\n1. c\n- [x] d\n> kutip\n---";
        assert_eq!(
            kinds(body),
            vec![
                (0..1, BlockKind::Heading(1)),
                (1..2, BlockKind::Paragraph),
                (2..3, BlockKind::Paragraph),
                (3..4, BlockKind::Blank),
                (4..5, BlockKind::ListItem),
                (5..6, BlockKind::ListItem),
                (6..7, BlockKind::ListItem),
                (7..8, BlockKind::Checklist { checked: true }),
                (8..9, BlockKind::Quote),
                (9..10, BlockKind::Rule),
            ]
        );
        assert_eq!(split_blocks(body)[5].indent, 2);
    }

    #[test]
    fn fences_math_tables_and_callouts_are_atomic() {
        let body = "```rust\nfn a() {}\n\n```\n```mermaid\nflowchart LR\nA-->B\n```\n$$\nx^2\n$$\n| a | b |\n| --- | :-: |\n| 1 | 2 |\n> [!note] Judul\n> isi\nsesudah";
        assert_eq!(
            kinds(body),
            vec![
                (0..4, BlockKind::Code),
                (4..8, BlockKind::Mermaid),
                (8..11, BlockKind::Math),
                (11..14, BlockKind::Table),
                (14..16, BlockKind::Callout),
                (16..17, BlockKind::Paragraph),
            ]
        );
        assert!(kinds(body).iter().all(|(_, k)| k.is_atomic() || *k == BlockKind::Paragraph));
    }

    #[test]
    fn unclosed_fence_is_just_a_line() {
        assert_eq!(kinds("```\nteks"), vec![(0..1, BlockKind::Paragraph), (1..2, BlockKind::Paragraph)]);
        assert_eq!(kinds("~~~~\nx\n~~~\n~~~~"), vec![(0..4, BlockKind::Code)]);
    }

    #[test]
    fn heading_needs_space_and_rules_need_three_marks() {
        assert_eq!(kinds("#tag"), vec![(0..1, BlockKind::Paragraph)]);
        assert_eq!(kinds("--"), vec![(0..1, BlockKind::Paragraph)]);
        assert_eq!(kinds("* * *"), vec![(0..1, BlockKind::Rule)]);
    }

    #[test]
    fn empty_body_and_crlf() {
        assert!(split_blocks("").is_empty());
        assert_eq!(kinds("a\r\n\r\n# b\r\n"), vec![(0..1, BlockKind::Paragraph), (1..2, BlockKind::Blank), (2..3, BlockKind::Heading(1))]);
    }

    #[test]
    fn replace_lines_keeps_line_endings() {
        assert_eq!(replace_lines("a\nb\nc\n", 1..2, "B"), "a\nB\nc\n");
        assert_eq!(replace_lines("a\nb", 1..2, "b1\nb2"), "a\nb1\nb2");
        assert_eq!(replace_lines("a\r\nb\r\n", 0..1, "x\ny"), "x\r\ny\r\nb\r\n");
        // Past the end appends, an empty range inserts.
        assert_eq!(replace_lines("a\n", 1..1, "z"), "a\nz\n");
        assert_eq!(replace_lines("", 0..0, "baru"), "baru");
        assert_eq!(replace_lines("a\nb", 1..1, "sisip"), "a\nsisip\nb");
    }

    #[test]
    fn lines_text_joins_the_range() {
        assert_eq!(lines_text("a\nb\nc", 1..3), "b\nc");
        assert_eq!(lines_text("a", 3..4), "");
        assert_eq!(line_count("a\nb\n"), 2);
    }

    #[test]
    fn list_continuation() {
        assert_eq!(continue_list("- beli"), ListContinuation::Continue("- ".into()));
        assert_eq!(continue_list("  * [x] selesai"), ListContinuation::Continue("  * [ ] ".into()));
        assert_eq!(continue_list("9. sembilan"), ListContinuation::Continue("10. ".into()));
        assert_eq!(continue_list("3) tiga"), ListContinuation::Continue("4) ".into()));
        assert_eq!(continue_list("- "), ListContinuation::EndList);
        assert_eq!(continue_list("- [ ] "), ListContinuation::EndList);
        assert_eq!(continue_list("teks biasa"), ListContinuation::None);
    }

    #[test]
    fn block_source_strips_indent_of_line_blocks_only() {
        let body = "    - dalam\n```\n    kode\n```";
        let lines: Vec<&str> = body.lines().collect();
        let blocks = split_blocks(body);
        assert_eq!(block_source(&lines, &blocks[0]), "- dalam");
        assert_eq!(block_source(&lines, &blocks[1]), "```\n    kode\n```");
    }
}
