//! Serializable request/response types of the agent interface (§Fase 2).
//! Every field name here is part of the stable `--json`/MCP contract:
//! snake_case, vault-relative paths, RFC3339 timestamps, lowercase enum
//! strings. Add fields freely; never rename or remove one without a
//! version bump in `docs/agent-interface.md`. Callers: `api::service`,
//! `api::mcp`, `src/bin/mnemonic-cli.rs`.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::notes::Note;
use crate::notes::frontmatter::NoteType;

/// Filter for `VaultService::list_notes`. All fields optional; an empty
/// filter lists every non-trashed note.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct NoteFilter {
    /// Case-insensitive tag match; `a` also matches nested `a/b`.
    pub tag: Option<String>,
    /// Vault-relative folder; matches the folder itself and its subtree.
    /// `""` or `"."` means the vault root only.
    pub folder: Option<String>,
    pub include_trashed: bool,
}

/// Frontmatter-level view of a note (no body).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NoteSummary {
    pub id: Uuid,
    pub title: String,
    /// Vault-relative file path, e.g. `Projects/Plan.md`.
    pub path: String,
    /// Vault-relative parent folder, `""` for the root.
    pub folder: String,
    pub note_type: NoteType,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub created: DateTime<Utc>,
    pub modified: DateTime<Utc>,
    pub pinned: bool,
    pub archived: bool,
    pub trashed: bool,
    pub canvas: bool,
}

/// A note with its Markdown body and the frontmatter keys MNEMONIC does
/// not model (`extra`, preserved verbatim on every write).
#[derive(Debug, Clone, Serialize)]
pub struct NoteDetail {
    #[serde(flatten)]
    pub summary: NoteSummary,
    pub body: String,
    pub extra: BTreeMap<String, serde_json::Value>,
    /// Targets of every `[[wikilink]]` in the body, in order, deduplicated.
    pub links: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WriteNoteRequest {
    /// Title, vault-relative path, alias or UUID of the note to update.
    /// Doubles as the title when the note is created.
    #[serde(alias = "title", alias = "ref")]
    pub reference: String,
    /// New body; `None` leaves the body untouched.
    pub body: Option<String>,
    /// Folder for a created note (ignored for an existing one).
    pub folder: Option<String>,
    /// Replaces the tag list when given.
    pub tags: Option<Vec<String>>,
    pub create_if_missing: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WriteNoteResult {
    #[serde(flatten)]
    pub note: NoteSummary,
    pub created: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CreateNoteRequest {
    pub title: String,
    pub body: String,
    /// Vault-relative folder, created when missing. Must stay inside the vault.
    pub folder: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrashResult {
    pub id: Uuid,
    pub title: String,
    /// Where the file was before trashing.
    pub previous_path: String,
    /// Where it lives now (inside `.trash/`).
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NoteRefOut {
    pub id: Uuid,
    pub title: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BacklinkOut {
    pub source_id: Uuid,
    pub source_title: String,
    pub source_path: String,
    /// 0-based line index in the source note's body (same convention as
    /// `markdown::wikilink::LinkOccurrence`).
    pub line: usize,
    pub context: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BacklinksResult {
    pub target: NoteRefOut,
    pub backlinks: Vec<BacklinkOut>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutgoingLink {
    /// The `[[target]]` text as written.
    pub target: String,
    pub heading: Option<String>,
    pub alias: Option<String>,
    /// 0-based line index in the body.
    pub line: usize,
    pub context: String,
    /// Vault-relative path the target resolves to, `None` for a ghost link.
    pub resolved_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinksResult {
    pub source: NoteRefOut,
    pub links: Vec<OutgoingLink>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphNodeOut {
    pub key: String,
    pub label: String,
    /// `note` | `canvas` | `pdf` | `ghost`.
    pub kind: &'static str,
    pub doc_id: Option<Uuid>,
    pub path: Option<String>,
    pub tag: Option<String>,
    pub degree: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphEdgeOut {
    /// Indices into `nodes`.
    pub a: usize,
    pub b: usize,
    /// `link` | `semantic`.
    pub kind: &'static str,
    pub weight: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphOut {
    pub nodes: Vec<GraphNodeOut>,
    pub edges: Vec<GraphEdgeOut>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(default)]
pub struct ReindexOptions {
    /// Re-chunk and re-embed every note even when its content hash is
    /// unchanged.
    pub full: bool,
    /// Never load the embedding model: chunks get the FTS (keyword) index
    /// only and are marked so a later semantic run still embeds them.
    pub keyword_only: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReindexFailure {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReindexReport {
    /// Notes (including trashed) whose metadata/links were written to `notes_index`.
    pub notes_indexed: usize,
    /// Notes chunked in this run.
    pub chunked: usize,
    /// Notes whose cached chunks were already up to date.
    pub skipped: usize,
    /// Cached chunk sets dropped because their note is gone or trashed.
    pub pruned: usize,
    /// Whether vectors were produced by the embedding model (`false` =
    /// keyword-only, either requested or because the model failed to load).
    pub semantic: bool,
    pub failed: Vec<ReindexFailure>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SearchRequest {
    pub query: String,
    pub k: usize,
    /// Try the vector route (loads the embedding model on first use).
    pub semantic: bool,
}

impl Default for SearchRequest {
    fn default() -> Self {
        SearchRequest {
            query: String::new(),
            k: 10,
            semantic: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHitOut {
    pub doc_id: Uuid,
    /// Vault-relative path of the note (or PDF) the chunk came from.
    pub path: String,
    /// Note title, or the file name for a PDF.
    pub title: String,
    /// 1-based page for PDF chunks, `None` for notes.
    pub page: Option<usize>,
    pub char_offset: usize,
    /// Cosine similarity when the semantic route found the chunk.
    pub score: Option<f32>,
    /// `semantic` | `keyword` | `both`.
    pub kind: &'static str,
    /// Keyword snippet with highlight markers removed.
    pub snippet: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub query: String,
    /// `true` when vector similarity contributed to the ranking.
    pub semantic: bool,
    pub hits: Vec<SearchHitOut>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AskRequest {
    pub question: String,
    pub max_tokens: usize,
}

impl Default for AskRequest {
    fn default() -> Self {
        AskRequest {
            question: String::new(),
            max_tokens: crate::llm::DEFAULT_MAX_TOKENS,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Citation {
    pub doc_id: Uuid,
    pub path: String,
    pub title: String,
    pub page: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AskResult {
    pub question: String,
    pub answer: String,
    pub citations: Vec<Citation>,
    /// Whether the retrieval that grounded the answer used vectors.
    pub semantic: bool,
    pub warnings: Vec<String>,
}

/// Converts a frontmatter `extra` map (YAML values) to JSON; a value JSON
/// can't represent (non-string mapping keys) is rendered as a string.
pub fn extra_to_json(extra: &BTreeMap<String, serde_yaml::Value>) -> BTreeMap<String, serde_json::Value> {
    extra
        .iter()
        .map(|(k, v)| {
            let json = serde_json::to_value(v).unwrap_or_else(|_| {
                serde_json::Value::String(serde_yaml::to_string(v).unwrap_or_default().trim().to_string())
            });
            (k.clone(), json)
        })
        .collect()
}

/// Which Mermaid diagram a diagram request is about: inline `source`, or
/// the `index`-th ```` ```mermaid ```` fence of note `ref`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DiagramRequest {
    pub source: Option<String>,
    #[serde(alias = "reference", alias = "note", alias = "title", alias = "path")]
    pub r#ref: Option<String>,
    pub index: usize,
    /// Render with the dark theme (unless the diagram sets its own).
    pub dark: bool,
}

/// One parser diagnostic of a Mermaid diagram (§3.7.2).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DiagramDiagnostic {
    /// 1-based line within the diagram source.
    pub line: usize,
    /// 1-based line within the note body, for diagrams read from a note.
    pub note_line: Option<usize>,
    /// 1-based column (chars).
    pub col: usize,
    /// `error` | `warning`.
    pub severity: String,
    pub message: String,
}

/// Result of validating one diagram.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DiagramCheck {
    /// Diagram type (`flowchart`, `sequence`, …, `unknown`).
    pub kind: String,
    /// This build can draw the type.
    pub supported: bool,
    /// No error-level diagnostics.
    pub valid: bool,
    pub diagnostics: Vec<DiagramDiagnostic>,
}

/// A Mermaid fence found in a note.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DiagramInfo {
    /// 0-based position among the note's mermaid fences.
    pub index: usize,
    /// 1-based line of the opening fence in the note body.
    pub line: usize,
    /// The closing fence exists.
    pub closed: bool,
    pub source: String,
    #[serde(flatten)]
    pub check: DiagramCheck,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DiagramList {
    pub path: String,
    pub diagrams: Vec<DiagramInfo>,
}

/// A rendered diagram.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DiagramRender {
    pub kind: String,
    /// Always `svg` for now.
    pub format: String,
    pub width: f32,
    pub height: f32,
    pub svg: String,
    pub valid: bool,
    pub diagnostics: Vec<DiagramDiagnostic>,
}

/// Builds the summary of `note`, with `rel` = its vault-relative path.
pub fn summarize(note: &Note, rel: String) -> NoteSummary {
    let fm = &note.frontmatter;
    let folder = match rel.rfind('/') {
        Some(i) => rel[..i].to_string(),
        None => String::new(),
    };
    NoteSummary {
        id: fm.id,
        title: fm.title.clone(),
        path: rel,
        folder,
        note_type: fm.note_type,
        tags: fm.tags.clone(),
        aliases: fm.aliases.clone(),
        created: fm.created,
        modified: fm.modified,
        pinned: fm.pinned,
        archived: fm.archived,
        trashed: fm.trashed,
        canvas: note.is_canvas(),
    }
}
