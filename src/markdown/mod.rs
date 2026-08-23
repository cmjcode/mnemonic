//! Markdown editing & rendering (§3.2): Source/Live Preview/Reading modes,
//! wikilink resolution + backlinks + autocomplete, interactive checklists,
//! heading outline, and syntax-highlighted, Obsidian-style callout
//! rendering via `egui_commonmark`. Callers: `app.rs`.

pub mod editor;
pub mod renderer;
pub mod wikilink;

pub use editor::{EditorMode, MarkdownEditor};
pub use wikilink::WikilinkIndex;
