//! Notes module: vault management, note CRUD, frontmatter, file watching,
//! and trash retention (§3.1 in pengembangan.md). Callers: `app.rs`.

pub mod frontmatter;
pub mod note;
pub mod trash;
pub mod vault;
pub mod watcher;

pub use note::Note;
pub use vault::Vault;
pub use watcher::VaultWatcher;
