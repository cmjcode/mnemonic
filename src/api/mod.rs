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
//! ask), `diagram` (Mermaid list/validate/render, §3.7.7), `canvas`
//! (section outline + canvas → Mermaid, §3.9.5), `sheet`
//! (CSV/XLSX list/read/query/edit, §3.8.5), `export` (reading themes,
//! HTML/PDF export, §3.2.5), `memory` (agent memory: vault overview,
//! folders, piecewise read/append/patch with `if_hash`, remember/recall/
//! related, §3.10), `mcp` (JSON-RPC 2.0 over stdio).
//! Callers: `src/bin/mnemonic-cli.rs`.

pub mod canvas;
pub mod diagram;
pub mod export;
pub mod index;
pub mod mcp;
pub mod memory;
pub mod service;
pub mod sheet;
pub mod types;

pub use service::VaultService;
pub use types::{
    AskRequest, AskResult, BacklinksResult, CreateNoteRequest, DiagramCheck, DiagramList, DiagramRender,
    DiagramRequest, ExportRequest, ExportResult, GraphOut, LinksResult, NoteDetail, ThemeInfo, ThemeList,
    NoteFilter, NoteSummary, ReindexOptions, ReindexReport, SearchRequest, SearchResult,
    TrashResult, WriteNoteRequest, WriteNoteResult,
};
