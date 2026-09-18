//! Agent interface (§Fase 2 "agent-harness friendliness"): an egui-free
//! service over a vault that external tools — `mnemonic-cli`, the MCP
//! server (`api::mcp`), scripts — call to list/read/write notes, follow
//! links, search, reindex and ask the local LLM. It reuses the same
//! `notes`, `core`, `graph`, `markdown::wikilink` and `llm` modules the
//! desktop app does, so both see one vault and one index
//! (`.mnemonic/index.sqlite3`, WAL, safe to share between processes).
//!
//! Layout: `types` (stable serializable request/response shapes),
//! `service` (open + note CRUD + links/graph), `index` (reindex, search,
//! ask), `diagram` (Mermaid list/validate/render, §3.7.7), `mcp`
//! (JSON-RPC 2.0 over stdio). Callers: `src/bin/mnemonic-cli.rs`.

pub mod diagram;
pub mod index;
pub mod mcp;
pub mod service;
pub mod types;

pub use service::VaultService;
pub use types::{
    AskRequest, AskResult, BacklinksResult, CreateNoteRequest, DiagramCheck, DiagramList, DiagramRender,
    DiagramRequest, GraphOut, LinksResult, NoteDetail,
    NoteFilter, NoteSummary, ReindexOptions, ReindexReport, SearchRequest, SearchResult,
    TrashResult, WriteNoteRequest, WriteNoteResult,
};
