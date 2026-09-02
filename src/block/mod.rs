//! Block-based architecture for documents and hybrid canvas-doc representations.
//!
//! Provides the core data structures (`BlockTree`, `BlockNode`, `BlockKind`, `BlockId`)
//! for representing rich documents as a tree of blocks, mirroring the modern
//! BlockSuite architecture while preserving seamless compatibility with local
//! Markdown files.

pub mod store;
pub mod tree;

pub use store::BlockStore;
pub use tree::{BlockId, BlockKind, BlockNode, BlockTree};
