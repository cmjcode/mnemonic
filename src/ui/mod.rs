//! UI presentation helpers that don't belong to a specific domain module.
//! The Notes Grid itself (§4 planned `ui/notes_grid_view.rs`) is still
//! implemented as a method on `LontarApp` in `app.rs` for now — splitting
//! it out hit enough nested-closure/borrow-checker friction to be worth
//! deferring past this phase; `theme` is the part that benefits from
//! living on its own regardless. Callers: `app.rs`.

pub mod theme;
