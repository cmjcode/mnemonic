//! Background indexing worker (§5: "Background task pool"): runs chunking
//! + embedding off the UI thread so opening a vault or editing a large
//! document doesn't stall `egui`'s frame loop. Shaped like
//! `notes::watcher::VaultWatcher` — spawn a thread, feed it jobs, poll a
//! channel once per frame — rather than pulling in `tokio`/`rayon` for
//! what is, per document, a single call into `EmbeddingEngine::embed`
//! (which already batches all of a document's chunks into one model
//! call). This module never touches SQLite itself: the caller owns the
//! `core::storage::IndexStore` and writes each `IndexResult` into
//! `IndexStore::replace_chunks` on the UI thread, matching the existing
//! split where `app.rs` (not the watcher) does the actual index rebuild.
//! Also handles ad-hoc *query* embedding (§Fase 7's search tab + RAG
//! chat) over a second channel: reusing this worker's already-lazily-
//! loaded `Embedder` for a one-off query avoids paying a second model
//! load for a role-specific worker. Callers: `app.rs`.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result};
use uuid::Uuid;

use super::embedding::EmbeddingEngine;
use super::ingestion::{self, DocumentChunk};
use crate::notes::Note;

/// What kind of source a chunk batch came from — mirrors the `doc_type`
/// column `IndexResult`s are ultimately written under in
/// `core::storage`'s `document_chunks` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocType {
    Note,
    Pdf,
}

impl DocType {
    pub fn as_str(self) -> &'static str {
        match self {
            DocType::Note => "note",
            DocType::Pdf => "pdf",
        }
    }
}

/// A unit of work submitted to the indexing worker.
pub enum IndexJob {
    Note(Note),
    Pdf(PathBuf),
    /// A search/chat query string to embed (§Fase 7) — carries its own id
    /// since, unlike notes/PDFs, a query has no natural `doc_id` to key
    /// its result on.
    Query { id: Uuid, text: String },
}

/// Chunks + embeddings for one document, ready for
/// `core::storage::IndexStore::replace_chunks`.
pub struct IndexResult {
    pub doc_id: Uuid,
    pub doc_type: DocType,
    pub source_path: PathBuf,
    pub chunks: Vec<(DocumentChunk, Vec<f32>)>,
}

/// Anything that can turn chunk texts into embedding vectors. Lets tests
/// inject a fake, instant embedder instead of downloading/loading the
/// real ~80 MB FastEmbed model — the model itself only needs to satisfy
/// this trait (impl below) to work as-is with `IndexingWorker::spawn`.
pub trait Embedder: Send {
    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

impl Embedder for EmbeddingEngine {
    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        EmbeddingEngine::embed(self, texts)
    }
}

/// A dedicated background thread that chunks + embeds submitted
/// notes/PDFs and reports results back over a channel. One worker thread
/// is enough here rather than a wider pool: `EmbeddingEngine::embed`
/// already batches an entire document's chunks into a single model call,
/// so the bottleneck is the model's own inference, not job scheduling —
/// more threads would just contend over the same ONNX runtime instance.
pub struct IndexingWorker {
    job_tx: Sender<IndexJob>,
    result_rx: Receiver<Result<IndexResult>>,
    query_result_rx: Receiver<(Uuid, Result<Vec<f32>>)>,
    _handle: JoinHandle<()>,
}

impl IndexingWorker {
    /// Spawns the worker backed by the real FastEmbed model. Loading is
    /// deferred to the first submitted job, so `spawn` itself never blocks
    /// on the model download/load.
    pub fn spawn() -> IndexingWorker {
        Self::spawn_with(|| EmbeddingEngine::new().map(|e| Box::new(e) as Box<dyn Embedder>))
    }

    /// Spawns the worker with a caller-supplied embedder factory.
    /// Production uses `spawn()`; tests inject a fake `Embedder` to stay
    /// offline and instant.
    pub fn spawn_with<F>(make_embedder: F) -> IndexingWorker
    where
        F: FnOnce() -> Result<Box<dyn Embedder>> + Send + 'static,
    {
        let (job_tx, job_rx) = channel::<IndexJob>();
        let (result_tx, result_rx) = channel::<Result<IndexResult>>();
        let (query_result_tx, query_result_rx) = channel::<(Uuid, Result<Vec<f32>>)>();

        let handle = thread::spawn(move || run(job_rx, result_tx, query_result_tx, make_embedder));

        IndexingWorker {
            job_tx,
            result_rx,
            query_result_rx,
            _handle: handle,
        }
    }

    /// Queues a note for (re)indexing. Non-blocking; silently dropped if
    /// the worker thread has already exited (e.g. during shutdown).
    pub fn submit_note(&self, note: Note) {
        let _ = self.job_tx.send(IndexJob::Note(note));
    }

    /// Queues a PDF file for (re)indexing. Non-blocking.
    pub fn submit_pdf(&self, path: PathBuf) {
        let _ = self.job_tx.send(IndexJob::Pdf(path));
    }

    /// Queues a search/chat query string for embedding (§Fase 7),
    /// returning the request id its result will be tagged with. Non-
    /// blocking; the result — or nothing, if the worker thread has
    /// already exited — arrives via `poll_query_results`.
    pub fn submit_query(&self, text: String) -> Uuid {
        let id = Uuid::new_v4();
        let _ = self.job_tx.send(IndexJob::Query { id, text });
        id
    }

    /// Drains all results currently available without blocking. Call once
    /// per UI frame (same shape as `VaultWatcher::poll_rescan_needed`) and
    /// write each `Ok` result into `IndexStore::replace_chunks`; log `Err`
    /// results (a bad PDF, a failed model load, ...).
    pub fn poll_results(&self) -> Vec<Result<IndexResult>> {
        let mut out = Vec::new();
        loop {
            match self.result_rx.try_recv() {
                Ok(result) => out.push(result),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        out
    }

    /// Drains all query-embedding results currently available without
    /// blocking. Call once per UI frame and match each id against the
    /// request id returned by `submit_query` to tell concurrent search vs.
    /// chat requests apart.
    pub fn poll_query_results(&self) -> Vec<(Uuid, Result<Vec<f32>>)> {
        let mut out = Vec::new();
        loop {
            match self.query_result_rx.try_recv() {
                Ok(result) => out.push(result),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        out
    }
}

/// Lazily-constructed embedder state inside the worker thread: built once
/// from the first job (so idle time never pays the model-load cost), and
/// remembered as `Failed` afterward so a broken model doesn't retry the
/// same slow failure on every subsequent job.
enum EmbedderState {
    Pending(Box<dyn FnOnce() -> Result<Box<dyn Embedder>> + Send>),
    Ready(Box<dyn Embedder>),
    Failed,
}

fn run<F>(
    job_rx: Receiver<IndexJob>,
    result_tx: Sender<Result<IndexResult>>,
    query_result_tx: Sender<(Uuid, Result<Vec<f32>>)>,
    make_embedder: F,
) where
    F: FnOnce() -> Result<Box<dyn Embedder>> + Send + 'static,
{
    let mut state = EmbedderState::Pending(Box::new(make_embedder));

    while let Ok(job) = job_rx.recv() {
        if matches!(state, EmbedderState::Pending(_)) {
            let EmbedderState::Pending(build) =
                std::mem::replace(&mut state, EmbedderState::Failed)
            else {
                unreachable!("just matched Pending above");
            };
            match build() {
                Ok(embedder) => state = EmbedderState::Ready(embedder),
                Err(e) => {
                    let msg = format!("{e:#}");
                    match job {
                        IndexJob::Query { id, .. } => {
                            let _ = query_result_tx
                                .send((id, Err(anyhow::anyhow!("loading embedding model: {msg}"))));
                        }
                        _ => {
                            let _ = result_tx.send(Err(e.context("loading embedding model")));
                        }
                    }
                    continue;
                }
            }
        }

        let embedder = match &mut state {
            EmbedderState::Ready(e) => e.as_mut(),
            EmbedderState::Failed => {
                match job {
                    IndexJob::Query { id, .. } => {
                        let _ = query_result_tx.send((
                            id,
                            Err(anyhow::anyhow!(
                                "embedding model failed to load earlier; skipping job"
                            )),
                        ));
                    }
                    _ => {
                        let _ = result_tx.send(Err(anyhow::anyhow!(
                            "embedding model failed to load earlier; skipping job"
                        )));
                    }
                }
                continue;
            }
            EmbedderState::Pending(_) => unreachable!("resolved above"),
        };

        match job {
            IndexJob::Query { id, text } => {
                let result = embedder
                    .embed(&[text])
                    .map(|mut v| v.pop().unwrap_or_default())
                    .context("embedding search query");
                if query_result_tx.send((id, result)).is_err() {
                    break; // receiver dropped (app shutting down)
                }
            }
            doc_job => {
                let result = process_job(doc_job, embedder);
                if result_tx.send(result).is_err() {
                    break; // receiver dropped (app shutting down)
                }
            }
        }
    }
}

/// Chunks + embeds one job. Kept separate from `run` so it stays a plain,
/// testable function of `(job, embedder) -> Result<IndexResult>` without
/// any channel/thread plumbing in the way.
fn process_job(job: IndexJob, embedder: &mut dyn Embedder) -> Result<IndexResult> {
    let (doc_id, doc_type, source_path, chunks) = match job {
        IndexJob::Query { .. } => {
            unreachable!("Query jobs are handled directly in run(), never passed to process_job")
        }
        IndexJob::Note(note) => {
            let doc_id = note.frontmatter.id;
            let source_path = note.path.clone();
            let chunks = ingestion::chunk_note(&note);
            (doc_id, DocType::Note, source_path, chunks)
        }
        IndexJob::Pdf(path) => {
            let chunks = ingestion::chunk_pdf(&path)
                .with_context(|| format!("indexing PDF {}", path.display()))?;
            // Chunks may be empty (e.g. an image-only PDF with no
            // extractable text) — fall back to the path-derived id so the
            // caller still learns this doc_id was (successfully, if
            // emptily) indexed rather than getting no result at all.
            let doc_id = chunks
                .first()
                .map(|c| c.doc_id)
                .unwrap_or_else(|| ingestion::pdf_doc_id(&path));
            (doc_id, DocType::Pdf, path, chunks)
        }
    };

    let texts: Vec<String> = chunks.iter().map(|c| c.text_content.clone()).collect();
    let embeddings = if texts.is_empty() {
        Vec::new()
    } else {
        embedder
            .embed(&texts)
            .with_context(|| format!("embedding chunks for {}", source_path.display()))?
    };

    Ok(IndexResult {
        doc_id,
        doc_type,
        source_path,
        chunks: chunks.into_iter().zip(embeddings).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    /// Deterministic stand-in for `EmbeddingEngine`: embeds each text as a
    /// 1-D vector holding its word count, so tests can assert on ordering
    /// without downloading the real model.
    struct FakeEmbedder;
    impl Embedder for FakeEmbedder {
        fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|t| vec![t.split_whitespace().count() as f32])
                .collect())
        }
    }

    struct FailingEmbedder;
    impl Embedder for FailingEmbedder {
        fn embed(&mut self, _texts: &[String]) -> Result<Vec<Vec<f32>>> {
            Err(anyhow::anyhow!("boom"))
        }
    }

    /// Polls `worker` until it has produced `n` results or a short
    /// deadline elapses — the worker runs on a real background thread, so
    /// results arrive asynchronously.
    fn wait_for_results(worker: &IndexingWorker, n: usize) -> Vec<Result<IndexResult>> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut out = Vec::new();
        while out.len() < n && Instant::now() < deadline {
            out.extend(worker.poll_results());
            if out.len() < n {
                thread::sleep(Duration::from_millis(10));
            }
        }
        out
    }

    /// Same as `wait_for_results` but for the query-embedding channel.
    fn wait_for_query_results(worker: &IndexingWorker, n: usize) -> Vec<(Uuid, Result<Vec<f32>>)> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut out = Vec::new();
        while out.len() < n && Instant::now() < deadline {
            out.extend(worker.poll_query_results());
            if out.len() < n {
                thread::sleep(Duration::from_millis(10));
            }
        }
        out
    }

    #[test]
    fn indexes_a_note_and_reports_chunk_embeddings() {
        let dir = tempdir().unwrap();
        let note = Note::create(dir.path(), "Judul", "satu dua tiga empat lima").unwrap();
        let note_id = note.frontmatter.id;

        let worker = IndexingWorker::spawn_with(|| Ok(Box::new(FakeEmbedder) as Box<dyn Embedder>));
        worker.submit_note(note);

        let results = wait_for_results(&worker, 1);
        assert_eq!(results.len(), 1);
        let result = results.into_iter().next().unwrap().unwrap();

        assert_eq!(result.doc_id, note_id);
        assert_eq!(result.doc_type, DocType::Note);
        assert_eq!(result.chunks.len(), 1);
        assert_eq!(result.chunks[0].0.text_content, "satu dua tiga empat lima");
        assert_eq!(result.chunks[0].1, vec![5.0]); // FakeEmbedder: word count
    }

    #[test]
    fn processes_multiple_submitted_jobs_in_order_of_completion() {
        let dir = tempdir().unwrap();
        let a = Note::create(dir.path(), "A", "satu dua").unwrap();
        let b = Note::create(dir.path(), "B", "satu dua tiga").unwrap();
        let (a_id, b_id) = (a.frontmatter.id, b.frontmatter.id);

        let worker = IndexingWorker::spawn_with(|| Ok(Box::new(FakeEmbedder) as Box<dyn Embedder>));
        worker.submit_note(a);
        worker.submit_note(b);

        let results: Vec<IndexResult> = wait_for_results(&worker, 2)
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(results.len(), 2);
        let doc_ids: Vec<Uuid> = results.iter().map(|r| r.doc_id).collect();
        assert!(doc_ids.contains(&a_id));
        assert!(doc_ids.contains(&b_id));
    }

    #[test]
    fn an_empty_note_body_yields_a_result_with_zero_chunks() {
        let dir = tempdir().unwrap();
        let note = Note::create(dir.path(), "Kosong", "").unwrap();
        let note_id = note.frontmatter.id;

        let worker = IndexingWorker::spawn_with(|| Ok(Box::new(FakeEmbedder) as Box<dyn Embedder>));
        worker.submit_note(note);

        let results = wait_for_results(&worker, 1);
        let result = results.into_iter().next().unwrap().unwrap();
        assert_eq!(result.doc_id, note_id);
        assert!(result.chunks.is_empty());
    }

    #[test]
    fn embedder_load_failure_is_reported_as_an_error_result() {
        let worker =
            IndexingWorker::spawn_with(|| Err::<Box<dyn Embedder>, _>(anyhow::anyhow!("no model")));
        let dir = tempdir().unwrap();
        worker.submit_note(Note::create(dir.path(), "Judul", "isi").unwrap());

        let results = wait_for_results(&worker, 1);
        assert_eq!(results.len(), 1);
        assert!(results[0].is_err());
    }

    #[test]
    fn embed_call_failure_is_reported_as_an_error_result_without_crashing_the_worker() {
        let worker =
            IndexingWorker::spawn_with(|| Ok(Box::new(FailingEmbedder) as Box<dyn Embedder>));
        let dir = tempdir().unwrap();
        worker.submit_note(Note::create(dir.path(), "Judul", "isi teks").unwrap());

        let results = wait_for_results(&worker, 1);
        assert_eq!(results.len(), 1);
        assert!(results[0].is_err());
    }

    #[test]
    fn chunk_pdf_extraction_failure_is_reported_without_crashing_the_worker() {
        let worker = IndexingWorker::spawn_with(|| Ok(Box::new(FakeEmbedder) as Box<dyn Embedder>));
        worker.submit_pdf(PathBuf::from("/nonexistent/missing.pdf"));

        // Follow up with a real job on the same worker to prove the
        // thread survived the earlier error instead of panicking out.
        let dir = tempdir().unwrap();
        worker.submit_note(Note::create(dir.path(), "Setelah", "masih hidup").unwrap());

        let results = wait_for_results(&worker, 2);
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|r| r.is_err()));
        assert!(results.iter().any(|r| r.is_ok()));
    }

    #[test]
    fn submit_query_reports_the_embedded_vector_tagged_with_its_request_id() {
        let worker = IndexingWorker::spawn_with(|| Ok(Box::new(FakeEmbedder) as Box<dyn Embedder>));
        let id = worker.submit_query("satu dua tiga".to_string());

        let results = wait_for_query_results(&worker, 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, id);
        assert_eq!(results[0].1.as_ref().unwrap(), &vec![3.0]); // FakeEmbedder: word count
    }

    #[test]
    fn query_embedding_failure_is_reported_without_crashing_the_worker() {
        let worker =
            IndexingWorker::spawn_with(|| Ok(Box::new(FailingEmbedder) as Box<dyn Embedder>));
        worker.submit_query("q".to_string());

        let results = wait_for_query_results(&worker, 1);
        assert_eq!(results.len(), 1);
        assert!(results[0].1.is_err());
    }

    #[test]
    fn document_and_query_jobs_on_the_same_worker_arrive_on_their_own_channels() {
        let worker = IndexingWorker::spawn_with(|| Ok(Box::new(FakeEmbedder) as Box<dyn Embedder>));
        let dir = tempdir().unwrap();
        worker.submit_note(Note::create(dir.path(), "Judul", "satu dua").unwrap());
        let query_id = worker.submit_query("tiga".to_string());

        let doc_results = wait_for_results(&worker, 1);
        assert_eq!(doc_results.len(), 1);
        assert!(doc_results[0].is_ok());

        let query_results = wait_for_query_results(&worker, 1);
        assert_eq!(query_results.len(), 1);
        assert_eq!(query_results[0].0, query_id);
    }
}
