//! Internationalization via Project Fluent (`fluent-rs`), §3.6.
//! Callers: `app.rs`, all `ui/*.rs` views.

pub mod loader;
pub mod locale_manager;

pub use locale_manager::LocaleManager;
