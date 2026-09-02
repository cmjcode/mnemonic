//! Operations and transactions over `BlockTree`.

use super::tree::{BlockId, BlockKind, BlockNode, BlockTree};

/// High-level operations over a block-based document session.
pub struct BlockStore {
    pub tree: BlockTree,
    pub active_block: Option<BlockId>,
}

impl BlockStore {
    pub fn new(tree: BlockTree) -> Self {
        BlockStore {
            tree,
            active_block: None,
        }
    }

    pub fn from_markdown(md: &str) -> Self {
        Self::new(BlockTree::from_markdown(md))
    }

    pub fn to_markdown(&self) -> String {
        self.tree.to_markdown()
    }

    pub fn insert_after(&mut self, target_id: BlockId, new_node: BlockNode) -> BlockId {
        let new_id = new_node.id;
        if let Some(pos) = self.tree.root_blocks.iter().position(|&id| id == target_id) {
            self.tree.insert_root_block(pos + 1, new_node);
        } else {
            self.tree.add_root_block(new_node);
        }
        new_id
    }

    pub fn split_paragraph_at_cursor(&mut self, block_id: BlockId, char_offset: usize) -> Option<BlockId> {
        let second_part = {
            let node = self.tree.get_mut(block_id)?;
            if let BlockKind::Paragraph(text) = &mut node.kind {
                let safe_offset = char_offset.min(text.len());
                let right = text[safe_offset..].to_string();
                text.truncate(safe_offset);
                Some(right)
            } else {
                None
            }
        }?;

        let new_node = BlockNode::new(BlockKind::Paragraph(second_part));
        let new_id = new_node.id;

        if let Some(pos) = self.tree.root_blocks.iter().position(|&id| id == block_id) {
            self.tree.insert_root_block(pos + 1, new_node);
            Some(new_id)
        } else {
            self.tree.add_root_block(new_node);
            Some(new_id)
        }
    }

    pub fn toggle_checklist(&mut self, block_id: BlockId) -> bool {
        if let Some(node) = self.tree.get_mut(block_id) {
            if let BlockKind::Checklist { checked, .. } = &mut node.kind {
                *checked = !*checked;
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_toggle_operations() {
        let mut store = BlockStore::from_markdown("Hello World\n\n- [ ] Task A");
        assert_eq!(store.tree.root_blocks.len(), 2);

        let first_id = store.tree.root_blocks[0];
        let split_id = store.split_paragraph_at_cursor(first_id, 5).expect("split succeeds");

        assert_eq!(store.tree.root_blocks.len(), 3);
        assert_eq!(store.tree.get(first_id).unwrap().text_content(), "Hello");
        assert_eq!(store.tree.get(split_id).unwrap().text_content(), " World");

        let task_id = store.tree.root_blocks[2];
        assert!(store.toggle_checklist(task_id));
        if let BlockKind::Checklist { checked, .. } = store.tree.get(task_id).unwrap().kind {
            assert!(checked);
        } else {
            panic!("Expected checklist block");
        }
    }
}
