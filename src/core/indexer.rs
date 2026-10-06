//! Background indexing worker (§5: "Background task pool"): runs chunking
//! and embedding off the UI thread so opening a vault or editing a large
//! document doesn't stall `egui`'s frame loop. Shaped like
//! `notes::watcher::VaultWatcher` — spawn a thread, feed it jobs, poll a
//! channel once per frame — rather than pulling in `tokio`/`rayon` for
//! what is, per document, a single call into `EmbeddingEngine::embed`
//! (which already batches all of a document's chunks into one model
//! call). This module never touches SQLite itself: the caller owns the
//! `core::storage::IndexStore` and writes each `IndexResult` into
//! `IndexStore::replace_chunks` on the UI thread.
//! Also handles ad-hoc *query* embedding (search + RAG chat) and optional
//! cross-encoder *reranking* of search candidates over their own
//! channels, reusing this worker's lazily-loaded models instead of paying
//! for a second model load in a role-specific worker. Callers: `app`.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result, anyhow};
use uuid::Uuid;

use super::embedding::{EmbeddingEngine, RerankEngine};
use super::ingestion::{self, DocumentChunk};
use crate::notes::Note;

/// What kind of source a chunk batch came from — mirrors the `doc_type`
/// column `IndexResult`s are ultimately written under in
/// `core::storage`'s `document_chunks` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocType {
    Note,
    Pdf,
    /// CSV/XLSX sheet (§3.8.4).
    Sheet,
}

impl DocType {
    pub fn as_str(self) -> &'static str {
        match self {
            DocType::Note => "note",
            DocType::Pdf => "pdf",
            DocType::Sheet => "sheet",
        }
    }
}

/// A unit of work submitted to the indexing worker.
pub enum IndexJob {
    Note(Note),
    Pdf(PathBuf),
    /// A CSV/XLSX file (§3.8.4).
    Sheet(PathBuf),
    /// A search/chat query string to embed — carries its own id since,
    /// unlike notes/PDFs, a query has no natural `doc_id` to key its
    /// result on.
    Query { id: Uuid, text: String },
    /// Score `documents` against `query` with the cross-encoder.
    Rerank {
        id: Uuid,
        query: String,
        documents: Vec<String>,
    },
}

/// Chunks + embeddings for one document, ready for
/// `core::storage::IndexStore::replace_chunks`.
pub struct IndexResult {
    pub doc_id: Uuid,
    pub doc_type: DocType,
    /// Note title, or the PDF's/sheet's file name — indexed for keyword
    /// search.
    pub title: String,
    pub source_path: PathBuf,
    pub chunks: Vec<(DocumentChunk, Vec<f32>)>,
    /// `ingestion::note_content_hash` of what was chunked (empty for
    /// PDFs; `ingestion::sheet_file_stamp` for sheets), recorded by the
    /// caller via `IndexStore::set_document_hash`.
    pub content_hash: String,
}

/// Anything that can turn texts into embedding vectors. Lets tests inject
/// a fake, instant embedder instead of downloading/loading the real
/// FastEmbed model.
pub trait Embedder: Send {
    /// Embeds document passages.
    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>>;

    /// Embeds a search query. Models with asymmetric query/passage
    /// encodings (E5) override this; the default treats it as a passage.
    fn embed_query(&mut self, text: &str) -> Result<Vec<f32>> {
        self.embed(&[text.to_string()])?
            .pop()
            .context("embedder returned no vector")
    }
}

impl Embedder for EmbeddingEngine {
    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        EmbeddingEngine::embed(self, texts)
    }

    fn embed_query(&mut self, text: &str) -> Result<Vec<f32>> {
        EmbeddingEngine::embed_query(self, text)
    }
}

/// Anything that scores documents against a query (higher = more
/// relevant), one score per document in input order.
pub trait Reranker: Send {
    fn rerank(&mut self, query: &str, documents: &[String]) -> Result<Vec<f32>>;
}

impl Reranker for RerankEngine {
    fn rerank(&mut self, query: &str, documents: &[String]) -> Result<Vec<f32>> {
        RerankEngine::rerank(self, query, documents)
    }
}

type EmbedderFactory = Box<dyn FnOnce() -> Result<Box<dyn Embedder>> + Send>;
type RerankerFactory = Box<dyn FnOnce() -> Result<Box<dyn Reranker>> + Send>;

/// A dedicated background thread that chunks + embeds submitted
/// notes/PDFs and reports results back over a channel. One worker thread
/// is enough here rather than a wider pool: `EmbeddingEngine::embed`
/// already batches an entire document's chunks into a single model call,
/// so the bottleneck is the model's own inference, not job scheduling.
pub struct IndexingWorker {
    job_tx: Sender<IndexJob>,
    result_rx: Receiver<Result<IndexResult>>,
    query_result_rx: Receiver<(Uuid, Result<Vec<f32>>)>,
    rerank_result_rx: Receiver<(Uuid, Result<Vec<f32>>)>,
    _handle: JoinHandle<()>,
}

impl IndexingWorker {
    /// Spawns the worker backed by the real FastEmbed models. Loading is
    /// deferred to the first job that needs each model, so `spawn` itself
    /// never blocks on a download/load.
    pub fn spawn() -> IndexingWorker {
        Self::spawn_with_reranker(
            || EmbeddingEngine::new().map(|e| Box::new(e) as Box<dyn Embedder>),
            || RerankEngine::new().map(|r| Box::new(r) as Box<dyn Reranker>),
        )
    }

    /// Spawns the worker with a caller-supplied embedder factory and no
    /// reranker (rerank jobs report an error). Tests inject a fake
    /// `Embedder` to stay offline and instant.
    pub fn spawn_with<F>(make_embedder: F) -> IndexingWorker
    where
        F: FnOnce() -> Result<Box<dyn Embedder>> + Send + 'static,
    {
        Self::spawn_with_reranker(make_embedder, || Err(anyhow!("no reranker configured")))
    }

    /// Spawns the worker with both model factories supplied by the caller.
    pub fn spawn_with_reranker<F, R>(make_embedder: F, make_reranker: R) -> IndexingWorker
    where
        F: FnOnce() -> Result<Box<dyn Embedder>> + Send + 'static,
        R: FnOnce() -> Result<Box<dyn Reranker>> + Send + 'static,
    {
        let (job_tx, job_rx) = channel::<IndexJob>();
        let (result_tx, result_rx) = channel::<Result<IndexResult>>();
        let (query_result_tx, query_result_rx) = channel::<(Uuid, Result<Vec<f32>>)>();
        let (rerank_result_tx, rerank_result_rx) = channel::<(Uuid, Result<Vec<f32>>)>();
        let outputs = Outputs {
            results: result_tx,
            queries: query_result_tx,
            reranks: rerank_result_tx,
        };

        let handle = thread::spawn(move || {
            run(
                job_rx,
                outputs,
                Box::new(make_embedder),
                Box::new(make_reranker),
            )
        });

        IndexingWorker {
            job_tx,
            result_rx,
            query_result_rx,
            rerank_result_rx,
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

    /// Queues a CSV/XLSX sheet for (re)indexing. Non-blocking.
    pub fn submit_sheet(&self, path: PathBuf) {
        let _ = self.job_tx.send(IndexJob::Sheet(path));
    }

    /// Queues a search/chat query string for embedding, returning the
    /// request id its result will be tagged with. Non-blocking; the result
    /// — or nothing, if the worker thread has already exited — arrives via
    /// `poll_query_results`.
    pub fn submit_query(&self, text: String) -> Uuid {
        let id = Uuid::new_v4();
        let _ = self.job_tx.send(IndexJob::Query { id, text });
        id
    }

    /// Queues a rerank of `documents` against `query`; scores arrive via
    /// `poll_rerank_results` tagged with the returned id.
    pub fn submit_rerank(&self, query: String, documents: Vec<String>) -> Uuid {
        let id = Uuid::new_v4();
        let _ = self.job_tx.send(IndexJob::Rerank {
            id,
            query,
            documents,
        });
        id
    }

    /// Drains all results currently available without blocking. Call once
    /// per UI frame and write each `Ok` result into
    /// `IndexStore::replace_chunks`; log `Err` results.
    pub fn poll_results(&self) -> Vec<Result<IndexResult>> {
        drain(&self.result_rx)
    }

    /// Drains all query-embedding results currently available without
    /// blocking. Match each id against the id returned by `submit_query`
    /// to tell concurrent search vs. chat requests apart.
    pub fn poll_query_results(&self) -> Vec<(Uuid, Result<Vec<f32>>)> {
        drain(&self.query_result_rx)
    }

    /// Drains all rerank results currently available without blocking.
    pub fn poll_rerank_results(&self) -> Vec<(Uuid, Result<Vec<f32>>)> {
        drain(&self.rerank_result_rx)
    }
}

fn drain<T>(rx: &Receiver<T>) -> Vec<T> {
    // Stops at `Empty` and at `Disconnected` (worker gone) alike.
    let mut out = Vec::new();
    while let Ok(item) = rx.try_recv() {
        out.push(item);
    }
    out
}

struct Outputs {
    results: Sender<Result<IndexResult>>,
    queries: Sender<(Uuid, Result<Vec<f32>>)>,
    reranks: Sender<(Uuid, Result<Vec<f32>>)>,
}

/// A model built on first use and remembered as `Failed` afterward, so a
/// broken model doesn't retry the same slow failure on every job.
enum Lazy<T: ?Sized> {
    Pending(Box<dyn FnOnce() -> Result<Box<T>> + Send>),
    Ready(Box<T>),
    Failed(String),
}

impl<T: ?Sized> Lazy<T> {
    fn get(&mut self, what: &str) -> Result<&mut T> {
        if matches!(self, Lazy::Pending(_)) {
            let Lazy::Pending(build) = std::mem::replace(self, Lazy::Failed(String::new())) else {
                unreachable!("just matched Pending above");
            };
            match build() {
                Ok(model) => *self = Lazy::Ready(model),
                Err(e) => {
                    let msg = format!("loading {what}: {e:#}");
                    *self = Lazy::Failed(msg.clone());
                    return Err(anyhow!(msg));
                }
            }
        }
        match self {
            Lazy::Ready(model) => Ok(model.as_mut()),
            Lazy::Failed(msg) => Err(anyhow!(
                "{what} failed to load earlier; skipping job ({msg})"
            )),
            Lazy::Pending(_) => unreachable!("resolved above"),
        }
    }
}

fn run(
    job_rx: Receiver<IndexJob>,
    out: Outputs,
    make_embedder: EmbedderFactory,
    make_reranker: RerankerFactory,
) {
    let mut embedder: Lazy<dyn Embedder> = Lazy::Pending(make_embedder);
    let mut reranker: Lazy<dyn Reranker> = Lazy::Pending(make_reranker);

    while let Ok(job) = job_rx.recv() {
        let sent = match job {
            IndexJob::Rerank {
                id,
                query,
                documents,
            } => {
                let result = reranker
                    .get("reranker model")
                    .and_then(|r| r.rerank(&query, &documents));
                out.reranks.send((id, result)).is_ok()
            }
            IndexJob::Query { id, text } => {
                let result = embedder
                    .get("embedding model")
                    .and_then(|e| e.embed_query(&text).context("embedding search query"));
                out.queries.send((id, result)).is_ok()
            }
            doc_job => {
                let result = embedder
                    .get("embedding model")
                    .and_then(|e| process_job(doc_job, e));
                out.results.send(result).is_ok()
            }
        };
        if !sent {
            break; // receiver dropped (app shutting down)
        }
    }
}

/// Chunks + embeds one document job. Kept separate from `run` so it stays
/// a plain, testable function of `(job, embedder) -> Result<IndexResult>`.
fn process_job(job: IndexJob, embedder: &mut dyn Embedder) -> Result<IndexResult> {
    let (doc_id, doc_type, title, source_path, chunks, inputs, content_hash) = match job {
        IndexJob::Query { .. } | IndexJob::Rerank { .. } => {
            unreachable!("query/rerank jobs are handled directly in run()")
        }
        IndexJob::Note(note) => {
            let title = note.frontmatter.title.clone();
            let content_hash = ingestion::note_content_hash(&note);
            let chunks = ingestion::chunk_note(&note);
            let text = ingestion::note_index_text(&note);
            let inputs: Vec<String> = chunks
                .iter()
                .map(|c| {
                    let heading = ingestion::heading_at(&text, c.char_offset);
                    ingestion::embedding_input(&title, heading.as_deref(), &c.text_content)
                })
                .collect();
            (
                note.frontmatter.id,
                DocType::Note,
                title,
                note.path,
                chunks,
                inputs,
                content_hash,
            )
        }
        IndexJob::Pdf(path) => {
            let chunks = ingestion::chunk_pdf(&path)
                .with_context(|| format!("indexing PDF {}", path.display()))?;
            // Chunks may be empty (e.g. an image-only PDF with no
            // extractable text) — fall back to the path-derived id so the
            // caller still learns this doc_id was (emptily) indexed.
            let doc_id = chunks
                .first()
                .map(|c| c.doc_id)
                .unwrap_or_else(|| ingestion::pdf_doc_id(&path));
            let title = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let inputs = chunks
                .iter()
                .map(|c| ingestion::embedding_input(&title, None, &c.text_content))
                .collect();
            (doc_id, DocType::Pdf, title, path, chunks, inputs, String::new())
        }
        IndexJob::Sheet(path) => {
            // Stamp before reading: an edit landing mid-read then just
            // triggers another pass on the next rescan.
            let stamp = ingestion::sheet_file_stamp(&path).unwrap_or_default();
            let chunks = ingestion::chunk_sheet(&path)?;
            let title = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let inputs = chunks
                .iter()
                .map(|c| ingestion::embedding_input(&title, None, &c.text_content))
                .collect();
            let doc_id = ingestion::sheet_doc_id(&path);
            (doc_id, DocType::Sheet, title, path, chunks, inputs, stamp)
        }
    };

    let embeddings = if inputs.is_empty() {
        Vec::new()
    } else {
        embedder
            .embed(&inputs)
            .with_context(|| format!("embedding chunks for {}", source_path.display()))?
    };
    if embeddings.len() != chunks.len() {
        return Err(anyhow!(
            "embedder returned {} vectors for {} chunks of {}",
            embeddings.len(),
            chunks.len(),
            source_path.display()
        ));
    }

    Ok(IndexResult {
        doc_id,
        doc_type,
        title,
        source_path,
        chunks: chunks.into_iter().zip(embeddings).collect(),
        content_hash,
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

    /// Records every text it is asked to embed.
    struct Recorder(Vec<String>);
    impl Embedder for Recorder {
        fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            self.0.extend(texts.iter().cloned());
            Ok(texts.iter().map(|_| vec![1.0]).collect())
        }
    }

    /// Scores each document by how many times it contains the query.
    struct FakeReranker;
    impl Reranker for FakeReranker {
        fn rerank(&mut self, query: &str, documents: &[String]) -> Result<Vec<f32>> {
            Ok(documents
                .iter()
                .map(|d| d.matches(query).count() as f32)
                .collect())
        }
    }

    fn fake_worker() -> IndexingWorker {
        IndexingWorker::spawn_with(|| Ok(Box::new(FakeEmbedder) as Box<dyn Embedder>))
    }

    /// Polls `poll` until it has produced `n` items or a short deadline
    /// elapses — the worker runs on a real background thread.
    fn wait_for<T>(n: usize, mut poll: impl FnMut() -> Vec<T>) -> Vec<T> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut out = Vec::new();
        while out.len() < n && Instant::now() < deadline {
            out.extend(poll());
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

        let worker = fake_worker();
        worker.submit_note(note);

        let results = wait_for(1, || worker.poll_results());
        assert_eq!(results.len(), 1);
        let result = results.into_iter().next().unwrap().unwrap();

        assert_eq!(result.doc_id, note_id);
        assert_eq!(result.doc_type, DocType::Note);
        assert_eq!(result.title, "Judul");
        assert_eq!(result.chunks.len(), 1);
        assert_eq!(result.chunks[0].0.text_content, "satu dua tiga empat lima");
        // FakeEmbedder counts words of the embedded input: title + body.
        assert_eq!(result.chunks[0].1, vec![6.0]);
    }

    #[test]
    fn embedded_input_includes_the_section_heading() {
        let dir = tempdir().unwrap();
        let note = Note::create(dir.path(), "Resep", "# Bahan\ntelur").unwrap();
        let mut recorder = Recorder(Vec::new());
        let result = process_job(IndexJob::Note(note), &mut recorder).unwrap();
        assert_eq!(result.chunks.len(), 1);
        assert_eq!(recorder.0, vec!["Resep › Bahan\n# Bahan\ntelur".to_string()]);
    }

    #[test]
    fn processes_multiple_submitted_jobs_in_order_of_completion() {
        let dir = tempdir().unwrap();
        let a = Note::create(dir.path(), "A", "satu dua").unwrap();
        let b = Note::create(dir.path(), "B", "satu dua tiga").unwrap();
        let (a_id, b_id) = (a.frontmatter.id, b.frontmatter.id);

        let worker = fake_worker();
        worker.submit_note(a);
        worker.submit_note(b);

        let results: Vec<IndexResult> = wait_for(2, || worker.poll_results())
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

        let worker = fake_worker();
        worker.submit_note(note);

        let results = wait_for(1, || worker.poll_results());
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
        worker.submit_query("q".to_string());

        let results = wait_for(1, || worker.poll_results());
        assert_eq!(results.len(), 1);
        assert!(results[0].is_err());
        // A later job reports the remembered failure instead of retrying.
        let queries = wait_for(1, || worker.poll_query_results());
        assert!(queries[0].1.is_err());
    }

    #[test]
    fn embed_call_failure_is_reported_as_an_error_result_without_crashing_the_worker() {
        let worker =
            IndexingWorker::spawn_with(|| Ok(Box::new(FailingEmbedder) as Box<dyn Embedder>));
        let dir = tempdir().unwrap();
        worker.submit_note(Note::create(dir.path(), "Judul", "isi teks").unwrap());

        let results = wait_for(1, || worker.poll_results());
        assert_eq!(results.len(), 1);
        assert!(results[0].is_err());
    }

    #[test]
    fn chunk_pdf_extraction_failure_is_reported_without_crashing_the_worker() {
        let worker = fake_worker();
        worker.submit_pdf(PathBuf::from("/nonexistent/missing.pdf"));

        // Follow up with a real job on the same worker to prove the
        // thread survived the earlier error instead of panicking out.
        let dir = tempdir().unwrap();
        worker.submit_note(Note::create(dir.path(), "Setelah", "masih hidup").unwrap());

        let results = wait_for(2, || worker.poll_results());
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|r| r.is_err()));
        assert!(results.iter().any(|r| r.is_ok()));
    }

    #[test]
    fn submit_query_reports_the_embedded_vector_tagged_with_its_request_id() {
        let worker = fake_worker();
        let id = worker.submit_query("satu dua tiga".to_string());

        let results = wait_for(1, || worker.poll_query_results());
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, id);
        assert_eq!(results[0].1.as_ref().unwrap(), &vec![3.0]); // FakeEmbedder: word count
    }

    #[test]
    fn query_embedding_failure_is_reported_without_crashing_the_worker() {
        let worker =
            IndexingWorker::spawn_with(|| Ok(Box::new(FailingEmbedder) as Box<dyn Embedder>));
        worker.submit_query("q".to_string());

        let results = wait_for(1, || worker.poll_query_results());
        assert_eq!(results.len(), 1);
        assert!(results[0].1.is_err());
    }

    #[test]
    fn document_and_query_jobs_on_the_same_worker_arrive_on_their_own_channels() {
        let worker = fake_worker();
        let dir = tempdir().unwrap();
        worker.submit_note(Note::create(dir.path(), "Judul", "satu dua").unwrap());
        let query_id = worker.submit_query("tiga".to_string());

        let doc_results = wait_for(1, || worker.poll_results());
        assert_eq!(doc_results.len(), 1);
        assert!(doc_results[0].is_ok());

        let query_results = wait_for(1, || worker.poll_query_results());
        assert_eq!(query_results.len(), 1);
        assert_eq!(query_results[0].0, query_id);
    }

    #[test]
    fn rerank_jobs_score_documents_without_loading_the_embedder() {
        let worker = IndexingWorker::spawn_with_reranker(
            || Err::<Box<dyn Embedder>, _>(anyhow::anyhow!("embedder must not load")),
            || Ok(Box::new(FakeReranker) as Box<dyn Reranker>),
        );
        let id = worker.submit_rerank(
            "nasi".to_string(),
            vec!["roti".to_string(), "nasi nasi".to_string()],
        );
        let results = wait_for(1, || worker.poll_rerank_results());
        assert_eq!(results[0].0, id);
        assert_eq!(results[0].1.as_ref().unwrap(), &vec![0.0, 2.0]);
    }

    #[test]
    fn rerank_without_a_reranker_reports_an_error() {
        let worker = fake_worker();
        worker.submit_rerank("q".to_string(), vec!["d".to_string()]);
        let results = wait_for(1, || worker.poll_rerank_results());
        assert!(results[0].1.is_err());
    }
}
