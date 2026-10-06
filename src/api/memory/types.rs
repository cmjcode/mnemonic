//! Request/response shapes of the agent-memory tools (§3.10), re-exported
//! from `api::types` and bound by the same contract: snake_case, stable
//! once shipped, vault-relative paths, add fields but never rename them.
//! Callers: `api::memory`, `api::mcp`, `src/bin/mnemonic-cli`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::api::types::{NoteRefOut, NoteSummary, SelectionOut};

/// `read_note` with an optional part: a heading (`Status`, `Proyek#Status`,
/// `^anchor`) or an anchored block id.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ReadNoteRequest {
    #[serde(alias = "reference", alias = "title", alias = "path")]
    pub r#ref: String,
    pub section: Option<String>,
    /// Block id, with or without `^`.
    pub block: Option<String>,
}

/// Add text to a note without resending it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AppendNoteRequest {
    #[serde(alias = "reference", alias = "title", alias = "path")]
    pub r#ref: String,
    #[serde(alias = "body")]
    pub text: String,
    /// Append at the end of this section/block instead of the note.
    pub section: Option<String>,
    pub if_hash: Option<String>,
    pub agent: Option<String>,
    /// Create the note (titled `ref`, in `folder`) when it doesn't exist.
    pub create_if_missing: bool,
    pub folder: Option<String>,
    /// Tags for a created note.
    pub tags: Vec<String>,
}

/// Change one part of a note. Target: `section` (a heading's content is
/// replaced, the heading line kept), `block` (the block text, anchor
/// kept), and/or `old_str` (exact text, replaced by `new_str`; searched
/// only inside the section/block when one is given).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PatchNoteRequest {
    #[serde(alias = "reference", alias = "title", alias = "path")]
    pub r#ref: String,
    pub section: Option<String>,
    pub block: Option<String>,
    pub old_str: Option<String>,
    #[serde(alias = "text")]
    pub new_str: String,
    /// Replace every occurrence of `old_str` (default: it must be unique).
    pub replace_all: bool,
    pub if_hash: Option<String>,
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EditNoteResult {
    #[serde(flatten)]
    pub note: NoteSummary,
    /// Fingerprint after the edit (use as the next `if_hash`).
    pub content_hash: String,
    /// `false` when the edit left the note as it was.
    pub changed: bool,
    pub created: bool,
    pub selection: Option<SelectionOut>,
    /// `old_str` occurrences replaced.
    pub replacements: usize,
    pub warnings: Vec<String>,
}

/// One folder of the vault (hidden folders are never listed).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FolderInfo {
    /// Vault-relative, `/`-separated; `""` for the root.
    pub path: String,
    pub name: String,
    pub depth: usize,
    /// Notes directly inside.
    pub notes: usize,
    /// Notes in the whole subtree.
    pub notes_total: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct FolderList {
    pub folders: Vec<FolderInfo>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TagCount {
    pub tag: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecentNote {
    #[serde(flatten)]
    pub note: NoteRefOut,
    pub modified: DateTime<Utc>,
}

/// A title several notes share: plain `[[links]]` and refs to it are
/// ambiguous, so use one of `paths`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DuplicateTitle {
    pub title: String,
    pub paths: Vec<String>,
}

/// The vault's own instructions for agents (`AGENTS.md` at the root).
#[derive(Debug, Clone, Serialize)]
pub struct AgentGuide {
    pub path: String,
    pub text: String,
    pub truncated: bool,
}

/// Everything an agent needs to orient itself in one call (§3.10.1).
#[derive(Debug, Clone, Serialize)]
pub struct VaultOverview {
    /// Name of the vault folder.
    pub name: String,
    pub notes: usize,
    pub canvases: usize,
    pub sheets: usize,
    pub pdfs: usize,
    pub folders: Vec<FolderInfo>,
    /// Most used tags (frontmatter + inline), at most 40.
    pub tags: Vec<TagCount>,
    /// Most recently modified notes, at most 10.
    pub recent: Vec<RecentNote>,
    pub duplicate_titles: Vec<DuplicateTitle>,
    pub guide: Option<AgentGuide>,
    /// How to use the interface well; stable wording is not guaranteed.
    pub tips: Vec<String>,
}

/// Store a fact/decision/observation (§3.10.4).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RememberRequest {
    pub text: String,
    /// Title of a new memory note (default: from the text's first line).
    pub title: Option<String>,
    /// Folder of a new memory note (default `Memory`).
    pub folder: Option<String>,
    pub tags: Vec<String>,
    /// Append to this existing note instead of creating one.
    #[serde(alias = "into", alias = "note")]
    pub r#ref: Option<String>,
    /// With `ref`: append inside this section.
    pub section: Option<String>,
    /// Notes to link from the memory (`[[target]]`s appended as "Related").
    pub links: Vec<String>,
    pub agent: Option<String>,
    /// Write even when the same content already exists.
    pub allow_duplicate: bool,
    /// Use vector similarity for the duplicate check (loads the model).
    pub semantic: bool,
}

impl Default for RememberRequest {
    fn default() -> Self {
        RememberRequest {
            text: String::new(),
            title: None,
            folder: None,
            tags: Vec::new(),
            r#ref: None,
            section: None,
            links: Vec::new(),
            agent: None,
            allow_duplicate: false,
            semantic: true,
        }
    }
}

/// A note close to some text.
#[derive(Debug, Clone, Serialize)]
pub struct SimilarNote {
    pub path: String,
    pub title: String,
    pub score: Option<f32>,
    /// `exact` (text already contained) | `semantic` | `keyword` | `both`.
    pub kind: &'static str,
    pub heading: Option<String>,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RememberResult {
    /// `created` | `appended` | `duplicate` (nothing written).
    pub status: &'static str,
    pub note: Option<NoteSummary>,
    pub content_hash: Option<String>,
    /// Existing notes resembling the text (the duplicate, when `status`
    /// is `duplicate`).
    pub similar: Vec<SimilarNote>,
    pub warnings: Vec<String>,
}

/// Retrieve context for a task within a token budget (§3.10.4).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RecallRequest {
    pub query: String,
    /// Approximate budget (≈ 4 characters per token).
    pub budget_tokens: usize,
    /// Chunks considered before expanding them to sections.
    pub k: usize,
    pub folder: Option<String>,
    pub tag: Option<String>,
    pub semantic: bool,
    /// Keep stale notes at their rank instead of moving them last.
    pub include_stale: bool,
}

impl Default for RecallRequest {
    fn default() -> Self {
        RecallRequest {
            query: String::new(),
            budget_tokens: 1500,
            k: 8,
            folder: None,
            tag: None,
            semantic: true,
            include_stale: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RecallItem {
    pub path: String,
    pub title: String,
    /// Heading path of the returned section, if the text is one.
    pub section: Option<String>,
    /// 1-based first body line of `text` (notes only).
    pub line: Option<usize>,
    pub text: String,
    pub score: Option<f32>,
    /// `semantic` | `keyword` | `both`.
    pub kind: &'static str,
    /// Why the note is outdated (`valid_until`, `superseded_by`, …), if it is.
    pub stale: Option<String>,
    /// `text` was cut to fit the budget.
    pub truncated: bool,
    pub tokens: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecallResult {
    pub query: String,
    pub semantic: bool,
    pub budget_tokens: usize,
    pub used_tokens: usize,
    pub items: Vec<RecallItem>,
    /// Relevant parts left out for lack of budget.
    pub omitted: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RelatedRequest {
    #[serde(alias = "reference", alias = "title", alias = "path")]
    pub r#ref: String,
    pub k: usize,
}

impl Default for RelatedRequest {
    fn default() -> Self {
        RelatedRequest { r#ref: String::new(), k: 10 }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RelatedNote {
    pub path: String,
    pub title: String,
    /// `links_to` | `linked_from` | `similar` | `shared_tag:<tag>`.
    pub reasons: Vec<String>,
    /// Embedding similarity, when `similar`.
    pub score: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelatedResult {
    pub note: NoteRefOut,
    pub related: Vec<RelatedNote>,
    pub warnings: Vec<String>,
}
