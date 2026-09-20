//! MNEMONIC Core Library.

pub mod api;
/// Aplikasi desktop (eframe). Hanya ada pada build dengan fitur `gui`.
#[cfg(feature = "gui")]
pub mod app;
pub mod block;
pub mod canvas;
pub mod core;
pub mod export;
pub mod graph;
pub mod i18n;
pub mod llm;
pub mod markdown;
pub mod mermaid;
pub mod notes;
pub mod pdf;
pub mod reading_theme;
pub mod settings;
pub mod sheet;
/// Widget desktop. Hanya ada pada build dengan fitur `gui`.
#[cfg(feature = "gui")]
pub mod ui;
