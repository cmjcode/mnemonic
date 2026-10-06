//! Obsidian block references (§Fase 1.1, foundation for §Fase 3): a block
//! (paragraph, list item, heading, table…) ends with ` ^id` and can then be
//! linked as `[[Note#^id]]` or bound to a diagram node. Pure text logic —
//! no `egui`, no IO — so it's unit-testable; rendering hides the anchors
//! (`markdown::renderer`), navigation lives in `app`, the canvas binds to
//! ids through `canvas::element::BlockBinding`. Callers: `markdown::
//! renderer`, `markdown::editor`, `app`, `canvas`.

/// Characters an id may contain (Obsidian: letters, digits, `-`).
fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-'
}

/// Length of generated ids.
const GENERATED_ID_LEN: usize = 6;

/// One anchored block in a note body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockAnchor {
    /// The id without its `^`.
    pub id: String,
    /// 0-based line range `[start, end]` (inclusive) of the block.
    pub start_line: usize,
    pub end_line: usize,
    /// The block's text with the anchor removed.
    pub text: String,
}

/// Splits a line into `(text, id)` when it ends with a block anchor
/// ` ^id` (or is only `^id`, Obsidian's form for tables/code blocks).
pub fn split_anchor(line: &str) -> (&str, Option<&str>) {
    let trimmed = line.trim_end();
    let Some(caret) = trimmed.rfind('^') else {
        return (line, None);
    };
    let id = &trimmed[caret + 1..];
    if id.is_empty() || !id.chars().all(is_id_char) {
        return (line, None);
    }
    let before = &trimmed[..caret];
    // Must be at the start of the line or preceded by whitespace, so a
    // `2^10` in prose isn't an anchor.
    if !before.is_empty() && !before.ends_with(char::is_whitespace) {
        return (line, None);
    }
    (before.trim_end(), Some(id))
}

/// `true` when `line` ends with the anchor `^id`.
pub fn line_has_anchor(line: &str, id: &str) -> bool {
    split_anchor(line).1 == Some(id)
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

fn is_block_start(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with('#') && t.chars().find(|c| *c != '#') == Some(' ')
        || t.starts_with("- ")
        || t.starts_with("* ")
        || t.starts_with("+ ")
        || t.starts_with("> ")
        || t.starts_with('|')
        || t.chars().take_while(|c| c.is_ascii_digit()).count() > 0
            && t.trim_start_matches(|c: char| c.is_ascii_digit()).starts_with(". ")
}

/// Every anchored block in `body`, in document order. Fenced code is
/// skipped. A block is the run of non-blank lines ending at the anchored
/// line, stopping at a heading/list/quote boundary, so a paragraph keeps
/// all its lines while a list item is just its own line.
pub fn block_anchors(body: &str) -> Vec<BlockAnchor> {
    let lines: Vec<&str> = body.lines().collect();
    let mut out = Vec::new();
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
        let (text, Some(id)) = split_anchor(line) else {
            continue;
        };
        // A standalone `^id` line anchors the block above it (table/code).
        let (start, end, mut text_lines) = if text.trim().is_empty() {
            let end = i.saturating_sub(1);
            if i == 0 || is_blank(lines[end]) {
                (i, i, Vec::new())
            } else {
                let start = block_start(&lines, end);
                (start, i, lines[start..=end].iter().map(|l| l.to_string()).collect())
            }
        } else {
            let start = if is_block_start(line) { i } else { block_start(&lines, i) };
            let mut v: Vec<String> = lines[start..i].iter().map(|l| l.to_string()).collect();
            v.push(text.to_string());
            (start, i, v)
        };
        while text_lines.last().is_some_and(|l| l.trim().is_empty()) {
            text_lines.pop();
        }
        out.push(BlockAnchor {
            id: id.to_string(),
            start_line: start,
            end_line: end,
            text: text_lines.join("\n"),
        });
    }
    out
}

/// First line of the block that ends at `end` (walks up over non-blank,
/// non-boundary lines).
fn block_start(lines: &[&str], end: usize) -> usize {
    let is_table_row = |l: &str| l.trim_start().starts_with('|');
    let mut start = end;
    while start > 0 {
        let prev = lines[start - 1];
        if is_blank(prev) {
            break;
        }
        // A table is one block even though every row "starts" one.
        let both_table = is_table_row(prev) && is_table_row(lines[start]);
        if !both_table && (is_block_start(prev) || is_block_start(lines[start])) {
            break;
        }
        start -= 1;
    }
    start
}

/// Text of the block anchored `^id`, if any.
pub fn block_text(body: &str, id: &str) -> Option<String> {
    block_anchors(body)
        .into_iter()
        .find(|b| b.id == id)
        .map(|b| b.text)
}

/// 0-based line of the anchor `^id`.
pub fn anchor_line(body: &str, id: &str) -> Option<usize> {
    block_anchors(body)
        .into_iter()
        .find(|b| b.id == id)
        .map(|b| b.end_line)
}

/// Replaces the text of the block anchored `^id` with `new_text` (which may
/// span lines), keeping the anchor on the last line and a leading list/
/// quote marker on the first. `None` when the anchor doesn't exist or the
/// text is unchanged.
pub fn replace_block_text(body: &str, id: &str, new_text: &str) -> Option<String> {
    let block = block_anchors(body).into_iter().find(|b| b.id == id)?;
    let new_text = new_text.trim_end();
    if block.text == new_text {
        return None;
    }
    let lines: Vec<&str> = body.lines().collect();
    let first_old = lines[block.start_line];
    let marker = list_marker(first_old);
    let mut replacement: Vec<String> = Vec::new();
    let mut new_lines = new_text.lines().peekable();
    let first_new = new_lines.next().unwrap_or("");
    let first_has_marker = list_marker(first_new).is_some();
    replacement.push(if first_has_marker || marker.is_none() {
        first_new.to_string()
    } else {
        format!("{}{}", marker.unwrap_or(""), first_new)
    });
    for l in new_lines {
        replacement.push(l.to_string());
    }
    if replacement.is_empty() {
        replacement.push(String::new());
    }
    let last = replacement.len() - 1;
    let sep = if replacement[last].trim().is_empty() { "" } else { " " };
    replacement[last] = format!("{}{sep}^{id}", replacement[last]);

    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    out.extend(lines[..block.start_line].iter().map(|l| l.to_string()));
    out.extend(replacement);
    out.extend(lines[block.end_line + 1..].iter().map(|l| l.to_string()));
    let mut joined = out.join("\n");
    if body.ends_with('\n') {
        joined.push('\n');
    }
    Some(joined)
}

/// The `- `, `* `, `1. `, `> ` or `- [ ] ` prefix (with indentation) of a
/// line, if it is a list/quote item.
fn list_marker(line: &str) -> Option<&str> {
    let indent = line.len() - line.trim_start().len();
    let t = &line[indent..];
    let marker_len = if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ ")) {
        let base = t.len() - rest.len();
        let task = ["[ ] ", "[x] ", "[X] "]
            .iter()
            .find(|m| rest.starts_with(*m))
            .map(|m| m.len())
            .unwrap_or(0);
        base + task
    } else if t.starts_with("> ") {
        2
    } else {
        let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 && t[digits..].starts_with(". ") {
            digits + 2
        } else {
            return None;
        }
    };
    Some(&line[..indent + marker_len])
}

/// A fresh 6-character lowercase alphanumeric id (Obsidian's style),
/// unique among `existing`.
pub fn generate_id(existing: &[String]) -> String {
    loop {
        let bytes = uuid::Uuid::new_v4();
        let id: String = bytes
            .as_bytes()
            .iter()
            .map(|b| {
                let v = b % 36;
                if v < 10 { (b'0' + v) as char } else { (b'a' + v - 10) as char }
            })
            .take(GENERATED_ID_LEN)
            .collect();
        if !existing.contains(&id) {
            return id;
        }
    }
}

/// Ensures the block containing 0-based `line` carries an anchor,
/// returning the (possibly new) body and the id. A blank line gets no
/// anchor (`None`).
pub fn ensure_anchor_at_line(body: &str, line: usize) -> Option<(String, String)> {
    let lines: Vec<&str> = body.lines().collect();
    let target = *lines.get(line)?;
    if is_blank(target) {
        return None;
    }
    let existing: Vec<String> = block_anchors(body).into_iter().map(|b| b.id).collect();
    // Already anchored on this line, or on the last line of this paragraph.
    if let Some(id) = split_anchor(target).1 {
        return Some((body.to_string(), id.to_string()));
    }
    let is_table = |l: &str| l.trim_start().starts_with('|');
    let mut end = line;
    if is_table(target) {
        // Obsidian anchors a table with `^id` on its own line below it.
        while end + 1 < lines.len() && is_table(lines[end + 1]) {
            end += 1;
        }
        if let Some(next) = lines.get(end + 1)
            && let ("", Some(id)) = split_anchor(next)
        {
            return Some((body.to_string(), id.to_string()));
        }
        let id = generate_id(&existing);
        let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        out.insert(end + 1, format!("^{id}"));
        let mut joined = out.join("\n");
        if body.ends_with('\n') {
            joined.push('\n');
        }
        return Some((joined, id));
    }
    if !is_block_start(target) || list_marker(target).is_none() {
        while end + 1 < lines.len() && !is_blank(lines[end + 1]) && !is_block_start(lines[end + 1]) {
            end += 1;
        }
    }
    if let Some(id) = split_anchor(lines[end]).1 {
        return Some((body.to_string(), id.to_string()));
    }
    let id = generate_id(&existing);
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    out[end] = format!("{} ^{id}", lines[end].trim_end());
    let mut joined = out.join("\n");
    if body.ends_with('\n') {
        joined.push('\n');
    }
    Some((joined, id))
}

/// Gives every block of `body` (headings, paragraphs, list items, quotes,
/// tables) an anchor if it lacks one, so each can become a diagram node.
/// Returns the new body and every anchor in document order.
pub fn anchor_all_blocks(body: &str) -> (String, Vec<BlockAnchor>) {
    let mut body = body.to_string();
    loop {
        let anchors = block_anchors(&body);
        let lines: Vec<&str> = body.lines().collect();
        let mut in_fence = false;
        let mut next: Option<usize> = None;
        for (i, line) in lines.iter().enumerate() {
            let t = line.trim_start();
            if t.starts_with("```") || t.starts_with("~~~") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence || is_blank(line) || t.starts_with("---") {
                continue;
            }
            if anchors.iter().any(|a| a.start_line <= i && i <= a.end_line) {
                continue;
            }
            next = Some(i);
            break;
        }
        let Some(i) = next else {
            return (body.clone(), anchors);
        };
        match ensure_anchor_at_line(&body, i) {
            Some((new_body, _)) if new_body != body => body = new_body,
            // Could not anchor this line (shouldn't happen for non-blank
            // lines); bail out rather than loop forever.
            _ => return (body.clone(), anchors),
        }
    }
}

/// Appends `text` as a new anchored paragraph at the end of `body`.
pub fn append_block(body: &str, text: &str, id: &str) -> String {
    let mut out = body.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    let text = text.trim_end();
    if text.is_empty() {
        out.push_str(&format!("^{id}\n"));
    } else {
        out.push_str(&format!("{text} ^{id}\n"));
    }
    out
}

/// `body` with every ` ^id` anchor removed — for rendering and previews.
pub fn strip_anchors(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut in_fence = false;
    for (i, line) in body.split_inclusive('\n').enumerate() {
        let _ = i;
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            out.push_str(line);
            continue;
        }
        if in_fence {
            out.push_str(line);
            continue;
        }
        let ending = &line[line.trim_end_matches(['\n', '\r']).len()..];
        let (text, id) = split_anchor(line);
        if id.is_some() {
            out.push_str(text);
            out.push_str(ending);
        } else {
            out.push_str(line);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_anchor_recognizes_trailing_ids_only() {
        assert_eq!(split_anchor("Halo dunia ^abc123"), ("Halo dunia", Some("abc123")));
        assert_eq!(split_anchor("^tbl"), ("", Some("tbl")));
        assert_eq!(split_anchor("2^10 adalah 1024"), ("2^10 adalah 1024", None));
        assert_eq!(split_anchor("tanpa anchor"), ("tanpa anchor", None));
        assert_eq!(split_anchor("spasi ^id-2  "), ("spasi", Some("id-2")));
    }

    #[test]
    fn block_anchors_cover_paragraphs_lists_and_standalone_ids() {
        let body = "# Judul\n\nBaris satu\nbaris dua ^para\n\n- item a\n- item b ^item\n\n| a | b |\n|---|---|\n^tbl\n```\nx ^bukan\n```\n";
        let blocks = block_anchors(body);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].id, "para");
        assert_eq!(blocks[0].text, "Baris satu\nbaris dua");
        assert_eq!((blocks[0].start_line, blocks[0].end_line), (2, 3));
        assert_eq!(blocks[1].id, "item");
        assert_eq!(blocks[1].text, "- item b");
        assert_eq!(blocks[2].id, "tbl");
        assert_eq!(blocks[2].text, "| a | b |\n|---|---|");
        assert_eq!(block_text(body, "para").as_deref(), Some("Baris satu\nbaris dua"));
        assert_eq!(anchor_line(body, "tbl"), Some(10));
    }

    #[test]
    fn replace_block_text_keeps_anchor_and_list_marker() {
        let body = "Intro\n\n- [ ] tugas lama ^t1\n\nPenutup ^p\n";
        let out = replace_block_text(body, "t1", "tugas baru").unwrap();
        assert_eq!(out, "Intro\n\n- [ ] tugas baru ^t1\n\nPenutup ^p\n");
        let out = replace_block_text(&out, "p", "Penutup\nbaris kedua").unwrap();
        assert_eq!(out, "Intro\n\n- [ ] tugas baru ^t1\n\nPenutup\nbaris kedua ^p\n");
        assert!(replace_block_text(&out, "p", "Penutup\nbaris kedua").is_none());
        assert!(replace_block_text(&out, "zzz", "x").is_none());
    }

    #[test]
    fn ensure_anchor_adds_id_to_paragraph_end_once() {
        let body = "Satu\ndua\n\nTiga\n";
        let (out, id) = ensure_anchor_at_line(body, 0).unwrap();
        assert_eq!(id.len(), 6);
        assert_eq!(out, format!("Satu\ndua ^{id}\n\nTiga\n"));
        let (again, same) = ensure_anchor_at_line(&out, 1).unwrap();
        assert_eq!(same, id);
        assert_eq!(again, out);
        assert!(ensure_anchor_at_line(body, 2).is_none());
    }

    #[test]
    fn anchor_all_blocks_anchors_each_block_once() {
        let body = "# Judul\n\nParagraf satu\nlanjut.\n\n- item a\n- item b ^sudah\n\n```\nkode\n```\n\n| a |\n|---|\n";
        let (out, anchors) = anchor_all_blocks(body);
        assert_eq!(anchors.len(), 5, "{out}");
        assert_eq!(anchors[3].id, "sudah");
        assert!(out.contains("kode\n```"));
        // Idempotent.
        let (again, more) = anchor_all_blocks(&out);
        assert_eq!(again, out);
        assert_eq!(more.len(), 5);
    }

    #[test]
    fn append_and_strip_round_trip() {
        let body = append_block("Awal", "Blok baru", "n1");
        assert_eq!(body, "Awal\n\nBlok baru ^n1\n");
        assert_eq!(strip_anchors(&body), "Awal\n\nBlok baru\n");
        assert_eq!(strip_anchors("```\nkode ^x\n```\n"), "```\nkode ^x\n```\n");
    }

    #[test]
    fn generated_ids_are_unique_and_well_formed() {
        let a = generate_id(&[]);
        let b = generate_id(&[a.clone()]);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    }
}
