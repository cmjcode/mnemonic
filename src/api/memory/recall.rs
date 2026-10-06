//! Memory semantics on top of search (§3.10.4): `remember` stores a fact
//! unless the vault already holds it (exact text, or a near-identical
//! chunk by embedding), `recall` expands the best chunks to their sections
//! and packs them into a token budget with outdated notes last, and
//! `related` lists a note's link neighbours, embedding neighbours and
//! tag mates. Staleness comes from frontmatter the user controls:
//! `valid_until`, `superseded_by`, another note's `supersedes`, `archived`.
//! Callers: `api::memory::tools`, `api::mcp`, `src/bin/mnemonic-cli`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{Result, bail};
use chrono::NaiveDate;

use super::types::*;
use super::{estimate_tokens, file_hash, locate, normalize_text};
use crate::api::VaultService;
use crate::api::index::{HitFilter, kind_str, strip_highlights};
use crate::api::types::{AppendNoteRequest, CreateNoteRequest};
use crate::markdown::outline;
use crate::markdown::wikilink;
use crate::notes::Note;

/// Folder new memories go to unless the request names one.
pub const DEFAULT_MEMORY_FOLDER: &str = "Memory";
/// Cosine similarity at or above which a chunk counts as the same fact.
const DUPLICATE_SIMILARITY: f32 = 0.92;
/// Similar notes reported by `remember` need at least this similarity.
const SIMILAR_REPORT: f32 = 0.75;
/// A section longer than this is represented by its matching chunk only.
const MAX_SECTION_CHARS: usize = 6000;
/// A heading-less note up to this size is returned whole.
const MAX_WHOLE_NOTE_CHARS: usize = 2000;
/// Titles derived from text are cut to about this many characters.
const MAX_TITLE_CHARS: usize = 60;

/// A title for a memory: the text's first line without Markdown markers
/// or characters file systems reject, cut at a word boundary.
fn derive_title(text: &str) -> String {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let cleaned: String = first
        .trim_start_matches(['#', '-', '*', '>', '+', ' '])
        .chars()
        .map(|c| if "/\\:*?\"<>|[]^#".contains(c) { ' ' } else { c })
        .collect();
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    let mut title = String::new();
    for w in words {
        if !title.is_empty() && title.chars().count() + 1 + w.chars().count() > MAX_TITLE_CHARS {
            break;
        }
        if !title.is_empty() {
            title.push(' ');
        }
        title.push_str(w);
    }
    let title: String = title.chars().take(MAX_TITLE_CHARS).collect();
    if title.is_empty() {
        format!("Memory {}", chrono::Local::now().format("%Y-%m-%d %H%M"))
    } else {
        title
    }
}

/// Strings in a frontmatter value (a string or a list of strings).
fn yaml_strings(v: &serde_yaml::Value) -> Vec<String> {
    match v {
        serde_yaml::Value::String(s) => vec![s.clone()],
        serde_yaml::Value::Sequence(items) => items.iter().flat_map(yaml_strings).collect(),
        _ => Vec::new(),
    }
}

/// `[[Target|x]]` / `Target` → `Target`.
fn link_target(s: &str) -> String {
    let inner = s.trim().trim_start_matches("[[").trim_end_matches("]]");
    wikilink::WikiLink::parse(inner).target
}

/// First `max` characters of `text`, with `…` when cut.
fn clip(text: &str, max: usize) -> (String, bool) {
    if text.chars().count() <= max {
        return (text.to_string(), false);
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    (out, true)
}

impl VaultService {
    /// Why each outdated note is outdated, keyed by path.
    pub(crate) fn stale_notes(&self) -> HashMap<PathBuf, String> {
        let today = chrono::Local::now().date_naive();
        let mut out = HashMap::new();
        for n in self.vault.notes.iter().filter(|n| !n.frontmatter.trashed) {
            let extra = &n.frontmatter.extra;
            if n.frontmatter.archived {
                out.insert(n.path.clone(), "archived".to_string());
            }
            if let Some(until) = extra.get("valid_until").and_then(|v| yaml_strings(v).into_iter().next())
                && let Ok(date) = NaiveDate::parse_from_str(until.trim().get(..10).unwrap_or(""), "%Y-%m-%d")
                && date < today
            {
                out.insert(n.path.clone(), format!("valid_until {date}"));
            }
            if let Some(by) = extra.get("superseded_by").map(yaml_strings).and_then(|v| v.into_iter().next())
                && !by.trim().is_empty()
            {
                out.insert(n.path.clone(), format!("superseded_by {}", by.trim()));
            }
            for target in extra.get("supersedes").map(yaml_strings).unwrap_or_default() {
                if let Some(old) = self.links.resolve_from(&link_target(&target), Some(&n.path))
                    && old != n.path
                {
                    out.insert(old.to_path_buf(), format!("superseded by [[{}]]", n.frontmatter.title));
                }
            }
        }
        out
    }

    /// Stores `text` as a memory: appended to `ref` (optionally inside
    /// `section`), or as a new note in `folder` (default `Memory/`).
    /// Nothing is written when the vault already holds it, unless
    /// `allow_duplicate`.
    pub fn remember(&mut self, req: &RememberRequest) -> Result<RememberResult> {
        let text = req.text.trim();
        if text.is_empty() {
            bail!("nothing to remember: `text` is empty");
        }
        let mut warnings = Vec::new();
        let target = match req.r#ref.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
            Some(r) => Some(self.resolve_index(r)?),
            None => None,
        };

        let needle = normalize_text(text);
        let mut similar: Vec<SimilarNote> = self
            .vault
            .notes
            .iter()
            .enumerate()
            .filter(|(i, n)| !n.frontmatter.trashed && !n.is_canvas() && target.is_none_or(|t| t == *i))
            .filter(|(_, n)| normalize_text(&n.body).contains(&needle))
            .map(|(_, n)| SimilarNote {
                path: self.rel(&n.path),
                title: n.frontmatter.title.clone(),
                score: Some(1.0),
                kind: "exact",
                heading: None,
                snippet: clip(text, 160).0,
            })
            .collect();
        let mut duplicate = !similar.is_empty();

        if !duplicate {
            let (hits, semantic, w) = self.retrieve(text, 3, req.semantic, true, &HitFilter::default())?;
            warnings.extend(w);
            let target_id = target.map(|t| self.vault.notes[t].frontmatter.id);
            for h in hits {
                let score = h.score.filter(|s| s.is_finite());
                let close = score.is_some_and(|s| s >= SIMILAR_REPORT);
                if semantic && !close {
                    continue;
                }
                if target_id.is_some_and(|id| id != h.chunk.doc_id) {
                    continue;
                }
                duplicate |= semantic && score.is_some_and(|s| s >= DUPLICATE_SIMILARITY);
                let (heading, _) = self.chunk_position(&h.chunk);
                similar.push(SimilarNote {
                    path: self.rel(&h.chunk.file_path),
                    title: self.doc_title(&h.chunk),
                    score,
                    kind: kind_str(h.kind),
                    heading,
                    snippet: clip(&h.snippet.as_deref().map(strip_highlights).unwrap_or(h.chunk.text_content), 200).0,
                });
            }
        }
        if duplicate && !req.allow_duplicate {
            return Ok(RememberResult {
                status: "duplicate",
                note: None,
                content_hash: None,
                similar,
                warnings,
            });
        }

        let mut body = text.to_string();
        let links: Vec<String> = req
            .links
            .iter()
            .map(|l| link_target(l))
            .filter(|l| !l.is_empty())
            .map(|l| format!("[[{l}]]"))
            .collect();
        if !links.is_empty() {
            body.push_str(&format!("\n\nRelated: {}", links.join(" · ")));
        }

        match target {
            Some(idx) => {
                let path = self.rel(&self.vault.notes[idx].path);
                let res = self.append_note(&AppendNoteRequest {
                    r#ref: path,
                    text: body,
                    section: req.section.clone(),
                    agent: req.agent.clone(),
                    ..Default::default()
                })?;
                warnings.extend(res.warnings);
                Ok(RememberResult {
                    status: "appended",
                    note: Some(res.note),
                    content_hash: Some(res.content_hash),
                    similar,
                    warnings,
                })
            }
            None => {
                let folder = req
                    .folder
                    .clone()
                    .filter(|f| !f.trim().is_empty())
                    .unwrap_or_else(|| DEFAULT_MEMORY_FOLDER.to_string());
                let title = req
                    .title
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| derive_title(text));
                let note = self.create_note_with_warnings(
                    &CreateNoteRequest {
                        title,
                        body: body + "\n",
                        folder: Some(folder),
                        tags: req.tags.clone(),
                        agent: Some(req.agent.clone().unwrap_or_else(|| "agent".to_string())),
                    },
                    &mut warnings,
                )?;
                let content_hash = file_hash(&self.root().join(&note.path))?;
                Ok(RememberResult {
                    status: "created",
                    note: Some(note),
                    content_hash: Some(content_hash),
                    similar,
                    warnings,
                })
            }
        }
    }

    /// The vault's knowledge about `query`, as whole sections where they
    /// fit, within `budget_tokens`; outdated notes go last.
    pub fn recall(&mut self, req: &RecallRequest) -> Result<RecallResult> {
        let query = req.query.trim().to_string();
        if query.is_empty() {
            bail!("empty query");
        }
        let budget = req.budget_tokens.max(50);
        let filter = HitFilter::new(req.folder.as_deref(), req.tag.as_deref());
        let (hits, semantic, warnings) = self.retrieve(&query, req.k.max(1), req.semantic, false, &filter)?;
        let stale = self.stale_notes();

        let mut items: Vec<RecallItem> = Vec::new();
        let mut seen: HashSet<(PathBuf, usize)> = HashSet::new();
        let mut whole: HashSet<PathBuf> = HashSet::new();
        for h in hits {
            let path = h.chunk.file_path.clone();
            if whole.contains(&path) {
                continue;
            }
            let note = self
                .vault
                .notes
                .iter()
                .find(|n| n.frontmatter.id == h.chunk.doc_id && !n.is_canvas());
            let (text, section, line, key) = match note {
                Some(n) => {
                    let (line, heading) = locate(&n.body, h.chunk.char_offset);
                    match heading {
                        Some(hd) => {
                            let sel = outline::Selection {
                                kind: outline::SelectionKind::Section,
                                start: hd.line,
                                end: hd.end,
                                label: hd.path_string(),
                                block_id: None,
                            };
                            let section_text = outline::selection_text(&n.body, &sel);
                            if section_text.chars().count() <= MAX_SECTION_CHARS {
                                (section_text, Some(hd.path_string()), Some(hd.line + 1), hd.line)
                            } else {
                                (h.chunk.text_content.clone(), Some(hd.path_string()), Some(line), line)
                            }
                        }
                        None if n.body.chars().count() <= MAX_WHOLE_NOTE_CHARS => {
                            whole.insert(path.clone());
                            (n.body.trim().to_string(), None, Some(1), 0)
                        }
                        None => (h.chunk.text_content.clone(), None, Some(line), line),
                    }
                }
                None => (h.chunk.text_content.clone(), None, None, h.chunk.char_offset),
            };
            if !seen.insert((path.clone(), key)) {
                continue;
            }
            items.push(RecallItem {
                path: self.rel(&path),
                title: self.doc_title(&h.chunk),
                section,
                line,
                tokens: estimate_tokens(&text),
                text,
                score: h.score.filter(|s| s.is_finite()),
                kind: kind_str(h.kind),
                stale: stale.get(&path).cloned(),
                truncated: false,
            });
        }
        if !req.include_stale {
            items.sort_by_key(|i| i.stale.is_some());
        }

        let mut used = 0;
        let mut kept = Vec::new();
        let mut omitted = 0;
        for mut item in items {
            let left = budget.saturating_sub(used);
            if item.tokens <= left {
                used += item.tokens;
                kept.push(item);
            } else if left >= 64 || kept.is_empty() {
                let (text, _) = clip(&item.text, left * 4);
                item.tokens = estimate_tokens(&text);
                item.text = text;
                item.truncated = true;
                used += item.tokens;
                kept.push(item);
            } else {
                omitted += 1;
            }
        }
        Ok(RecallResult {
            query,
            semantic,
            budget_tokens: budget,
            used_tokens: used,
            items: kept,
            omitted,
            warnings,
        })
    }

    /// Notes connected to `ref`: outgoing links, backlinks, embedding
    /// neighbours (needs a semantic index) and notes sharing a tag.
    pub fn related(&mut self, req: &RelatedRequest) -> Result<RelatedResult> {
        let idx = self.resolve_index(&req.r#ref)?;
        let note = self.vault.notes[idx].clone();
        let k = req.k.max(1);
        let mut warnings = Vec::new();
        let mut found: Vec<(PathBuf, RelatedNote)> = Vec::new();
        let add = |found: &mut Vec<(PathBuf, RelatedNote)>, other: &Note, reason: String, score: Option<f32>| {
            if other.path == note.path || other.frontmatter.trashed {
                return;
            }
            match found.iter_mut().find(|(p, _)| *p == other.path) {
                Some((_, r)) => {
                    if !r.reasons.contains(&reason) {
                        r.reasons.push(reason);
                    }
                    r.score = r.score.or(score);
                }
                None => found.push((
                    other.path.clone(),
                    RelatedNote {
                        path: self.rel(&other.path),
                        title: other.frontmatter.title.clone(),
                        reasons: vec![reason],
                        score,
                    },
                )),
            }
        };
        let by_path = |p: &std::path::Path| self.vault.notes.iter().find(|n| n.path == p);

        if !note.is_canvas() {
            for occ in wikilink::parse_wikilinks(&note.body) {
                if let Some(other) = self.links.resolve_from(&occ.link.target, Some(&note.path)).and_then(by_path) {
                    add(&mut found, other, "links_to".into(), None);
                }
            }
        }
        for b in self.backlinks(&self.rel(&note.path))?.backlinks {
            if let Some(other) = by_path(&self.root().join(&b.source_path)) {
                add(&mut found, other, "linked_from".into(), None);
            }
        }
        let similar = self.index.similar_documents(note.frontmatter.id, k).unwrap_or_default();
        if similar.is_empty() {
            warnings.push("no embedding neighbours: index this note semantically (`reindex` without keyword_only) to get `similar`".to_string());
        }
        for (id, score) in similar {
            if let Some(other) = self.vault.notes.iter().find(|n| n.frontmatter.id == id) {
                add(&mut found, other, "similar".into(), Some(score));
            }
        }
        let tags: Vec<String> = note.frontmatter.tags.iter().map(|t| t.to_lowercase()).collect();
        if !tags.is_empty() {
            for other in &self.vault.notes {
                if let Some(t) = other.frontmatter.tags.iter().find(|t| tags.contains(&t.to_lowercase())) {
                    add(&mut found, other, format!("shared_tag:{t}"), None);
                }
            }
        }

        let mut related: Vec<RelatedNote> = found.into_iter().map(|(_, r)| r).collect();
        related.sort_by(|a, b| {
            b.reasons
                .len()
                .cmp(&a.reasons.len())
                .then(b.score.unwrap_or(0.0).total_cmp(&a.score.unwrap_or(0.0)))
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        });
        related.truncate(k);
        Ok(RelatedResult {
            note: self.note_ref(&note),
            related,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::ReindexOptions;
    use tempfile::tempdir;

    fn keyword_svc(notes: &[(&str, &str)]) -> (tempfile::TempDir, VaultService) {
        let dir = tempdir().unwrap();
        for (title, body) in notes {
            Note::create(dir.path(), title, body).unwrap();
        }
        let mut svc = VaultService::open(dir.path()).unwrap();
        svc.reindex(ReindexOptions { full: false, keyword_only: true }).unwrap();
        (dir, svc)
    }

    #[test]
    fn derive_title_cleans_and_cuts() {
        assert_eq!(derive_title("## Server: prod/db pakai Postgres 16\nisi"), "Server prod db pakai Postgres 16");
        let long = "kata ".repeat(40);
        assert!(derive_title(&long).chars().count() <= MAX_TITLE_CHARS);
        assert!(derive_title("  \n ").starts_with("Memory "));
    }

    #[test]
    fn remember_creates_then_refuses_duplicate_and_recall_finds_it() {
        let (d, mut svc) = keyword_svc(&[("Server", "# Infra\nDatabase utama pakai Postgres.\n")]);
        let req = RememberRequest {
            text: "Deploy produksi setiap Kamis jam 20.00 WIB.".into(),
            tags: vec!["ops".into()],
            links: vec!["[[Server]]".into()],
            agent: Some("tester".into()),
            semantic: false,
            ..Default::default()
        };
        let res = svc.remember(&req).unwrap();
        assert_eq!(res.status, "created");
        let note = res.note.unwrap();
        assert!(note.path.starts_with("Memory/Deploy produksi setiap Kamis"), "{}", note.path);
        let raw = std::fs::read_to_string(d.path().join(&note.path)).unwrap();
        assert!(raw.contains("created_by: tester") && raw.contains("Related: [[Server]]"), "{raw}");

        let again = svc.remember(&req).unwrap();
        assert_eq!(again.status, "duplicate");
        assert_eq!(again.similar[0].kind, "exact");

        // Chunked on write: recall sees it without a reindex.
        let got = svc
            .recall(&RecallRequest { query: "Kamis".into(), semantic: false, ..Default::default() })
            .unwrap();
        assert_eq!(got.items.len(), 1);
        assert!(got.items[0].text.contains("Kamis"));
        let into = svc
            .remember(&RememberRequest {
                text: "Port Postgres 5433.".into(),
                r#ref: Some("Server".into()),
                section: Some("Infra".into()),
                semantic: false,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(into.status, "appended");
        let raw = std::fs::read_to_string(d.path().join("Server.md")).unwrap();
        assert!(raw.ends_with("Database utama pakai Postgres.\n\nPort Postgres 5433.\n"), "{raw}");
    }

    #[test]
    fn recall_returns_sections_in_budget_with_stale_last() {
        let (d, mut svc) = keyword_svc(&[("Baru", "# Kebijakan\nCuti 14 hari per tahun.\n\n# Lain\nTidak terkait.\n")]);
        std::fs::write(
            d.path().join("Lama.md"),
            "---\ntitle: Lama\nsuperseded_by: \"[[Baru]]\"\n---\n# Kebijakan\nCuti 12 hari per tahun.\n",
        )
        .unwrap();
        svc.reindex(ReindexOptions { full: false, keyword_only: true }).unwrap();
        let got = svc
            .recall(&RecallRequest { query: "cuti".into(), semantic: false, ..Default::default() })
            .unwrap();
        assert_eq!(got.items.len(), 2);
        assert_eq!(got.items[0].title, "Baru");
        assert_eq!(got.items[0].section.as_deref(), Some("Kebijakan"));
        assert_eq!(got.items[0].text, "# Kebijakan\nCuti 14 hari per tahun.");
        assert!(got.items[0].stale.is_none());
        assert_eq!(got.items[1].stale.as_deref(), Some("superseded_by [[Baru]]"));

        let tiny = svc
            .recall(&RecallRequest { query: "cuti".into(), semantic: false, budget_tokens: 50, ..Default::default() })
            .unwrap();
        assert!(tiny.used_tokens <= 50, "{}", tiny.used_tokens);
    }

    #[test]
    fn related_combines_links_backlinks_and_tags() {
        let dir = tempdir().unwrap();
        let mut a = Note::create(dir.path(), "A", "ke [[B]]").unwrap();
        a.frontmatter.tags = vec!["x".into()];
        a.save().unwrap();
        Note::create(dir.path(), "B", "balik [[A]]").unwrap();
        let mut c = Note::create(dir.path(), "C", "").unwrap();
        c.frontmatter.tags = vec!["X".into()];
        c.save().unwrap();
        let mut svc = VaultService::open(dir.path()).unwrap();
        let res = svc.related(&RelatedRequest { r#ref: "A".into(), k: 10 }).unwrap();
        assert_eq!(res.related[0].title, "B");
        assert_eq!(res.related[0].reasons, vec!["links_to", "linked_from"]);
        assert_eq!(res.related[1].reasons, vec!["shared_tag:X"]);
    }
}
