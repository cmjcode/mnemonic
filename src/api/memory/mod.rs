//! Agent memory (§3.10): the vault as a long-term memory an AI agent can
//! orient itself in, read piecewise and write safely.
//!
//! - `overview`: folder tree, `vault_overview` (one-call orientation incl.
//!   the vault's own `AGENTS.md`, §3.10.1).
//! - `edit`: `read_note` of one section/block with the note's outline,
//!   `append_note` and `patch_note` (§3.10.2), all guarded by a
//!   `content_hash` → `if_hash` check and stamped with `updated_by`
//!   (§3.10.3).
//! - `recall`: `remember` (duplicate-aware), `recall` (sections packed
//!   into a token budget, stale notes last) and `related` (§3.10.4).
//! - `tools`: the MCP tool definitions of all of the above.
//!
//! Every write reloads the note from disk first (the desktop app may have
//! changed it), goes through `Note::save` (atomic, unknown frontmatter
//! kept) and refreshes the note's index row, links and chunks, so a
//! following `search`/`recall` sees it. Callers: `api::mcp`,
//! `src/bin/mnemonic-cli`.

mod edit;
mod overview;
mod recall;
pub mod tools;
pub mod types;

use std::path::Path;

use anyhow::{Context, Result, bail};

use super::VaultService;
use super::types::{NoteSummary, OutlineEntry, SelectionOut};
use crate::markdown::outline::{self, Heading, Selection, SelectionKind};

/// Frontmatter key naming the agent that last wrote a note.
pub const UPDATED_BY_KEY: &str = "updated_by";
/// Frontmatter key naming the agent that created a note.
pub const CREATED_BY_KEY: &str = "created_by";

/// FNV-1a (64-bit) of `bytes` as 16 hex digits: a stable fingerprint for
/// optimistic concurrency (not a security hash).
pub fn content_hash(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// [`content_hash`] of the file at `path` as it is on disk now.
pub fn file_hash(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(content_hash(&bytes))
}

/// The outline of `body` in API form (1-based inclusive lines).
pub fn outline_of(body: &str) -> Vec<OutlineEntry> {
    outline::headings(body)
        .into_iter()
        .map(|h| OutlineEntry {
            level: h.level,
            path: h.path_string(),
            text: h.text,
            anchor: h.anchor,
            line: h.line + 1,
            end_line: h.end.max(h.line + 1),
        })
        .collect()
}

pub(crate) fn selection_out(sel: &Selection) -> SelectionOut {
    SelectionOut {
        kind: match sel.kind {
            SelectionKind::Section => "section",
            SelectionKind::Block => "block",
        },
        label: sel.label.clone(),
        line: sel.start + 1,
        end_line: sel.end.max(sel.start + 1),
    }
}

/// `section` or `^block` spec of a request, whichever is given.
pub(crate) fn part_spec(section: Option<&str>, block: Option<&str>) -> Option<String> {
    section
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            block
                .map(|b| b.trim().trim_start_matches('^'))
                .filter(|b| !b.is_empty())
                .map(|b| format!("^{b}"))
        })
}

/// Lowercase with whitespace collapsed — for "is this text already here".
pub(crate) fn normalize_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// 1-based line of byte `offset` in `body` and the innermost heading
/// whose section contains it.
pub(crate) fn locate(body: &str, offset: usize) -> (usize, Option<Heading>) {
    let mut off = offset.min(body.len());
    while !body.is_char_boundary(off) {
        off -= 1;
    }
    let line = body[..off].matches('\n').count();
    let heading = outline::headings(body)
        .into_iter()
        .rev()
        .find(|h| h.line <= line && line < h.end);
    (line + 1, heading)
}

/// Rough token count used for budgets (≈ 4 characters per token).
pub(crate) fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

impl VaultService {
    /// Resolves `reference`, reloads the note from disk (picking up edits
    /// made meanwhile by the app or another agent) and, when `if_hash` is
    /// given, refuses to continue if the file no longer matches it.
    pub(crate) fn open_for_edit(&mut self, reference: &str, if_hash: Option<&str>) -> Result<usize> {
        let idx = self.resolve_index(reference)?;
        self.reload_note(idx)?;
        let note = &self.vault.notes[idx];
        if note.is_canvas() {
            bail!(
                "`{}` is a canvas note (its body is a diagram); edit it in the app",
                self.rel(&note.path)
            );
        }
        self.check_if_hash(idx, if_hash)?;
        Ok(idx)
    }

    /// Fails when `if_hash` is given and note `idx`'s file on disk no
    /// longer has that fingerprint.
    pub(crate) fn check_if_hash(&self, idx: usize, if_hash: Option<&str>) -> Result<()> {
        let Some(expected) = if_hash.map(str::trim).filter(|h| !h.is_empty()) else {
            return Ok(());
        };
        let note = &self.vault.notes[idx];
        let current = file_hash(&note.path)?;
        if current != expected {
            bail!(
                "`{}` changed since it was read (content_hash is now {current}, if_hash was {expected}); read it again and redo the edit",
                self.rel(&note.path)
            );
        }
        Ok(())
    }

    /// Records `agent` as `updated_by` (and `created_by` for a new note)
    /// in the frontmatter. Returns a warning when the frontmatter is not
    /// valid YAML and can't carry it.
    pub(crate) fn stamp_agent(&mut self, idx: usize, agent: Option<&str>, created: bool) -> Option<String> {
        let agent = agent.map(str::trim).filter(|a| !a.is_empty())?;
        let fm = &mut self.vault.notes[idx].frontmatter;
        if fm.unparsed_header.is_some() {
            return Some(format!(
                "frontmatter is not valid YAML; `{UPDATED_BY_KEY}: {agent}` was not recorded"
            ));
        }
        let value = serde_yaml::Value::String(agent.to_string());
        if created {
            fm.extra.insert(CREATED_BY_KEY.to_string(), value.clone());
        }
        fm.extra.insert(UPDATED_BY_KEY.to_string(), value);
        None
    }

    /// Saves note `idx`, reloads it, refreshes its index row, the link
    /// resolver and its chunks. Returns its summary and new content hash;
    /// a chunking failure only adds a warning (the file is saved).
    pub(crate) fn commit_note(&mut self, idx: usize, warnings: &mut Vec<String>) -> Result<(NoteSummary, String)> {
        {
            let note = &mut self.vault.notes[idx];
            note.save()
                .with_context(|| format!("saving note {}", note.path.display()))?;
        }
        self.reload_note(idx)?;
        let note = &self.vault.notes[idx];
        self.index
            .upsert_note(note)
            .with_context(|| format!("indexing note {}", note.path.display()))?;
        let hash = file_hash(&note.path)?;
        let summary = self.summary(note);
        self.refresh_links();
        if let Err(e) = self.refresh_chunks(idx) {
            warnings.push(format!("saved, but search chunks were not refreshed: {e:#}"));
        }
        Ok((summary, hash))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_stable_fnv1a() {
        assert_eq!(content_hash(b""), "cbf29ce484222325");
        assert_eq!(content_hash(b"a"), "af63dc4c8601ec8c");
        assert_ne!(content_hash(b"ab"), content_hash(b"ba"));
    }

    #[test]
    fn locate_finds_line_and_innermost_heading() {
        let body = "intro\n# A\nx\n## B\ny\n";
        let (line, h) = locate(body, body.find('y').unwrap());
        assert_eq!(line, 5);
        assert_eq!(h.unwrap().path_string(), "A#B");
        assert!(locate(body, 0).1.is_none());
        assert_eq!(part_spec(None, Some("^abc")).as_deref(), Some("^abc"));
        assert_eq!(part_spec(Some(" "), Some("abc")).as_deref(), Some("^abc"));
        assert_eq!(estimate_tokens("abcde"), 2);
    }
}
