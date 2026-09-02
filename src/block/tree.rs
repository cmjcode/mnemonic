//! Block data structures and bidirectional conversion with Markdown.
//!
//! Every document can be represented as a hierarchical tree of `BlockNode`s,
//! supporting both rich structured block operations (like drag-to-reorder,
//! block formatting, and canvas projection) and lossless round-tripping
//! to standard Markdown for file-system storage.

use std::collections::HashMap;
use uuid::Uuid;

/// Unique identifier for a block node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct BlockId(pub Uuid);

impl BlockId {
    pub fn new() -> Self {
        BlockId(Uuid::new_v4())
    }
}

impl Default for BlockId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The specific content type and data of a block.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum BlockKind {
    Paragraph(String),
    Heading { level: u8, text: String },
    Checklist { checked: bool, text: String },
    CodeBlock { lang: String, code: String },
    Callout { kind: String, text: String },
    Quote(String),
    Divider,
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    CanvasEmbed { canvas_id: String },
    PdfEmbed { file_name: String, page: Option<usize> },
}

/// A node within the `BlockTree`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BlockNode {
    pub id: BlockId,
    pub kind: BlockKind,
    pub children: Vec<BlockId>,
    #[serde(default)]
    pub properties: HashMap<String, String>,
}

impl BlockNode {
    pub fn new(kind: BlockKind) -> Self {
        BlockNode {
            id: BlockId::new(),
            kind,
            children: Vec::new(),
            properties: HashMap::new(),
        }
    }

    pub fn with_id(id: BlockId, kind: BlockKind) -> Self {
        BlockNode {
            id,
            kind,
            children: Vec::new(),
            properties: HashMap::new(),
        }
    }

    pub fn text_content(&self) -> &str {
        match &self.kind {
            BlockKind::Paragraph(t)
            | BlockKind::Heading { text: t, .. }
            | BlockKind::Checklist { text: t, .. }
            | BlockKind::Callout { text: t, .. }
            | BlockKind::Quote(t) => t.as_str(),
            BlockKind::CodeBlock { code, .. } => code.as_str(),
            BlockKind::Divider | BlockKind::Table { .. } | BlockKind::CanvasEmbed { .. } | BlockKind::PdfEmbed { .. } => "",
        }
    }
}

/// A document represented as a hierarchy of blocks.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BlockTree {
    pub root_blocks: Vec<BlockId>,
    pub blocks: HashMap<BlockId, BlockNode>,
}

impl BlockTree {
    pub fn new() -> Self {
        BlockTree {
            root_blocks: Vec::new(),
            blocks: HashMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.root_blocks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.root_blocks.len()
    }

    pub fn get(&self, id: BlockId) -> Option<&BlockNode> {
        self.blocks.get(&id)
    }

    pub fn get_mut(&mut self, id: BlockId) -> Option<&mut BlockNode> {
        self.blocks.get_mut(&id)
    }

    pub fn add_root_block(&mut self, node: BlockNode) -> BlockId {
        let id = node.id;
        self.blocks.insert(id, node);
        self.root_blocks.push(id);
        id
    }

    pub fn insert_root_block(&mut self, index: usize, node: BlockNode) -> BlockId {
        let id = node.id;
        self.blocks.insert(id, node);
        let safe_idx = index.min(self.root_blocks.len());
        self.root_blocks.insert(safe_idx, id);
        id
    }

    pub fn remove_block(&mut self, id: BlockId) -> Option<BlockNode> {
        self.root_blocks.retain(|&b_id| b_id != id);
        // Also remove from any parents' children lists
        for node in self.blocks.values_mut() {
            node.children.retain(|&c_id| c_id != id);
        }
        self.blocks.remove(&id)
    }

    pub fn move_root_block(&mut self, from_index: usize, to_index: usize) {
        if from_index < self.root_blocks.len() && to_index < self.root_blocks.len() {
            let id = self.root_blocks.remove(from_index);
            self.root_blocks.insert(to_index, id);
        }
    }

    /// Converts a Markdown text body into a structured `BlockTree`.
    pub fn from_markdown(md: &str) -> Self {
        let mut tree = BlockTree::new();
        let lines: Vec<&str> = md.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i];
            let trimmed = line.trim();

            // 1. Empty lines -> skip or preserve as paragraph breaks
            if trimmed.is_empty() {
                i += 1;
                continue;
            }

            // 2. Fenced Code Block: ```lang ... ```
            if trimmed.starts_with("```") {
                let lang = trimmed.trim_start_matches('`').trim().to_string();
                let mut code_lines = Vec::new();
                i += 1;
                while i < lines.len() {
                    let code_line = lines[i];
                    if code_line.trim().starts_with("```") {
                        i += 1;
                        break;
                    }
                    code_lines.push(code_line);
                    i += 1;
                }
                tree.add_root_block(BlockNode::new(BlockKind::CodeBlock {
                    lang,
                    code: code_lines.join("\n"),
                }));
                continue;
            }

            // 3. Horizontal Rule
            if trimmed == "---" || trimmed == "***" || trimmed == "___" {
                tree.add_root_block(BlockNode::new(BlockKind::Divider));
                i += 1;
                continue;
            }

            // 4. Headings: #, ##, ###, ####, #####, ######
            if trimmed.starts_with('#') {
                let mut hashes = 0;
                for ch in trimmed.chars() {
                    if ch == '#' {
                        hashes += 1;
                    } else {
                        break;
                    }
                }
                if hashes >= 1 && hashes <= 6 && trimmed.chars().nth(hashes) == Some(' ') {
                    let text = trimmed[hashes + 1..].trim().to_string();
                    tree.add_root_block(BlockNode::new(BlockKind::Heading {
                        level: hashes as u8,
                        text,
                    }));
                    i += 1;
                    continue;
                }
            }

            // 5. Checklist Item: - [ ] or - [x]
            if trimmed.starts_with("- [ ] ") || trimmed.starts_with("- [x] ") || trimmed.starts_with("- [X] ") {
                let checked = trimmed.starts_with("- [x] ") || trimmed.starts_with("- [X] ");
                let text = trimmed[6..].trim().to_string();
                tree.add_root_block(BlockNode::new(BlockKind::Checklist { checked, text }));
                i += 1;
                continue;
            }

            // 6. Callout Alert: > [!note], > [!tip], > [!warning], etc.
            if trimmed.starts_with("> [!") && trimmed.contains(']') {
                if let Some(close_bracket) = trimmed.find(']') {
                    let kind = trimmed[4..close_bracket].to_string();
                    let mut callout_text = Vec::new();
                    let first_line_remainder = trimmed[close_bracket + 1..].trim();
                    if !first_line_remainder.is_empty() {
                        callout_text.push(first_line_remainder);
                    }
                    i += 1;
                    while i < lines.len() {
                        let next_line = lines[i].trim();
                        if next_line.starts_with('>') {
                            let content = next_line.trim_start_matches('>').trim();
                            callout_text.push(content);
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    tree.add_root_block(BlockNode::new(BlockKind::Callout {
                        kind,
                        text: callout_text.join("\n"),
                    }));
                    continue;
                }
            }

            // 7. Blockquote: > text
            if trimmed.starts_with('>') {
                let mut quote_lines = Vec::new();
                while i < lines.len() {
                    let next_line = lines[i].trim();
                    if next_line.starts_with('>') {
                        let content = next_line.trim_start_matches('>').trim();
                        quote_lines.push(content);
                        i += 1;
                    } else {
                        break;
                    }
                }
                tree.add_root_block(BlockNode::new(BlockKind::Quote(quote_lines.join("\n"))));
                continue;
            }

            // 8. Markdown Table: starts with | and contains |
            if trimmed.starts_with('|') && trimmed.ends_with('|') && lines.len() > i + 1 && lines[i + 1].trim().starts_with('|') && lines[i + 1].contains("---") {
                let headers: Vec<String> = trimmed
                    .trim_matches('|')
                    .split('|')
                    .map(|s| s.trim().to_string())
                    .collect();
                i += 2; // skip header line and separator line
                let mut rows = Vec::new();
                while i < lines.len() {
                    let row_line = lines[i].trim();
                    if row_line.starts_with('|') && row_line.ends_with('|') {
                        let row: Vec<String> = row_line
                            .trim_matches('|')
                            .split('|')
                            .map(|s| s.trim().to_string())
                            .collect();
                        rows.push(row);
                        i += 1;
                    } else {
                        break;
                    }
                }
                tree.add_root_block(BlockNode::new(BlockKind::Table { headers, rows }));
                continue;
            }

            // 9. Standard Paragraph
            let mut para_lines = vec![line];
            i += 1;
            while i < lines.len() {
                let next_line = lines[i];
                let next_trimmed = next_line.trim();
                if next_trimmed.is_empty()
                    || next_trimmed.starts_with('#')
                    || next_trimmed.starts_with("```")
                    || next_trimmed == "---"
                    || next_trimmed.starts_with("- [ ] ")
                    || next_trimmed.starts_with("- [x] ")
                    || next_trimmed.starts_with('>')
                    || (next_trimmed.starts_with('|') && next_trimmed.ends_with('|'))
                {
                    break;
                }
                para_lines.push(next_line);
                i += 1;
            }
            tree.add_root_block(BlockNode::new(BlockKind::Paragraph(para_lines.join("\n"))));
        }

        tree
    }

    /// Converts the `BlockTree` back into standard Markdown.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();

        for (idx, &block_id) in self.root_blocks.iter().enumerate() {
            let Some(node) = self.blocks.get(&block_id) else {
                continue;
            };

            if idx > 0 {
                out.push_str("\n\n");
            }

            match &node.kind {
                BlockKind::Paragraph(text) => {
                    out.push_str(text);
                }
                BlockKind::Heading { level, text } => {
                    let hashes = "#".repeat((*level as usize).clamp(1, 6));
                    out.push_str(&format!("{} {}", hashes, text));
                }
                BlockKind::Checklist { checked, text } => {
                    let marker = if *checked { "- [x]" } else { "- [ ]" };
                    out.push_str(&format!("{} {}", marker, text));
                }
                BlockKind::CodeBlock { lang, code } => {
                    out.push_str(&format!("```{}\n{}\n```", lang, code));
                }
                BlockKind::Callout { kind, text } => {
                    out.push_str(&format!("> [!{}]\n", kind));
                    for line in text.lines() {
                        out.push_str(&format!("> {}\n", line));
                    }
                    if out.ends_with('\n') {
                        out.pop();
                    }
                }
                BlockKind::Quote(text) => {
                    for (i, line) in text.lines().enumerate() {
                        if i > 0 {
                            out.push('\n');
                        }
                        out.push_str(&format!("> {}", line));
                    }
                }
                BlockKind::Divider => {
                    out.push_str("---");
                }
                BlockKind::Table { headers, rows } => {
                    if !headers.is_empty() {
                        out.push_str(&format!("| {} |\n", headers.join(" | ")));
                        let sep: Vec<String> = headers.iter().map(|_| "---".to_string()).collect();
                        out.push_str(&format!("| {} |", sep.join(" | ")));
                        for row in rows {
                            out.push_str(&format!("\n| {} |", row.join(" | ")));
                        }
                    }
                }
                BlockKind::CanvasEmbed { canvas_id } => {
                    out.push_str(&format!("![[canvas:{}]]", canvas_id));
                }
                BlockKind::PdfEmbed { file_name, page } => {
                    if let Some(p) = page {
                        out.push_str(&format!("![[pdf:{}#page={}]]", file_name, p));
                    } else {
                        out.push_str(&format!("![[pdf:{}]]", file_name));
                    }
                }
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_heading_and_paragraphs() {
        let md = "# Title 1\n\nFirst paragraph.\n\n## Subtitle\n\nSecond paragraph.";
        let tree = BlockTree::from_markdown(md);
        assert_eq!(tree.len(), 4);

        let round_trip = tree.to_markdown();
        assert_eq!(round_trip, md);
    }

    #[test]
    fn parses_checklists_and_code_blocks() {
        let md = "- [ ] Item 1\n\n- [x] Item 2\n\n```rust\nfn main() {}\n```";
        let tree = BlockTree::from_markdown(md);
        assert_eq!(tree.len(), 3);

        let round_trip = tree.to_markdown();
        assert_eq!(round_trip, md);
    }

    #[test]
    fn parses_callouts_and_tables() {
        let md = "> [!note]\n> Important note\n\n| Name | Status |\n| --- | --- |\n| Task A | Done |";
        let tree = BlockTree::from_markdown(md);
        assert_eq!(tree.len(), 2);

        let round_trip = tree.to_markdown();
        assert_eq!(round_trip, md);
    }

    #[test]
    fn block_mutations_and_reordering() {
        let mut tree = BlockTree::new();
        let b1 = tree.add_root_block(BlockNode::new(BlockKind::Paragraph("One".into())));
        let b2 = tree.add_root_block(BlockNode::new(BlockKind::Paragraph("Two".into())));

        assert_eq!(tree.root_blocks, vec![b1, b2]);
        tree.move_root_block(0, 1);
        assert_eq!(tree.root_blocks, vec![b2, b1]);

        tree.remove_block(b2);
        assert_eq!(tree.root_blocks, vec![b1]);
    }
}
