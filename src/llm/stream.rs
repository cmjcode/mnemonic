//! Background LLM generation worker (§3.4 point 3, §6 risk 2): runs
//! Candle inference on a dedicated thread and streams decoded text
//! fragments back over a channel so the (future, Fase 7) chat UI can
//! render tokens as they arrive instead of blocking `egui`'s frame loop
//! on a multi-second generation call. Shaped like
//! `core::indexer::IndexingWorker`/`Embedder`: one worker thread, a job
//! channel in, a results channel out, polled once per frame — and the
//! same injectable-trait split (`Generator` here, `Embedder` there) so
//! tests exercise the worker/streaming plumbing with a fake, instant
//! generator instead of downloading/loading the real ~1.1 GB Qwen2.5
//! model. Callers: future chat UI (§Fase 7).

use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::{self, JoinHandle};

use anyhow::Result;
use uuid::Uuid;

use super::candle_engine::CandleEngine;

/// Anything that can turn a prompt into a streamed reply. Lets tests
/// inject a fake, instant generator instead of the real Candle/Qwen2.5
/// model — the model itself only needs to satisfy this trait (impl in
/// `llm::candle_engine`) to work as-is with `GenerationWorker::spawn`.
pub trait Generator: Send {
    /// Generates a reply to `prompt`, calling `on_token` with each new
    /// text fragment as it's decoded, stopping after at most
    /// `max_tokens` generated tokens (or sooner, on a model-specific
    /// end-of-turn token).
    fn generate(
        &mut self,
        prompt: &str,
        max_tokens: usize,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<()>;
}

/// A unit of work submitted to the generation worker.
struct GenerationJob {
    id: Uuid,
    prompt: String,
    max_tokens: usize,
}

/// One streamed update for a submitted generation request, tagged with
/// its `Uuid` so a caller that started a new request while an old one
/// was still draining can tell them apart (and discard stale events).
#[derive(Debug, Clone, PartialEq)]
pub enum GenerationEvent {
    /// A newly-decoded text fragment, ready to append to the chat bubble
    /// under construction.
    Token(String),
    /// Generation for this request finished successfully (end-of-turn
    /// token or `max_tokens` reached).
    Done,
    /// Generation for this request failed (model load failure, a
    /// tokenizer/inference error, ...); carries a display-ready message
    /// rather than `anyhow::Error` so this type stays comparable in
    /// tests and cheap to pass across the channel.
    Error(String),
}

/// Default cap on generated tokens per reply — generous enough for a
/// multi-paragraph answer while bounding worst-case latency/CPU time for
/// a runaway (non-terminating) generation.
pub use super::DEFAULT_MAX_TOKENS;

/// A dedicated background thread that generates streamed replies for
/// submitted prompts, one at a time (chat is inherently sequential —
/// there's no benefit to a wider pool here, unlike a batchable embedding
/// workload).
pub struct GenerationWorker {
    job_tx: Sender<GenerationJob>,
    event_rx: Receiver<(Uuid, GenerationEvent)>,
    _handle: JoinHandle<()>,
}

impl GenerationWorker {
    /// Spawns the worker backed by the real Candle/Qwen2.5 engine.
    /// Loading is deferred to the first submitted job, so `spawn` itself
    /// never blocks on the ~1.1 GB model download/load.
    pub fn spawn() -> GenerationWorker {
        Self::spawn_with(|| CandleEngine::new().map(|e| Box::new(e) as Box<dyn Generator>))
    }

    /// Spawns the worker with a caller-supplied generator factory.
    /// Production uses `spawn()`; tests inject a fake `Generator` to stay
    /// offline and instant.
    pub fn spawn_with<F>(make_generator: F) -> GenerationWorker
    where
        F: FnOnce() -> Result<Box<dyn Generator>> + Send + 'static,
    {
        let (job_tx, job_rx) = channel::<GenerationJob>();
        let (event_tx, event_rx) = channel::<(Uuid, GenerationEvent)>();

        let handle = thread::spawn(move || run(job_rx, event_tx, make_generator));

        GenerationWorker {
            job_tx,
            event_rx,
            _handle: handle,
        }
    }

    /// Queues a prompt for generation, returning the request id its
    /// streamed events will be tagged with. Non-blocking; silently
    /// dropped if the worker thread has already exited.
    pub fn submit(&self, prompt: String, max_tokens: usize) -> Uuid {
        let id = Uuid::new_v4();
        let _ = self.job_tx.send(GenerationJob {
            id,
            prompt,
            max_tokens,
        });
        id
    }

    /// Drains all events currently available without blocking. Call once
    /// per UI frame (same shape as `IndexingWorker::poll_results`) and
    /// append each `Token` to the in-progress chat bubble matching its
    /// request id; `Done`/`Error` close that bubble out.
    pub fn poll_events(&self) -> Vec<(Uuid, GenerationEvent)> {
        let mut out = Vec::new();
        loop {
            match self.event_rx.try_recv() {
                Ok(event) => out.push(event),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        out
    }
}

/// Lazily-constructed generator state inside the worker thread — same
/// shape as `core::indexer`'s `EmbedderState`: built once from the first
/// job so idle time never pays the model-load cost, and remembered as
/// `Failed` afterward so a broken model doesn't retry the same slow
/// failure on every subsequent job.
enum GeneratorState {
    Pending(Box<dyn FnOnce() -> Result<Box<dyn Generator>> + Send>),
    Ready(Box<dyn Generator>),
    Failed,
}

fn run<F>(
    job_rx: Receiver<GenerationJob>,
    event_tx: Sender<(Uuid, GenerationEvent)>,
    make_generator: F,
) where
    F: FnOnce() -> Result<Box<dyn Generator>> + Send + 'static,
{
    let mut state = GeneratorState::Pending(Box::new(make_generator));

    while let Ok(job) = job_rx.recv() {
        if matches!(state, GeneratorState::Pending(_)) {
            let GeneratorState::Pending(build) =
                std::mem::replace(&mut state, GeneratorState::Failed)
            else {
                unreachable!("just matched Pending above");
            };
            match build() {
                Ok(generator) => state = GeneratorState::Ready(generator),
                Err(e) => {
                    let _ = event_tx.send((
                        job.id,
                        GenerationEvent::Error(format!("loading generation model: {e:#}")),
                    ));
                    continue;
                }
            }
        }

        let generator = match &mut state {
            GeneratorState::Ready(g) => g.as_mut(),
            GeneratorState::Failed => {
                let _ = event_tx.send((
                    job.id,
                    GenerationEvent::Error(
                        "generation model failed to load earlier; skipping job".to_string(),
                    ),
                ));
                continue;
            }
            GeneratorState::Pending(_) => unreachable!("resolved above"),
        };

        let id = job.id;
        let mut on_token = |text: &str| {
            let _ = event_tx.send((id, GenerationEvent::Token(text.to_string())));
        };
        let final_event = match generator.generate(&job.prompt, job.max_tokens, &mut on_token) {
            Ok(()) => GenerationEvent::Done,
            Err(e) => GenerationEvent::Error(format!("{e:#}")),
        };
        if event_tx.send((id, final_event)).is_err() {
            break; // receiver dropped (app shutting down)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Deterministic stand-in for `CandleEngine`: emits a fixed sequence
    /// of text fragments instantly, so tests can assert on streaming
    /// order without downloading/loading the real ~1.1 GB model.
    struct FakeGenerator {
        fragments: Vec<&'static str>,
    }
    impl Generator for FakeGenerator {
        fn generate(
            &mut self,
            _prompt: &str,
            max_tokens: usize,
            on_token: &mut dyn FnMut(&str),
        ) -> Result<()> {
            for frag in self.fragments.iter().take(max_tokens) {
                on_token(frag);
            }
            Ok(())
        }
    }

    struct FailingGenerator;
    impl Generator for FailingGenerator {
        fn generate(
            &mut self,
            _prompt: &str,
            _max_tokens: usize,
            _on_token: &mut dyn FnMut(&str),
        ) -> Result<()> {
            Err(anyhow::anyhow!("boom"))
        }
    }

    /// Polls `worker` until it has produced `n` events or a short
    /// deadline elapses — the worker runs on a real background thread,
    /// so events arrive asynchronously.
    fn wait_for_events(worker: &GenerationWorker, n: usize) -> Vec<(Uuid, GenerationEvent)> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut out = Vec::new();
        while out.len() < n && Instant::now() < deadline {
            out.extend(worker.poll_events());
            if out.len() < n {
                thread::sleep(Duration::from_millis(10));
            }
        }
        out
    }

    #[test]
    fn streams_tokens_in_order_then_done() {
        let worker = GenerationWorker::spawn_with(|| {
            Ok(Box::new(FakeGenerator {
                fragments: vec!["Hal", "o", " dunia"],
            }) as Box<dyn Generator>)
        });
        let id = worker.submit("halo?".to_string(), 10);

        let events = wait_for_events(&worker, 4);
        assert_eq!(events.len(), 4);
        assert!(events.iter().all(|(eid, _)| *eid == id));
        assert_eq!(events[0].1, GenerationEvent::Token("Hal".to_string()));
        assert_eq!(events[1].1, GenerationEvent::Token("o".to_string()));
        assert_eq!(events[2].1, GenerationEvent::Token(" dunia".to_string()));
        assert_eq!(events[3].1, GenerationEvent::Done);
    }

    #[test]
    fn max_tokens_caps_the_number_of_fragments_emitted() {
        let worker = GenerationWorker::spawn_with(|| {
            Ok(Box::new(FakeGenerator {
                fragments: vec!["a", "b", "c"],
            }) as Box<dyn Generator>)
        });
        worker.submit("q".to_string(), 2);

        let events = wait_for_events(&worker, 3); // 2 tokens + Done
        assert_eq!(events.len(), 3);
        assert_eq!(events[2].1, GenerationEvent::Done);
    }

    #[test]
    fn each_submission_gets_a_distinct_request_id() {
        let worker = GenerationWorker::spawn_with(|| {
            Ok(Box::new(FakeGenerator {
                fragments: vec!["x"],
            }) as Box<dyn Generator>)
        });
        let id_a = worker.submit("a".to_string(), 5);
        let id_b = worker.submit("b".to_string(), 5);
        assert_ne!(id_a, id_b);

        let events = wait_for_events(&worker, 4); // 2x (Token + Done)
        assert_eq!(events.len(), 4);
        assert!(events.iter().any(|(id, _)| *id == id_a));
        assert!(events.iter().any(|(id, _)| *id == id_b));
    }

    #[test]
    fn generator_load_failure_is_reported_as_an_error_event() {
        let worker = GenerationWorker::spawn_with(|| {
            Err::<Box<dyn Generator>, _>(anyhow::anyhow!("no model"))
        });
        worker.submit("q".to_string(), 5);

        let events = wait_for_events(&worker, 1);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].1, GenerationEvent::Error(_)));
    }

    #[test]
    fn generate_call_failure_is_reported_without_crashing_the_worker() {
        let worker =
            GenerationWorker::spawn_with(|| Ok(Box::new(FailingGenerator) as Box<dyn Generator>));
        worker.submit("q".to_string(), 5);

        let first = wait_for_events(&worker, 1);
        assert_eq!(first.len(), 1);
        assert!(matches!(first[0].1, GenerationEvent::Error(_)));

        // Follow up with a second job on the same worker to prove the
        // thread survived the earlier error instead of panicking out.
        worker.submit("q2".to_string(), 5);
        let second = wait_for_events(&worker, 1);
        assert_eq!(second.len(), 1);
        assert!(matches!(second[0].1, GenerationEvent::Error(_)));
    }

    #[test]
    fn zero_max_tokens_still_reports_done_with_no_tokens() {
        let worker = GenerationWorker::spawn_with(|| {
            Ok(Box::new(FakeGenerator {
                fragments: vec!["a"],
            }) as Box<dyn Generator>)
        });
        worker.submit("q".to_string(), 0);

        let events = wait_for_events(&worker, 1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1, GenerationEvent::Done);
    }
}
