//! Relationship graph (Obsidian-style graph view): `model` turns notes,
//! PDFs, wikilinks and AI similarity into nodes/edges; `layout` places
//! them with a force-directed simulation. Both are pure (no `egui` UI or
//! IO) so they're unit-testable; drawing and interaction live in
//! `app::graph`. Callers: `app`.

pub mod layout;
pub mod model;

pub use layout::{ForceParams, Layout};
pub use model::{EdgeKind, GraphData, GraphEdge, GraphNode, GraphOptions, NodeKind};
