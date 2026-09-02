//! Entry point: initializes logging and runs the `eframe` window (§4).

mod app;
mod block;
mod canvas;
mod core;
mod i18n;
mod llm;
mod markdown;
mod notes;
mod pdf;
mod ui;

// §Fase 10 "memory profiling", opt-in via `--features dhat-heap` (see
// Cargo.toml's `dhat` dependency comment) — swaps the global allocator for
// `dhat`'s instrumented one only when the feature is enabled, so a normal
// build carries zero profiling overhead.
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() -> eframe::Result<()> {
    // Held for the whole process lifetime: `Profiler`'s `Drop` impl is what
    // actually writes `dhat-heap.json`, so it must outlive `run_native`
    // (which blocks until the window closes), not just be constructed.
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    env_logger::init();

    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "MNEMONIC",
        native_options,
        Box::new(|_cc| Ok(Box::new(app::MnemonicApp::new()))),
    )
}
