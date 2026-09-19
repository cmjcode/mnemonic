//! `mnemonic-cli` subcommands of the agent-memory layer (§3.10): `folders`,
//! `overview`, `notes append`, `notes patch`, `remember`, `recall` and
//! `related`. Thin wrappers over `VaultService`; `--json` prints the
//! service result verbatim, otherwise a compact text rendering.

use anyhow::{Result, bail};
use clap::Args;

use mnemonic::api::VaultService;
use mnemonic::api::types::*;

use crate::{BodySource, emit};

#[derive(Args)]
pub struct AppendArgs {
    /// Note: vault-relative path, title, alias or UUID.
    r#ref: String,
    /// Text to append (or use --body-file / --stdin).
    #[arg(long, conflicts_with_all = ["body_file", "stdin"])]
    text: Option<String>,
    #[command(flatten)]
    body: BodySource,
    /// Append at the end of this section (`Heading`, `Parent#Child`, `^id`).
    #[arg(long)]
    section: Option<String>,
    /// Refuse if the note's content_hash is no longer this.
    #[arg(long)]
    if_hash: Option<String>,
    /// Recorded as updated_by / created_by.
    #[arg(long)]
    agent: Option<String>,
    /// Create the note (titled REF) when no note has that name.
    #[arg(long)]
    create: bool,
    /// Folder of a created note.
    #[arg(long)]
    folder: Option<String>,
    /// Tags of a created note (repeatable).
    #[arg(long = "tag")]
    tags: Vec<String>,
}

#[derive(Args)]
pub struct PatchArgs {
    r#ref: String,
    /// Replace this section's content (heading kept).
    #[arg(long)]
    section: Option<String>,
    /// Replace this anchored block's text (anchor kept).
    #[arg(long)]
    block: Option<String>,
    /// Exact text to replace (inside --section/--block when given).
    #[arg(long)]
    old: Option<String>,
    /// Replacement text (or use --body-file / --stdin).
    #[arg(long = "new", conflicts_with_all = ["body_file", "stdin"])]
    new_text: Option<String>,
    #[command(flatten)]
    body: BodySource,
    /// Replace every occurrence of --old.
    #[arg(long)]
    replace_all: bool,
    #[arg(long)]
    if_hash: Option<String>,
    #[arg(long)]
    agent: Option<String>,
}

#[derive(Args)]
pub struct RememberArgs {
    /// The memory (or use --body-file / --stdin).
    #[arg(conflicts_with_all = ["body_file", "stdin"])]
    text: Option<String>,
    #[command(flatten)]
    body: BodySource,
    /// Title of a new memory note (default: from the first line).
    #[arg(long)]
    title: Option<String>,
    /// Folder of a new memory note (default: Memory).
    #[arg(long)]
    folder: Option<String>,
    #[arg(long = "tag")]
    tags: Vec<String>,
    /// Append to this existing note instead.
    #[arg(long)]
    into: Option<String>,
    /// With --into: append inside this section.
    #[arg(long)]
    section: Option<String>,
    /// Link a related note (repeatable).
    #[arg(long = "link")]
    links: Vec<String>,
    #[arg(long)]
    agent: Option<String>,
    /// Write even if the vault already holds it.
    #[arg(long)]
    allow_duplicate: bool,
    /// Duplicate check without the embedding model.
    #[arg(long)]
    keyword_only: bool,
}

#[derive(Args)]
pub struct RecallArgs {
    query: String,
    /// Approximate token budget.
    #[arg(long, default_value_t = 1500)]
    budget: usize,
    /// Chunks considered.
    #[arg(short, long, default_value_t = 8)]
    k: usize,
    #[arg(long)]
    folder: Option<String>,
    #[arg(long)]
    tag: Option<String>,
    #[arg(long)]
    keyword_only: bool,
    /// Keep stale notes at their rank.
    #[arg(long)]
    include_stale: bool,
}

/// Text from `--text`/positional, else --body-file/--stdin.
fn text_of(inline: Option<String>, body: &BodySource, what: &str) -> Result<String> {
    match inline.or(body.read()?) {
        Some(t) => Ok(t),
        None => bail!("no {what}: pass it inline, with --body-file or --stdin"),
    }
}

fn edit_lines(out: &mut Vec<String>, res: &EditNoteResult) {
    let what = if res.created {
        "created"
    } else if res.changed {
        "updated"
    } else {
        "unchanged"
    };
    let part = res.selection.as_ref().map(|s| format!(" [{}]", s.label)).unwrap_or_default();
    out.push(format!("{what} {}{part} (content_hash {})", res.note.path, res.content_hash));
    out.extend(res.warnings.iter().map(|w| format!("warning: {w}")));
}

pub fn append(json: bool, svc: &mut VaultService, a: AppendArgs) -> Result<()> {
    let text = text_of(a.text, &a.body, "text to append")?;
    let res = svc.append_note(&AppendNoteRequest {
        r#ref: a.r#ref,
        text,
        section: a.section,
        if_hash: a.if_hash,
        agent: a.agent,
        create_if_missing: a.create,
        folder: a.folder,
        tags: a.tags,
    })?;
    emit(json, &res, |out| edit_lines(out, &res))
}

pub fn patch(json: bool, svc: &mut VaultService, a: PatchArgs) -> Result<()> {
    let new_str = text_of(a.new_text, &a.body, "replacement text")?;
    let res = svc.patch_note(&PatchNoteRequest {
        r#ref: a.r#ref,
        section: a.section,
        block: a.block,
        old_str: a.old,
        new_str,
        replace_all: a.replace_all,
        if_hash: a.if_hash,
        agent: a.agent,
    })?;
    emit(json, &res, |out| {
        edit_lines(out, &res);
        if res.replacements > 0 {
            out.push(format!("{} replacement(s)", res.replacements));
        }
    })
}

pub fn folders(json: bool, svc: &VaultService) -> Result<()> {
    let list = svc.list_folders();
    emit(json, &list, |out| {
        for f in &list.folders {
            out.push(format!("{}{}/  {} ({} total)", "  ".repeat(f.depth), f.name, f.notes, f.notes_total));
        }
    })
}

pub fn overview(json: bool, svc: &VaultService) -> Result<()> {
    let o = svc.vault_overview();
    emit(json, &o, |out| {
        out.push(format!(
            "{}: {} notes, {} canvases, {} sheets, {} PDFs, {} folders",
            o.name,
            o.notes,
            o.canvases,
            o.sheets,
            o.pdfs,
            o.folders.len()
        ));
        let top: Vec<String> = o.tags.iter().take(15).map(|t| format!("#{} ({})", t.tag, t.count)).collect();
        if !top.is_empty() {
            out.push(format!("tags: {}", top.join(", ")));
        }
        out.push("recent:".into());
        out.extend(o.recent.iter().map(|r| format!("  {}  {}", r.modified.format("%Y-%m-%d %H:%M"), r.note.path)));
        for d in &o.duplicate_titles {
            out.push(format!("duplicate title `{}`: {}", d.title, d.paths.join(", ")));
        }
        match &o.guide {
            Some(g) => {
                out.push(format!("--- {} ---", g.path));
                out.push(g.text.trim_end().to_string());
            }
            None => out.push("(no AGENTS.md at the vault root)".into()),
        }
    })
}

pub fn remember(json: bool, svc: &mut VaultService, a: RememberArgs) -> Result<()> {
    let text = text_of(a.text, &a.body, "memory text")?;
    let res = svc.remember(&RememberRequest {
        text,
        title: a.title,
        folder: a.folder,
        tags: a.tags,
        r#ref: a.into,
        section: a.section,
        links: a.links,
        agent: a.agent,
        allow_duplicate: a.allow_duplicate,
        semantic: !a.keyword_only,
    })?;
    emit(json, &res, |out| {
        match &res.note {
            Some(n) => out.push(format!("{} {}", res.status, n.path)),
            None => out.push(format!("{}: nothing written", res.status)),
        }
        for s in &res.similar {
            let score = s.score.map(|x| format!(" {x:.2}")).unwrap_or_default();
            out.push(format!("  similar [{}{score}] {} — {}", s.kind, s.path, s.snippet.replace('\n', " ")));
        }
        out.extend(res.warnings.iter().map(|w| format!("warning: {w}")));
    })
}

pub fn recall(json: bool, svc: &mut VaultService, a: RecallArgs) -> Result<()> {
    let res = svc.recall(&RecallRequest {
        query: a.query,
        budget_tokens: a.budget,
        k: a.k,
        folder: a.folder,
        tag: a.tag,
        semantic: !a.keyword_only,
        include_stale: a.include_stale,
    })?;
    emit(json, &res, |out| {
        out.extend(res.warnings.iter().map(|w| format!("warning: {w}")));
        if res.items.is_empty() {
            out.push("(nothing found)".into());
        }
        for i in &res.items {
            let at = i.section.as_deref().map(|s| format!("#{s}")).unwrap_or_default();
            let stale = i.stale.as_deref().map(|s| format!("  [stale: {s}]")).unwrap_or_default();
            out.push(format!("── {}{at} (line {}){stale}", i.path, i.line.unwrap_or(0)));
            out.push(i.text.clone());
        }
        out.push(format!(
            "({} of ~{} tokens used{})",
            res.used_tokens,
            res.budget_tokens,
            if res.omitted > 0 { format!(", {} more omitted", res.omitted) } else { String::new() }
        ));
    })
}

pub fn related(json: bool, svc: &mut VaultService, r#ref: String, k: usize) -> Result<()> {
    let res = svc.related(&RelatedRequest { r#ref, k })?;
    emit(json, &res, |out| {
        out.push(format!("related to {}:", res.note.path));
        for r in &res.related {
            let score = r.score.map(|s| format!(" {s:.2}")).unwrap_or_default();
            out.push(format!("  {} — {}{score}", r.path, r.reasons.join(", ")));
        }
        if res.related.is_empty() {
            out.push("  (none)".into());
        }
        out.extend(res.warnings.iter().map(|w| format!("note: {w}")));
    })
}
