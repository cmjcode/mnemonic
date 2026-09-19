//! MCP tools of the agent-memory layer (§3.10): definitions (name,
//! description, JSON Schema) and dispatch, merged into `api::mcp`'s
//! `tools/list` / `tools/call`. Each tool maps 1:1 onto a `VaultService`
//! method. Callers: `api::mcp`.

use anyhow::Result;
use serde_json::{Value, json};

use super::types::*;
use crate::api::VaultService;

pub const TOOL_NAMES: &[&str] = &[
    "vault_overview",
    "list_folders",
    "append_note",
    "patch_note",
    "remember",
    "recall",
    "related_notes",
];

fn parse<T: serde::de::DeserializeOwned>(args: Value) -> Result<T> {
    serde_json::from_value(args).map_err(|e| anyhow::anyhow!("invalid arguments: {e}"))
}

/// Runs memory tool `name`; `None` when `name` is not one of [`TOOL_NAMES`].
pub fn call_tool(service: &mut VaultService, name: &str, args: Value) -> Option<Result<Value>> {
    let run = || -> Result<Value> {
        Ok(match name {
            "vault_overview" => serde_json::to_value(service.vault_overview())?,
            "list_folders" => serde_json::to_value(service.list_folders())?,
            "append_note" => serde_json::to_value(service.append_note(&parse::<AppendNoteRequest>(args)?)?)?,
            "patch_note" => serde_json::to_value(service.patch_note(&parse::<PatchNoteRequest>(args)?)?)?,
            "remember" => serde_json::to_value(service.remember(&parse::<RememberRequest>(args)?)?)?,
            "recall" => serde_json::to_value(service.recall(&parse::<RecallRequest>(args)?)?)?,
            "related_notes" => serde_json::to_value(service.related(&parse::<RelatedRequest>(args)?)?)?,
            other => anyhow::bail!("unknown tool: {other}"),
        })
    };
    TOOL_NAMES.contains(&name).then(run)
}

fn note_ref() -> Value {
    json!({ "type": "string", "description": "Note: vault-relative path (safest), title, alias or UUID" })
}

fn if_hash() -> Value {
    json!({ "type": "string", "description": "content_hash from your last read_note/edit; the edit is refused if the note changed since" })
}

fn agent() -> Value {
    json!({ "type": "string", "description": "Your name (e.g. claude-code); recorded as updated_by/created_by in the frontmatter" })
}

fn section() -> Value {
    json!({ "type": "string", "description": "Heading (`Status`), heading path (`Proyek#Status`) or `^anchor`, as listed in read_note's outline" })
}

/// Definitions for `tools/list`.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "vault_overview",
            "description": "Start here. One-call orientation: folder tree with note counts, most used tags, recently modified notes, titles shared by several notes (refer to those by path), the vault's own AGENTS.md instructions, and usage tips.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "list_folders",
            "description": "Every folder of the vault (hidden ones excluded) with direct and subtree note counts. Folders mirror an Obsidian vault; `folder` arguments elsewhere use these paths.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "append_note",
            "description": "Append Markdown to a note — at the end, or at the end of one section — without resending the note. Spacing and list continuation are handled. Can create the note when missing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ref": note_ref(),
                    "text": { "type": "string", "description": "Markdown to append" },
                    "section": section(),
                    "if_hash": if_hash(),
                    "agent": agent(),
                    "create_if_missing": { "type": "boolean", "default": false, "description": "Create a note titled `ref` in `folder` when no note has that name" },
                    "folder": { "type": "string", "description": "Folder of a created note" },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags of a created note" }
                },
                "required": ["ref", "text"]
            }
        }),
        json!({
            "name": "patch_note",
            "description": "Change one part of a note: replace a section's content (heading kept, subsections included) or a ^block's text (anchor kept), and/or replace an exact `old_str` (must be unique unless replace_all; searched inside the section/block when given). Use instead of write_note for targeted edits.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ref": note_ref(),
                    "section": section(),
                    "block": { "type": "string", "description": "Block anchor id, with or without ^" },
                    "old_str": { "type": "string", "description": "Exact text to replace" },
                    "new_str": { "type": "string", "description": "Replacement (for old_str) or new content (for section/block)" },
                    "replace_all": { "type": "boolean", "default": false },
                    "if_hash": if_hash(),
                    "agent": agent()
                },
                "required": ["ref", "new_str"]
            }
        }),
        json!({
            "name": "remember",
            "description": "Store a durable fact, decision or observation in the vault. Creates a note in Memory/ (or `folder`), or appends to `ref` (optionally inside `section`). Refuses when the vault already holds the same text or a near-identical chunk (status `duplicate`, with the match in `similar`). The note is indexed immediately.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "The memory, in Markdown; keep it self-contained" },
                    "title": { "type": "string", "description": "Title of a new note (default: from the first line)" },
                    "folder": { "type": "string", "default": "Memory" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "ref": { "type": "string", "description": "Append to this existing note instead of creating one" },
                    "section": section(),
                    "links": { "type": "array", "items": { "type": "string" }, "description": "Notes to link as `Related: [[…]]`" },
                    "agent": agent(),
                    "allow_duplicate": { "type": "boolean", "default": false },
                    "semantic": { "type": "boolean", "default": true, "description": "Use embeddings for the duplicate check (loads the model)" }
                },
                "required": ["text"]
            }
        }),
        json!({
            "name": "recall",
            "description": "Get what the vault knows about a topic, packed into a token budget: best-matching chunks expanded to their whole sections (with path, heading and line to cite or patch), outdated notes (valid_until passed, superseded, archived) last and flagged in `stale`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "budget_tokens": { "type": "integer", "minimum": 50, "default": 1500 },
                    "k": { "type": "integer", "minimum": 1, "default": 8, "description": "Chunks considered" },
                    "folder": { "type": "string", "description": "Only this folder (and subfolders)" },
                    "tag": { "type": "string", "description": "Only notes with this tag" },
                    "semantic": { "type": "boolean", "default": true },
                    "include_stale": { "type": "boolean", "default": false, "description": "Keep stale notes at their rank" }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "related_notes",
            "description": "Notes connected to a note, with why: links_to, linked_from, similar (embedding neighbours; needs a semantic index) and shared_tag:<tag>.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ref": note_ref(),
                    "k": { "type": "integer", "minimum": 1, "default": 10 }
                },
                "required": ["ref"]
            }
        }),
    ]
}
