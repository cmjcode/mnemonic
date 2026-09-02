//! Notes module: vault management, note CRUD, frontmatter, file watching,
//! and trash retention (§3.1 in pengembangan.md). Callers: `app.rs`.

pub mod frontmatter;
pub mod note;
pub mod query;
pub mod tags;
pub mod trash;
pub mod vault;
pub mod watcher;

pub use frontmatter::NoteType;
pub use note::Note;
pub use vault::Vault;
pub use watcher::VaultWatcher;
