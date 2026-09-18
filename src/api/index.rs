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
use crate::core::{EMBEDDING_DIM, EMBEDDING_MODEL_ID, EmbeddingEngine, ingestion};
use crate::llm::{self, CandleEngine, Generator};

/// KNN / FTS candidate pool fed to RRF (same as `app::RETRIEVAL_CANDIDATES`).
const RETRIEVAL_CANDIDATES: usize = 30;
/// Chunks retrieved for the RAG prompt (same as `app::CHAT_TOP_K`).
const ASK_TOP_K: usize = 5;
/// Prefix of a `document_hashes` entry written by a keyword-only run: the
/// note is chunked and FTS-indexed but its vectors are zeros, so a later
/// semantic run must redo it while a keyword-only run can skip it.
pub(super) const KEYWORD_ONLY_HASH_PREFIX: &str = "kw:";

/// A model that is loaded on first use and, once it failed, is not
/// retried for the lifetime of the service (loading is slow).
#[derive(Default)]
pub(super) enum Lazy<T> {
    #[default]
    Unloaded,
    Ready(T),
    Failed(String),
}

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

pub(super) type LazyEmbedder = Lazy<EmbeddingEngine>;
pub(super) type LazyGenerator = Lazy<CandleEngine>;

fn is_sheet(path: &std::path::Path) -> bool {
    crate::sheet::is_sheet_path(path)
}

fn kind_str(kind: MatchKind) -> &'static str {
    match kind {
        MatchKind::Semantic => "semantic",
        MatchKind::Keyword => "keyword",
        MatchKind::Both => "both",
    }
}

fn strip_highlights(s: &str) -> String {
    s.chars()
        .filter(|c| *c != HIGHLIGHT_START && *c != HIGHLIGHT_END)
        .collect()
}

impl VaultService {
    fn embedder(&mut self) -> Result<&mut EmbeddingEngine, String> {
        self.embedder.get_or_load(|| {
            log::info!("api: loading embedding model {EMBEDDING_MODEL_ID}");
            EmbeddingEngine::new()
        })
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
        if !semantic || inputs.is_empty() {
            return Ok(vec![vec![0.0; EMBEDDING_DIM]; inputs.len()]);
        }
        let embedder = self.embedder().map_err(anyhow::Error::msg)?;
        let v = embedder.embed(inputs).context("embedding chunks")?;
        anyhow::ensure!(
            v.len() == inputs.len(),
            "embedder returned {} vectors for {} chunks",
            v.len(),
            inputs.len()
        );
        Ok(v)
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

    /// Hybrid ranking over the cached chunks. Returns the hits plus whether
    /// the vector route ran and any degradation warnings.
    fn retrieve(
        &mut self,
        query: &str,
        k: usize,
        semantic: bool,
        one_per_doc: bool,
    ) -> Result<(Vec<SearchHit>, bool, Vec<String>)> {
        let mut warnings = Vec::new();
        let mut used_semantic = false;
        let mut vector_hits = Vec::new();
        if semantic {
            let embedding = match self.embedder() {
                Ok(embedder) => embedder
                    .embed_query(query)
                    .map_err(|e| format!("query embedding failed: {e:#}")),
                Err(msg) => Err(format!("embedding model unavailable, keyword-only: {msg}")),
            };
            match embedding.and_then(|e| {
                self.index
                    .knn_chunks(&e, RETRIEVAL_CANDIDATES)
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
            .keyword_chunks(query, RETRIEVAL_CANDIDATES)
            .context("keyword search")?;
        let hits = hybrid_rank(
            vector_hits,
            keyword,
            HybridOptions {
                k,
                min_similarity: llm::SIMILARITY_THRESHOLD,
                one_per_doc,
            },
        );
        Ok((hits, used_semantic, warnings))
    }

    /// Title of the note (or file name of the PDF) a chunk came from.
    fn doc_title(&self, chunk: &ingestion::DocumentChunk) -> String {
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
        let (hits, semantic, warnings) = self.retrieve(&query, k, req.semantic, true)?;
        let hits = hits
            .into_iter()
            .map(|h| SearchHitOut {
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
            })
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
        let question = req.question.trim().to_string();
        anyhow::ensure!(!question.is_empty(), "empty question");
        let (hits, semantic, warnings) = self.retrieve(&question, ASK_TOP_K, true, false)?;
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
                semantic: false
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
                semantic: false
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
            })
            .unwrap();
        assert!(res.semantic);
        assert_eq!(res.hits[0].title, "Resep");
    }
}
