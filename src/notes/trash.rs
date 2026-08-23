//! Trash retention policy: permanently delete notes that have sat in
//! `.trash/` longer than the retention window (§3.1.4). Callers: `app.rs`
//! (invoked periodically, e.g. once per startup).

use std::path::Path;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};

/// Default retention window before a trashed note is permanently deleted.
pub const DEFAULT_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60); // 30 days

/// Scan `vault_root/.trash` and permanently remove any `.md` file whose
/// modification time is older than `retention`. Returns the number of
/// files deleted. Unreadable entries are logged and skipped, not fatal.
pub fn purge_expired(vault_root: &Path, retention: Duration) -> Result<usize> {
    let trash_dir = vault_root.join(".trash");
    if !trash_dir.exists() {
        return Ok(0);
    }

    let now = SystemTime::now();
    let mut deleted = 0;

    for entry in std::fs::read_dir(&trash_dir)
        .with_context(|| format!("reading trash dir {}", trash_dir.display()))?
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                log::warn!("trash purge: skipping unreadable entry: {e}");
                continue;
            }
        };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        let modified = match entry.metadata().and_then(|m| m.modified()) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("trash purge: no mtime for {}: {e}", path.display());
                continue;
            }
        };

        let age = now.duration_since(modified).unwrap_or(Duration::ZERO);
        if age >= retention {
            match std::fs::remove_file(&path) {
                Ok(()) => deleted += 1,
                Err(e) => log::warn!("trash purge: failed to remove {}: {e}", path.display()),
            }
        }
    }

    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::note::Note;
    use tempfile::tempdir;

    #[test]
    fn purges_only_files_older_than_retention() {
        let dir = tempdir().unwrap();
        let trash_dir = dir.path().join(".trash");
        std::fs::create_dir_all(&trash_dir).unwrap();

        let old_note = Note::create(&trash_dir, "Lama", "isi").unwrap();
        let new_note = Note::create(&trash_dir, "Baru", "isi").unwrap();

        // Backdate the "old" note's mtime to 31 days ago.
        let old_time = SystemTime::now() - Duration::from_secs(31 * 24 * 60 * 60);
        let old_ft = filetime::FileTime::from_system_time(old_time);
        filetime::set_file_mtime(&old_note.path, old_ft).unwrap();

        let deleted = purge_expired(dir.path(), DEFAULT_RETENTION).unwrap();

        assert_eq!(deleted, 1);
        assert!(!old_note.path.exists());
        assert!(new_note.path.exists());
    }

    #[test]
    fn no_trash_dir_is_a_noop() {
        let dir = tempdir().unwrap();
        let deleted = purge_expired(dir.path(), DEFAULT_RETENTION).unwrap();
        assert_eq!(deleted, 0);
    }
}
