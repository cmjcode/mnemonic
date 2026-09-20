//! Retrieval half of `VaultService` (§Fase 2): incremental chunk indexing,
//! hybrid search and the RAG `ask`, mirroring what `app::MnemonicApp`
//! does on its worker threads but synchronously and without egui. The
//! embedding model and the LLM are loaded lazily on first use; a load
//! failure degrades `search`/`reindex` to keyword-only and is reported in
//! the result rather than failing the call. Callers: `api::mcp`,
//! `src/bin/mnemonic-cli.rs`.

use std::collections::HashSet;

use anyhow::{Context, Result};

use super::service::VaultService;
use super::types::*;
use crate::core::search::{HybridOptions, MatchKind, SearchHit, hybrid_rank};
use crate::core::storage::{HIGHLIGHT_END, HIGHLIGHT_START};
use crate::core::{EMBEDDING_DIM, EMBEDDING_MODEL_ID, ingestion};
#[cfg(feature = "semantic")]
use crate::core::EmbeddingEngine;
use crate::llm;
#[cfg(feature = "semantic")]
use crate::llm::{CandleEngine, Generator};

/// KNN / FTS candidate pool fed to RRF (same as `app::RETRIEVAL_CANDIDATES`).
const RETRIEVAL_CANDIDATES: usize = 30;
/// Larger pool when a folder/tag filter drops hits after ranking, so a
/// narrow filter still finds its matches.
const FILTERED_CANDIDATES: usize = 300;
/// Chunks retrieved for the RAG prompt (same as `app::CHAT_TOP_K`).
#[cfg_attr(not(feature = "semantic"), allow(dead_code))]
const ASK_TOP_K: usize = 5;
/// Prefix of a `document_hashes` entry written by a keyword-only run: the
/// note is chunked and FTS-indexed but its vectors are zeros, so a later
/// semantic run must redo it while a keyword-only run can skip it.
pub(super) const KEYWORD_ONLY_HASH_PREFIX: &str = "kw:";

/// A model that is loaded on first use and, once it failed, is not
/// retried for the lifetime of the service (loading is slow).
#[derive(Default)]
#[cfg_attr(not(feature = "semantic"), allow(dead_code))]
pub(super) enum Lazy<T> {
    #[default]
    Unloaded,
    Ready(T),
    Failed(String),
}

#[cfg_attr(not(feature = "semantic"), allow(dead_code))]
impl<T> Lazy<T> {
    fn get_or_load(&mut self, load: impl FnOnce() -> Result<T>) -> Result<&mut T, String> {
        if matches!(self, Lazy::Unloaded) {
            *self = match load() {
                Ok(t) => Lazy::Ready(t),
                Err(e) => Lazy::Failed(format!("{e:#}")),
            };
        }
        match self {
            Lazy::Ready(t) => Ok(t),
            Lazy::Failed(msg) => Err(msg.clone()),
            Lazy::Unloaded => unreachable!("just loaded"),
        }
    }
}

/// Tanpa fitur `semantic` tidak ada model yang bisa dimuat; tipe kosong
/// ini menjaga bentuk `VaultService` tetap sama sehingga jalur leksikal
/// (FTS SQLite) tidak perlu percabangan di mana-mana.
#[cfg(not(feature = "semantic"))]
pub(super) enum NoModel {}

#[cfg(feature = "semantic")]
pub(super) type LazyEmbedder = Lazy<EmbeddingEngine>;
#[cfg(not(feature = "semantic"))]
pub(super) type LazyEmbedder = Lazy<NoModel>;
#[cfg(feature = "semantic")]
pub(super) type LazyGenerator = Lazy<CandleEngine>;
#[cfg(not(feature = "semantic"))]
pub(super) type LazyGenerator = Lazy<NoModel>;

/// Pesan seragam saat build tidak memuat model di perangkat.
#[cfg(not(feature = "semantic"))]
pub(super) const NO_SEMANTIC: &str =
    "build ini tanpa fitur `semantic`: pencarian vektor dan `ask` tidak tersedia, pakai pencarian kata kunci";

fn is_sheet(path: &std::path::Path) -> bool {
    crate::sheet::is_sheet_path(path)
}

pub(crate) fn kind_str(kind: MatchKind) -> &'static str {
    match kind {
        MatchKind::Semantic => "semantic",
        MatchKind::Keyword => "keyword",
        MatchKind::Both => "both",
    }
}

pub(crate) fn strip_highlights(s: &str) -> String {
    s.chars()
        .filter(|c| *c != HIGHLIGHT_START && *c != HIGHLIGHT_END)
        .collect()
}

/// Folder/tag restriction of a retrieval (§3.10.4); empty = everything.
#[derive(Debug, Clone, Default)]
pub(crate) struct HitFilter {
    folder: Option<String>,
    tag: Option<String>,
}

impl HitFilter {
    /// `folder` is vault-relative (`.`/`""` = root only); `tag` may carry `#`.
    pub(crate) fn new(folder: Option<&str>, tag: Option<&str>) -> HitFilter {
        HitFilter {
            folder: folder
                .map(|f| f.trim().trim_start_matches("./").trim_matches('/'))
                .map(|f| if f == "." { String::new() } else { f.to_string() }),
            tag: tag
                .map(|t| t.trim().trim_start_matches('#').to_string())
                .filter(|t| !t.is_empty()),
        }
    }

    fn is_empty(&self) -> bool {
        self.folder.is_none() && self.tag.is_none()
    }
}

impl VaultService {
    #[cfg(feature = "semantic")]
    fn embedder(&mut self) -> Result<&mut EmbeddingEngine, String> {
        self.embedder.get_or_load(|| {
            log::info!("api: loading embedding model {EMBEDDING_MODEL_ID}");
            EmbeddingEngine::new()
        })
    }

    /// Tanpa model di perangkat: selalu gagal, dan pemanggilnya jatuh ke
    /// jalur kata kunci sambil menambahkan peringatan.
    #[cfg(not(feature = "semantic"))]
    fn embedder(&mut self) -> Result<&mut NoModel, String> {
        Err(NO_SEMANTIC.to_string())
    }

    // ─── Reindex ─────────────────────────────────────────────────────────

    /// Rescans the vault, rewrites `notes_index`/`links`, then chunks (and
    /// embeds, unless keyword-only) every non-trashed note whose content
    /// hash changed, and prunes chunks of notes that are gone.
    pub fn reindex(&mut self, opts: ReindexOptions) -> Result<ReindexReport> {
        self.vault.rescan().context("rescanning vault")?;
        self.refresh_links();
        self.index
            .rebuild(&self.vault.notes)
            .context("rebuilding notes index")?;

        let mut report = ReindexReport {
            notes_indexed: self.vault.notes.len(),
            chunked: 0,
            skipped: 0,
            pruned: 0,
            semantic: false,
            sheets_indexed: 0,
            sheets_chunked: 0,
            failed: Vec::new(),
            warnings: Vec::new(),
        };

        let mut semantic = !opts.keyword_only;
        if semantic && let Err(msg) = self.embedder() {
            report
                .warnings
                .push(format!("embedding model unavailable, indexing keywords only: {msg}"));
            semantic = false;
        }
        report.semantic = semantic;

        // Keep vector tables aligned with the app's model; a model change
        // drops every chunk, so everything must be redone.
        let reset = self
            .index
            .ensure_embedding_model(EMBEDDING_MODEL_ID, EMBEDDING_DIM)
            .context("preparing vector tables")?;
        let full = opts.full || reset;

        let live: Vec<usize> = (0..self.vault.notes.len())
            .filter(|&i| !self.vault.notes[i].frontmatter.trashed)
            .collect();
        for i in live.iter().copied() {
            let note = &self.vault.notes[i];
            let hash = ingestion::note_content_hash(note);
            let stored = self.index.document_hash(note.frontmatter.id)?;
            let up_to_date = match stored.as_deref() {
                Some(s) if s == hash => true,
                // A keyword-only pass never overwrites real vectors that are
                // still current, and can skip its own earlier work.
                Some(s) if !semantic => s == format!("{KEYWORD_ONLY_HASH_PREFIX}{hash}"),
                _ => false,
            };
            let has_real_vectors = stored.as_deref() == Some(hash.as_str());
            if up_to_date && (!full || (!semantic && has_real_vectors)) {
                report.skipped += 1;
                continue;
            }
            let path = self.rel(&note.path);
            match self.chunk_and_store(i, semantic, &hash) {
                Ok(()) => report.chunked += 1,
                Err(e) => {
                    log::warn!("api: indexing {path} failed: {e:#}");
                    report.failed.push(ReindexFailure {
                        path,
                        error: format!("{e:#}"),
                    });
                }
            }
        }

        let live_ids: HashSet<_> = live
            .iter()
            .map(|&i| self.vault.notes[i].frontmatter.id)
            .collect();
        report.pruned = self
            .index
            .prune_notes_not_in(&live_ids)
            .context("pruning stale chunks")?;
        self.reindex_sheets(semantic, full, &mut report)?;
        Ok(report)
    }

    /// Chunk vectors for `inputs`: real embeddings when `semantic`, zero
    /// vectors for a keyword-only pass.
    pub(super) fn vectors_for(&mut self, inputs: &[String], semantic: bool) -> Result<Vec<Vec<f32>>> {
        // Tanpa model di perangkat, setiap pass adalah pass kata kunci.
        #[cfg(not(feature = "semantic"))]
        let semantic = {
            let _ = semantic;
            false
        };
        if !semantic || inputs.is_empty() {
            return Ok(vec![vec![0.0; EMBEDDING_DIM]; inputs.len()]);
        }
        #[cfg(not(feature = "semantic"))]
        unreachable!("tanpa fitur semantic, `semantic` selalu false");
        #[cfg(feature = "semantic")]
        let embedder = self.embedder().map_err(anyhow::Error::msg)?;
        #[cfg(feature = "semantic")]
        {
            let v = embedder.embed(inputs).context("embedding chunks")?;
            anyhow::ensure!(
                v.len() == inputs.len(),
                "embedder returned {} vectors for {} chunks",
                v.len(),
                inputs.len()
            );
            Ok(v)
        }
    }

    /// Re-chunks note `i` after an agent edit so `search`/`recall` see it
    /// at once: with real vectors when the embedding model is already
    /// loaded, otherwise keyword-only (marked so the next semantic run, or
    /// the app's indexer, embeds it).
    pub(crate) fn refresh_chunks(&mut self, i: usize) -> Result<()> {
        if self.vault.notes[i].frontmatter.trashed {
            return Ok(());
        }
        let semantic = matches!(self.embedder, Lazy::Ready(_));
        let hash = ingestion::note_content_hash(&self.vault.notes[i]);
        self.chunk_and_store(i, semantic, &hash)
    }

    fn chunk_and_store(&mut self, i: usize, semantic: bool, hash: &str) -> Result<()> {
        let note = &self.vault.notes[i];
        let id = note.frontmatter.id;
        let title = note.frontmatter.title.clone();
        let chunks = ingestion::chunk_note(note);
        let text = ingestion::note_index_text(note);
        let inputs: Vec<String> = chunks
            .iter()
            .map(|c| {
                let heading = ingestion::heading_at(&text, c.char_offset);
                ingestion::embedding_input(&title, heading.as_deref(), &c.text_content)
            })
            .collect();
        let vectors = self.vectors_for(&inputs, semantic)?;
        let pairs: Vec<_> = chunks.into_iter().zip(vectors).collect();
        self.index
            .replace_chunks(id, "note", &title, &pairs)
            .context("storing chunks")?;
        let recorded = if semantic {
            hash.to_string()
        } else {
            format!("{KEYWORD_ONLY_HASH_PREFIX}{hash}")
        };
        self.index.set_document_hash(id, &recorded)?;
        Ok(())
    }

    // ─── Search ──────────────────────────────────────────────────────────

    /// Hybrid ranking over the cached chunks, restricted to `filter`.
    /// Returns the hits plus whether the vector route ran and any
    /// degradation warnings.
    pub(crate) fn retrieve(
        &mut self,
        query: &str,
        k: usize,
        semantic: bool,
        one_per_doc: bool,
        filter: &HitFilter,
    ) -> Result<(Vec<SearchHit>, bool, Vec<String>)> {
        let pool = if filter.is_empty() { RETRIEVAL_CANDIDATES } else { FILTERED_CANDIDATES };
        let mut warnings = Vec::new();
        let mut used_semantic = false;
        let mut vector_hits = Vec::new();
        if semantic {
            let embedding: Result<Vec<f32>, String> = match self.embedder() {
                #[cfg(feature = "semantic")]
                Ok(embedder) => embedder
                    .embed_query(query)
                    .map_err(|e| format!("query embedding failed: {e:#}")),
                #[cfg(not(feature = "semantic"))]
                Ok(_) => unreachable!("tanpa fitur semantic tidak ada model"),
                Err(msg) => Err(format!("embedding model unavailable, keyword-only: {msg}")),
            };
            match embedding.and_then(|e| {
                self.index
                    .knn_chunks(&e, pool)
                    .map_err(|e| format!("vector search failed: {e:#}"))
            }) {
                Ok(hits) => {
                    used_semantic = true;
                    vector_hits = hits;
                }
                Err(w) => warnings.push(w),
            }
        }
        let keyword = self
            .index
            .keyword_chunks(query, pool)
            .context("keyword search")?;
        let mut hits = hybrid_rank(
            vector_hits,
            keyword,
            HybridOptions {
                k: if filter.is_empty() { k } else { pool },
                min_similarity: llm::SIMILARITY_THRESHOLD,
                one_per_doc,
            },
        );
        if !filter.is_empty() {
            hits.retain(|h| self.hit_passes(h, filter));
            hits.truncate(k);
        }
        Ok((hits, used_semantic, warnings))
    }

    /// Whether a hit's document lies in `filter.folder` and carries
    /// `filter.tag` (frontmatter or inline; PDFs/sheets have no tags).
    fn hit_passes(&self, hit: &SearchHit, filter: &HitFilter) -> bool {
        if let Some(folder) = filter.folder.as_deref() {
            let rel = self.rel(&hit.chunk.file_path);
            let dir = rel.rfind('/').map_or("", |i| &rel[..i]);
            let inside = dir == folder || (!folder.is_empty() && dir.starts_with(&format!("{folder}/")));
            if !inside {
                return false;
            }
        }
        if let Some(tag) = filter.tag.as_deref() {
            let note = self
                .vault
                .notes
                .iter()
                .find(|n| n.frontmatter.id == hit.chunk.doc_id || n.path == hit.chunk.file_path);
            let tagged = note.is_some_and(|n| n.effective_tags().iter().any(|t| crate::notes::tags::matches_tag(t, tag)));
            if !tagged {
                return false;
            }
        }
        true
    }

    /// `(heading path, 1-based line)` of a note chunk; `(None, None)` for
    /// PDFs, sheets and canvas notes.
    pub(crate) fn chunk_position(&self, chunk: &ingestion::DocumentChunk) -> (Option<String>, Option<usize>) {
        match self.vault.notes.iter().find(|n| n.frontmatter.id == chunk.doc_id) {
            Some(n) if !n.is_canvas() => {
                let (line, heading) = super::memory::locate(&n.body, chunk.char_offset);
                (heading.map(|h| h.path_string()), Some(line))
            }
            _ => (None, None),
        }
    }

    /// Title of the note (or file name of the PDF) a chunk came from.
    pub(crate) fn doc_title(&self, chunk: &ingestion::DocumentChunk) -> String {
        self.vault
            .notes
            .iter()
            .find(|n| n.frontmatter.id == chunk.doc_id || n.path == chunk.file_path)
            .map(|n| n.frontmatter.title.clone())
            .unwrap_or_else(|| {
                chunk
                    .file_path
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default()
            })
    }

    pub fn search(&mut self, req: &SearchRequest) -> Result<SearchResult> {
        let query = req.query.trim().to_string();
        if query.is_empty() {
            return Ok(SearchResult {
                query,
                semantic: false,
                hits: Vec::new(),
                warnings: vec!["empty query".to_string()],
            });
        }
        let k = req.k.max(1);
        let filter = HitFilter::new(req.folder.as_deref(), req.tag.as_deref());
        let (hits, semantic, warnings) = self.retrieve(&query, k, req.semantic, true, &filter)?;
        let hits = hits
            .into_iter()
            .map(|h| {
                let (heading, line) = self.chunk_position(&h.chunk);
                SearchHitOut {
                doc_id: h.chunk.doc_id,
                path: self.rel(&h.chunk.file_path),
                title: self.doc_title(&h.chunk),
                page: h.chunk.page_num.filter(|_| !is_sheet(&h.chunk.file_path)),
                row: h.chunk.page_num.filter(|_| is_sheet(&h.chunk.file_path)),
                char_offset: h.chunk.char_offset,
                score: h.score.filter(|s| s.is_finite()),
                kind: kind_str(h.kind),
                snippet: h.snippet.as_deref().map(strip_highlights),
                text: h.chunk.text_content,
                heading,
                line,
            }})
            .collect();
        Ok(SearchResult {
            query,
            semantic,
            hits,
            warnings,
        })
    }

    // ─── Ask (RAG) ───────────────────────────────────────────────────────

    /// Retrieves context for `question`, builds the grounded prompt and
    /// runs the local LLM to completion. Loads the model on first call.
    pub fn ask(&mut self, req: &AskRequest) -> Result<AskResult> {
        #[cfg(not(feature = "semantic"))]
        {
            let _ = req;
            anyhow::bail!(NO_SEMANTIC);
        }
        #[cfg(feature = "semantic")]
        {
        let question = req.question.trim().to_string();
        anyhow::ensure!(!question.is_empty(), "empty question");
        let (hits, semantic, warnings) = self.retrieve(&question, ASK_TOP_K, true, false, &HitFilter::default())?;
        let context = llm::select_context(&hits, llm::SIMILARITY_THRESHOLD);
        let citations = context
            .iter()
            .map(|c| Citation {
                doc_id: c.doc_id,
                path: self.rel(&c.file_path),
                title: self.doc_title(c),
                page: c.page_num.filter(|_| !is_sheet(&c.file_path)),
                row: c.page_num.filter(|_| is_sheet(&c.file_path)),
            })
            .collect();
        let prompt = llm::build_rag_prompt(&context, &question);
        let max_tokens = if req.max_tokens == 0 {
            llm::DEFAULT_MAX_TOKENS
        } else {
            req.max_tokens
        };
        let generator = self
            .generator
            .get_or_load(|| {
                log::info!("api: loading LLM");
                CandleEngine::new()
            })
            .map_err(anyhow::Error::msg)?;
        let mut answer = String::new();
        generator
            .generate(&prompt, max_tokens, &mut |tok| answer.push_str(tok))
            .context("generating answer")?;
        Ok(AskResult {
            question,
            answer: answer.trim().to_string(),
            citations,
            semantic,
            warnings,
        })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::Note;
    use tempfile::tempdir;

    #[test]
    fn keyword_only_reindex_then_search_finds_note() {
        let dir = tempdir().unwrap();
        Note::create(dir.path(), "Resep", "Nasi goreng butuh dua butir telur dan kecap.").unwrap();
        Note::create(dir.path(), "Lain", "Tentang sepeda dan cuaca.").unwrap();
        let mut svc = VaultService::open(dir.path()).unwrap();

        let report = svc
            .reindex(ReindexOptions {
                full: false,
                keyword_only: true,
            })
            .unwrap();
        assert!(!report.semantic);
        assert_eq!(report.chunked, 2);
        assert!(report.failed.is_empty(), "{:?}", report.failed);

        // Second run skips unchanged notes.
        let again = svc
            .reindex(ReindexOptions {
                full: false,
                keyword_only: true,
            })
            .unwrap();
        assert_eq!(again.chunked, 0);
        assert_eq!(again.skipped, 2);

        let res = svc
            .search(&SearchRequest {
                query: "telur".into(),
                k: 5,
                semantic: false,
                ..Default::default()
            })
            .unwrap();
        assert!(!res.semantic);
        assert_eq!(res.hits.len(), 1);
        assert_eq!(res.hits[0].title, "Resep");
        assert_eq!(res.hits[0].path, "Resep.md");
        assert_eq!(res.hits[0].kind, "keyword");
        let snippet = res.hits[0].snippet.as_deref().unwrap();
        assert!(!snippet.contains(HIGHLIGHT_START));
        assert!(snippet.contains("telur"));
    }

    #[test]
    fn trashed_and_deleted_notes_are_pruned() {
        let dir = tempdir().unwrap();
        Note::create(dir.path(), "Hapus", "kata unik zebra").unwrap();
        let mut svc = VaultService::open(dir.path()).unwrap();
        svc.reindex(ReindexOptions {
            full: false,
            keyword_only: true,
        })
        .unwrap();
        assert_eq!(
            svc.search(&SearchRequest {
                query: "zebra".into(),
                k: 5,
                semantic: false,
                ..Default::default()
            })
            .unwrap()
            .hits
            .len(),
            1
        );
        std::fs::remove_file(dir.path().join("Hapus.md")).unwrap();
        let report = svc
            .reindex(ReindexOptions {
                full: false,
                keyword_only: true,
            })
            .unwrap();
        assert_eq!(report.pruned, 1);
        assert!(svc
            .search(&SearchRequest {
                query: "zebra".into(),
                k: 5,
                semantic: false,
                ..Default::default()
            })
            .unwrap()
            .hits
            .is_empty());
    }

    #[test]
    fn full_keyword_run_rechunks_changed_notes() {
        let dir = tempdir().unwrap();
        let mut note = Note::create(dir.path(), "Ubah", "versi satu").unwrap();
        let mut svc = VaultService::open(dir.path()).unwrap();
        svc.reindex(ReindexOptions {
            full: false,
            keyword_only: true,
        })
        .unwrap();
        note.body = "versi dua kucing".into();
        note.save().unwrap();
        let report = svc
            .reindex(ReindexOptions {
                full: true,
                keyword_only: true,
            })
            .unwrap();
        assert_eq!(report.chunked, 1);
        let res = svc
            .search(&SearchRequest {
                query: "kucing".into(),
                k: 5,
                semantic: false,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(res.hits.len(), 1);
    }

    /// Needs the FastEmbed model on disk (downloads ~120 MB on first run).
    #[test]
    #[ignore]
    fn semantic_search_ranks_related_note_first() {
        let dir = tempdir().unwrap();
        Note::create(dir.path(), "Resep", "Cara memasak nasi goreng dengan telur.").unwrap();
        Note::create(dir.path(), "Sepeda", "Merawat rantai sepeda gunung.").unwrap();
        let mut svc = VaultService::open(dir.path()).unwrap();
        let report = svc.reindex(ReindexOptions::default()).unwrap();
        assert!(report.semantic, "{:?}", report.warnings);
        let res = svc
            .search(&SearchRequest {
                query: "resep makanan".into(),
                k: 2,
                semantic: true,
                ..Default::default()
            })
            .unwrap();
        assert!(res.semantic);
        assert_eq!(res.hits[0].title, "Resep");
    }
}
