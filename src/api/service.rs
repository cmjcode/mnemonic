//! `VaultService`: the egui-free façade external agents drive (§Fase 2).
//! Owns an open `Vault`, its `IndexStore` and a `WikilinkIndex`, and
//! exposes note CRUD plus link/graph queries. Retrieval (`search`, `ask`)
//! and `reindex` live in `api::index` as further `impl` blocks on the same
//! type. Every method returns the serializable types in `api::types` with
//! vault-relative paths. Callers: `api::mcp`, `src/bin/mnemonic-cli.rs`.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use uuid::Uuid;

use super::index::{LazyEmbedder, LazyGenerator};
use super::types::*;
use crate::core::IndexStore;
use crate::graph::model::{EdgeKind, GraphData, GraphOptions, NodeKind};
use crate::markdown::wikilink::{self, WikilinkIndex};
use crate::notes::{Note, Vault};

/// One open vault plus everything needed to answer agent requests.
pub struct VaultService {
    pub(super) vault: Vault,
    pub(super) index: IndexStore,
    pub(super) links: WikilinkIndex,
    pub(super) embedder: LazyEmbedder,
    pub(super) generator: LazyGenerator,
}

impl VaultService {
    /// Opens `root` (created when missing), its SQLite index, and refreshes
    /// the note/link tables from disk so backlinks and the graph reflect
    /// the current files even if the desktop app never ran here. Chunks
    /// and vectors are only touched by `reindex`.
    pub fn open(root: impl Into<PathBuf>) -> Result<VaultService> {
        let root: PathBuf = root.into();
        std::fs::create_dir_all(&root)
            .with_context(|| format!("creating vault root {}", root.display()))?;
        let root = root
            .canonicalize()
            .with_context(|| format!("resolving vault path {}", root.display()))?;
        let vault = Vault::open(root.clone())
            .with_context(|| format!("opening vault {}", root.display()))?;
        let mut index = IndexStore::open(&root)
            .with_context(|| format!("opening index of {}", root.display()))?;
        index
            .rebuild(&vault.notes)
            .context("refreshing notes index")?;
        let links = WikilinkIndex::build(&vault.notes);
        Ok(VaultService {
            vault,
            index,
            links,
            embedder: LazyEmbedder::default(),
            generator: LazyGenerator::default(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.vault.root
    }

    /// Vault-relative, `/`-separated form of an absolute note path.
    pub(super) fn rel(&self, path: &Path) -> String {
        let rel = path.strip_prefix(&self.vault.root).unwrap_or(path);
        rel.components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    }

    pub(super) fn summary(&self, note: &Note) -> NoteSummary {
        summarize(note, self.rel(&note.path))
    }

    fn note_ref(&self, note: &Note) -> NoteRefOut {
        NoteRefOut {
            id: note.frontmatter.id,
            title: note.frontmatter.title.clone(),
            path: self.rel(&note.path),
        }
    }

    /// Rebuilds the wikilink resolver after any change to `vault.notes`.
    pub(super) fn refresh_links(&mut self) {
        self.links = WikilinkIndex::build(&self.vault.notes);
    }

    /// Absolute path of a vault-relative `folder`, rejecting anything that
    /// would escape the vault (`..`, absolute paths) or land in hidden
    /// state folders.
    fn folder_path(&self, folder: Option<&str>) -> Result<PathBuf> {
        let folder = folder.unwrap_or("").trim().trim_matches('/');
        let mut out = self.vault.root.clone();
        for component in Path::new(folder).components() {
            match component {
                Component::Normal(part) => {
                    let s = part.to_string_lossy();
                    if s.starts_with('.') {
                        bail!("folder `{folder}` points into a hidden folder");
                    }
                    out.push(part);
                }
                Component::CurDir => {}
                _ => bail!("folder `{folder}` must be relative to the vault root"),
            }
        }
        Ok(out)
    }

    // ─── Resolution ──────────────────────────────────────────────────────

    /// Finds a non-trashed note by UUID, title/alias/file stem (wikilink
    /// rules), or vault-relative path (with or without `.md`).
    pub fn resolve_index(&self, reference: &str) -> Result<usize> {
        let reference = reference.trim();
        if reference.is_empty() {
            bail!("empty note reference");
        }
        let notes = &self.vault.notes;
        if let Ok(id) = Uuid::parse_str(reference)
            && let Some(i) = notes.iter().position(|n| n.frontmatter.id == id)
        {
            return Ok(i);
        }
        if let Some(path) = self.links.resolve(reference)
            && let Some(i) = notes.iter().position(|n| n.path == path)
        {
            return Ok(i);
        }
        let candidate = self.vault.root.join(reference.trim_start_matches("./"));
        let with_md = candidate.with_extension("md");
        if let Some(i) = notes.iter().position(|n| {
            !n.frontmatter.trashed && (n.path == candidate || n.path == with_md)
        }) {
            return Ok(i);
        }
        // Path-like reference without folder: match on the file name.
        if let Some(i) = notes.iter().position(|n| {
            !n.frontmatter.trashed
                && n.path.file_name().is_some_and(|f| Some(f) == candidate.file_name())
        }) {
            return Ok(i);
        }
        bail!("note not found: {reference}")
    }

    pub fn resolve(&self, reference: &str) -> Result<&Note> {
        let i = self.resolve_index(reference)?;
        Ok(&self.vault.notes[i])
    }

    // ─── Notes ───────────────────────────────────────────────────────────

    pub fn list_notes(&self, filter: &NoteFilter) -> Vec<NoteSummary> {
        let folder = filter
            .folder
            .as_deref()
            .map(|f| f.trim().trim_start_matches("./").trim_matches('/').to_string())
            .map(|f| if f == "." { String::new() } else { f });
        let tag = filter.tag.as_deref().map(|t| t.trim_start_matches('#').to_lowercase());
        let mut out: Vec<NoteSummary> = self
            .vault
            .notes
            .iter()
            .filter(|n| filter.include_trashed || !n.frontmatter.trashed)
            .map(|n| self.summary(n))
            .filter(|s| match &folder {
                Some(f) => s.folder == *f || (!f.is_empty() && s.folder.starts_with(&format!("{f}/"))),
                None => true,
            })
            .filter(|s| match &tag {
                Some(t) => s.tags.iter().any(|x| {
                    let x = x.to_lowercase();
                    x == *t || x.starts_with(&format!("{t}/"))
                }),
                None => true,
            })
            .collect();
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    pub fn read_note(&self, reference: &str) -> Result<NoteDetail> {
        let note = self.resolve(reference)?;
        let mut links = Vec::new();
        if !note.is_canvas() {
            for occ in wikilink::parse_wikilinks(&note.body) {
                if !links.contains(&occ.link.target) {
                    links.push(occ.link.target);
                }
            }
        }
        Ok(NoteDetail {
            summary: self.summary(note),
            body: note.body.clone(),
            extra: extra_to_json(&note.frontmatter.extra),
            links,
        })
    }

    /// Updates body and/or tags of an existing note through `Note::save`
    /// (atomic write, unknown frontmatter keys preserved) and refreshes
    /// its index row. Creates the note when missing and
    /// `create_if_missing` is set.
    pub fn write_note(&mut self, req: &WriteNoteRequest) -> Result<WriteNoteResult> {
        let idx = match self.resolve_index(&req.reference) {
            Ok(i) => i,
            Err(_) if req.create_if_missing => {
                let created = self.create_note(&CreateNoteRequest {
                    title: req.reference.trim().to_string(),
                    body: req.body.clone().unwrap_or_default(),
                    folder: req.folder.clone(),
                    tags: req.tags.clone().unwrap_or_default(),
                })?;
                return Ok(WriteNoteResult {
                    note: created,
                    created: true,
                    warnings: Vec::new(),
                });
            }
            Err(e) => return Err(e),
        };
        let mut warnings = Vec::new();
        {
            let note = &mut self.vault.notes[idx];
            if let Some(body) = &req.body {
                note.body = body.clone();
            }
            if let Some(tags) = &req.tags {
                note.frontmatter.tags = normalize_tags(tags);
                if note.frontmatter.unparsed_header.is_some() {
                    warnings.push(
                        "frontmatter block is not valid YAML; it was written back verbatim and the tag change was not persisted"
                            .to_string(),
                    );
                }
            }
            note.save()
                .with_context(|| format!("saving note {}", note.path.display()))?;
        }
        self.reload_note(idx)?;
        let note = &self.vault.notes[idx];
        self.index
            .upsert_note(note)
            .with_context(|| format!("indexing note {}", note.path.display()))?;
        let summary = self.summary(note);
        self.refresh_links();
        Ok(WriteNoteResult {
            note: summary,
            created: false,
            warnings,
        })
    }

    /// Creates `<folder>/<title>.md` (collision-safe name) and indexes it.
    pub fn create_note(&mut self, req: &CreateNoteRequest) -> Result<NoteSummary> {
        let title = req.title.trim();
        if title.is_empty() {
            bail!("a note needs a non-empty title");
        }
        let dir = self.folder_path(req.folder.as_deref())?;
        let mut note = Note::create(&dir, title, &req.body)
            .with_context(|| format!("creating note `{title}` in {}", dir.display()))?;
        if !req.tags.is_empty() {
            note.frontmatter.tags = normalize_tags(&req.tags);
            note.save()?;
        }
        let note = Note::load(&note.path)?;
        self.index
            .upsert_note(&note)
            .with_context(|| format!("indexing note {}", note.path.display()))?;
        let summary = self.summary(&note);
        self.vault.notes.push(note);
        self.refresh_links();
        Ok(summary)
    }

    /// Soft-deletes: moves the file into `.trash/`, flags it, and drops its
    /// cached chunks so retrieval never cites it.
    pub fn trash_note(&mut self, reference: &str) -> Result<TrashResult> {
        let idx = self.resolve_index(reference)?;
        let note = self.vault.notes.remove(idx);
        let previous_path = self.rel(&note.path);
        let id = note.frontmatter.id;
        let title = note.frontmatter.title.clone();
        let trashed = match note.move_to_trash(&self.vault.root) {
            Ok(n) => n,
            Err(e) => {
                // Put it back so the in-memory view stays consistent.
                self.vault.rescan().ok();
                self.refresh_links();
                return Err(e);
            }
        };
        self.index.upsert_note(&trashed)?;
        self.index.delete_chunks_for_doc(id)?;
        let path = self.rel(&trashed.path);
        self.vault.notes.push(trashed);
        self.refresh_links();
        Ok(TrashResult {
            id,
            title,
            previous_path,
            path,
        })
    }

    /// Re-reads one note from disk (after a save) so timestamps and the
    /// stable id match the file.
    fn reload_note(&mut self, idx: usize) -> Result<()> {
        let path = self.vault.notes[idx].path.clone();
        let mut fresh = Note::load(&path)?;
        fresh.frontmatter.trashed |= self.vault.notes[idx].frontmatter.trashed;
        self.vault.notes[idx] = fresh;
        Ok(())
    }

    // ─── Links & graph ───────────────────────────────────────────────────

    /// Notes linking to `reference` by title, file stem or alias.
    pub fn backlinks(&self, reference: &str) -> Result<BacklinksResult> {
        let note = self.resolve(reference)?;
        let keys = wikilink::link_keys_for(note);
        let rows = self.index.backlinks_for_keys(&keys, note.frontmatter.id)?;
        Ok(BacklinksResult {
            target: self.note_ref(note),
            backlinks: rows
                .into_iter()
                .map(|b| BacklinkOut {
                    source_id: b.src_id,
                    source_title: b.src_title,
                    source_path: self.rel(&b.src_path),
                    line: b.line,
                    context: b.context,
                })
                .collect(),
        })
    }

    /// Every `[[wikilink]]` in `reference`'s body, with what it resolves to.
    pub fn outgoing_links(&self, reference: &str) -> Result<LinksResult> {
        let note = self.resolve(reference)?;
        let links = if note.is_canvas() {
            Vec::new()
        } else {
            wikilink::parse_wikilinks(&note.body)
                .into_iter()
                .map(|occ| OutgoingLink {
                    resolved_path: self.links.resolve(&occ.link.target).map(|p| self.rel(p)),
                    target: occ.link.target,
                    heading: occ.link.heading,
                    alias: occ.link.alias,
                    line: occ.line,
                    context: occ.context,
                })
                .collect()
        };
        Ok(LinksResult {
            source: self.note_ref(note),
            links,
        })
    }

    /// The link graph over non-trashed notes, imported PDFs, vault sheets
    /// and ghost targets (no semantic edges: those need every note embedded).
    /// `GraphData::build` matches link targets by title only, so edges
    /// written via an alias or file stem are first resolved through the
    /// wikilink index to the target's title key.
    pub fn graph(&self) -> Result<GraphOut> {
        let mut files = self.index.list_pdf_documents().unwrap_or_default();
        files.extend(crate::sheet::find_sheets(self.root()));
        let mut edges = self.index.link_edges()?;
        for edge in &mut edges {
            if let Some(path) = self.links.resolve(&edge.target)
                && let Some(note) = self.vault.notes.iter().find(|n| n.path == path)
            {
                edge.target_key = wikilink::title_key(&note.frontmatter.title);
            }
        }
        let data = GraphData::build(&self.vault.notes, &files, &edges, &[], GraphOptions::default());
        Ok(GraphOut {
            nodes: data
                .nodes
                .iter()
                .map(|n| GraphNodeOut {
                    key: n.key.clone(),
                    label: n.label.clone(),
                    kind: match n.kind {
                        NodeKind::Note => "note",
                        NodeKind::Canvas => "canvas",
                        NodeKind::Pdf => "pdf",
                        NodeKind::Sheet => "sheet",
                        NodeKind::Ghost => "ghost",
                    },
                    doc_id: n.doc_id,
                    path: n.path.as_deref().map(|p| self.rel(p)),
                    tag: n.tag.clone(),
                    degree: n.degree,
                })
                .collect(),
            edges: data
                .edges
                .iter()
                .map(|e| GraphEdgeOut {
                    a: e.a,
                    b: e.b,
                    kind: match e.kind {
                        EdgeKind::Link => "link",
                        EdgeKind::Semantic => "semantic",
                    },
                    weight: e.weight,
                })
                .collect(),
        })
    }
}

fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tags {
        let t = t.trim().trim_start_matches('#').to_string();
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn service_with_notes() -> (tempfile::TempDir, VaultService) {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("Projects");
        std::fs::create_dir_all(&sub).unwrap();
        let mut a = Note::create(dir.path(), "Alpha", "Lihat [[Beta]] dan [[Gamma|g]].").unwrap();
        a.frontmatter.tags = vec!["work".into(), "work/deep".into()];
        a.frontmatter.aliases = vec!["A1".into()];
        a.save().unwrap();
        Note::create(&sub, "Beta", "Kembali ke [[Alpha]].").unwrap();
        let svc = VaultService::open(dir.path()).unwrap();
        (dir, svc)
    }

    #[test]
    fn list_filters_by_folder_and_nested_tag() {
        let (_d, svc) = service_with_notes();
        let all = svc.list_notes(&NoteFilter::default());
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].path, "Alpha.md");
        assert_eq!(all[1].path, "Projects/Beta.md");
        assert_eq!(all[1].folder, "Projects");

        let root_only = svc.list_notes(&NoteFilter {
            folder: Some(".".into()),
            ..Default::default()
        });
        assert_eq!(root_only.len(), 1);
        let projects = svc.list_notes(&NoteFilter {
            folder: Some("Projects/".into()),
            ..Default::default()
        });
        assert_eq!(projects.len(), 1);
        let tagged = svc.list_notes(&NoteFilter {
            tag: Some("#Work".into()),
            ..Default::default()
        });
        assert_eq!(tagged.len(), 1);
    }

    #[test]
    fn resolves_by_title_alias_path_and_uuid() {
        let (_d, svc) = service_with_notes();
        let by_title = svc.resolve("alpha").unwrap();
        assert_eq!(by_title.frontmatter.title, "Alpha");
        assert_eq!(svc.resolve("A1").unwrap().frontmatter.title, "Alpha");
        assert_eq!(svc.resolve("Projects/Beta.md").unwrap().frontmatter.title, "Beta");
        assert_eq!(svc.resolve("Projects/Beta").unwrap().frontmatter.title, "Beta");
        let id = by_title.frontmatter.id.to_string();
        assert_eq!(svc.resolve(&id).unwrap().frontmatter.title, "Alpha");
        assert!(svc.resolve("Nope").is_err());
    }

    #[test]
    fn read_lists_links_and_extra_keys() {
        let (d, _) = service_with_notes();
        std::fs::write(
            d.path().join("Custom.md"),
            "---\ntitle: Custom\ncustom: 1\nnested:\n  a: [1, 2]\n---\nhello [[Alpha]]",
        )
        .unwrap();
        let svc = VaultService::open(d.path()).unwrap();
        let detail = svc.read_note("Custom").unwrap();
        assert_eq!(detail.body, "hello [[Alpha]]");
        assert_eq!(detail.links, vec!["Alpha"]);
        assert_eq!(detail.extra["custom"], serde_json::json!(1));
        assert_eq!(detail.extra["nested"]["a"], serde_json::json!([1, 2]));
    }

    #[test]
    fn write_keeps_unknown_keys_and_updates_index() {
        let (d, _) = service_with_notes();
        std::fs::write(d.path().join("Custom.md"), "---\ntitle: Custom\ncustom: 1\n---\nold").unwrap();
        let mut svc = VaultService::open(d.path()).unwrap();
        let res = svc
            .write_note(&WriteNoteRequest {
                reference: "Custom".into(),
                body: Some("new [[Beta]]".into()),
                tags: Some(vec!["#x".into(), "x".into()]),
                ..Default::default()
            })
            .unwrap();
        assert!(!res.created);
        assert_eq!(res.note.tags, vec!["x"]);
        let raw = std::fs::read_to_string(d.path().join("Custom.md")).unwrap();
        assert!(raw.contains("custom: 1"), "{raw}");
        assert!(raw.ends_with("new [[Beta]]"));
        let back = svc.backlinks("Beta").unwrap();
        assert!(back.backlinks.iter().any(|b| b.source_path == "Custom.md"));
    }

    #[test]
    fn write_creates_when_asked_and_rejects_escape() {
        let (_d, mut svc) = service_with_notes();
        assert!(svc
            .write_note(&WriteNoteRequest {
                reference: "Missing".into(),
                body: Some("x".into()),
                ..Default::default()
            })
            .is_err());
        let res = svc
            .write_note(&WriteNoteRequest {
                reference: "Missing".into(),
                body: Some("x".into()),
                folder: Some("Inbox/Today".into()),
                create_if_missing: true,
                ..Default::default()
            })
            .unwrap();
        assert!(res.created);
        assert_eq!(res.note.path, "Inbox/Today/Missing.md");
        assert!(svc.resolve("Missing").is_ok());
        let bad = svc.create_note(&CreateNoteRequest {
            title: "Evil".into(),
            folder: Some("../outside".into()),
            ..Default::default()
        });
        assert!(bad.is_err());
        let hidden = svc.create_note(&CreateNoteRequest {
            title: "Evil".into(),
            folder: Some(".mnemonic".into()),
            ..Default::default()
        });
        assert!(hidden.is_err());
    }

    #[test]
    fn trash_moves_file_and_hides_note() {
        let (d, mut svc) = service_with_notes();
        let res = svc.trash_note("Beta").unwrap();
        assert_eq!(res.previous_path, "Projects/Beta.md");
        assert!(res.path.starts_with(".trash/"));
        assert!(d.path().join(&res.path).exists());
        assert!(svc.resolve("Beta").is_err());
        assert_eq!(svc.list_notes(&NoteFilter::default()).len(), 1);
        assert_eq!(
            svc.list_notes(&NoteFilter {
                include_trashed: true,
                ..Default::default()
            })
            .len(),
            2
        );
        // Ghost now: the link from Alpha no longer resolves.
        let links = svc.outgoing_links("Alpha").unwrap();
        let beta = links.links.iter().find(|l| l.target == "Beta").unwrap();
        assert!(beta.resolved_path.is_none());
    }

    #[test]
    fn backlinks_links_and_graph_agree() {
        let (_d, svc) = service_with_notes();
        let back = svc.backlinks("Alpha").unwrap();
        assert_eq!(back.backlinks.len(), 1);
        assert_eq!(back.backlinks[0].source_title, "Beta");
        assert_eq!(back.backlinks[0].source_path, "Projects/Beta.md");

        let links = svc.outgoing_links("Alpha").unwrap();
        assert_eq!(links.links.len(), 2);
        assert_eq!(links.links[0].resolved_path.as_deref(), Some("Projects/Beta.md"));
        assert_eq!(links.links[1].alias.as_deref(), Some("g"));
        assert!(links.links[1].resolved_path.is_none());

        let g = svc.graph().unwrap();
        assert_eq!(g.nodes.iter().filter(|n| n.kind == "note").count(), 2);
        assert_eq!(g.nodes.iter().filter(|n| n.kind == "ghost").count(), 1);
        assert!(g.edges.iter().all(|e| e.kind == "link"));
        assert_eq!(g.edges.len(), 2);
    }
}
