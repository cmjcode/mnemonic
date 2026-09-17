//! Trash retention policy: permanently delete notes that have sat in
//! `.trash/` longer than the retention window (§3.1.4). Callers: `app.rs`
//! (invoked periodically, e.g. once per startup).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};

/// Default retention window before a trashed note is permanently deleted.
pub const DEFAULT_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60); // 30 days

/// A path inside `vault_root/.trash` for `file_name` that doesn't collide
/// with anything already there (`name.md`, `name (2).md`, ...), so
/// trashing two files with the same name never overwrites the first.
pub fn unique_trash_path(vault_root: &Path, file_name: &std::ffi::OsStr) -> Result<PathBuf> {
    let trash_dir = vault_root.join(".trash");
    std::fs::create_dir_all(&trash_dir)
        .with_context(|| format!("creating trash dir {}", trash_dir.display()))?;
    Ok(unique_path_in(&trash_dir, file_name))
}

/// `dir/file_name`, or `dir/stem (n).ext` for the first free `n` if that
/// already exists.
pub fn unique_path_in(dir: &Path, file_name: &std::ffi::OsStr) -> PathBuf {
    let candidate = dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }
    let name = Path::new(file_name);
    let stem = name
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = name.extension().map(|e| e.to_string_lossy().to_string());
    (2..)
        .map(|n| match &ext {
            Some(ext) => dir.join(format!("{stem} ({n}).{ext}")),
            None => dir.join(format!("{stem} ({n})")),
        })
        .find(|p| !p.exists())
        .expect("unbounded range always yields a free path")
}

/// Moves any file or folder (PDF, whole folder, ...) into `.trash/`
/// instead of deleting it, returning where it landed so the caller can
/// offer an undo. Notes should use `Note::move_to_trash` instead, which
/// also flips the `trashed` frontmatter flag.
pub fn move_path_to_trash(vault_root: &Path, path: &Path) -> Result<PathBuf> {
    let file_name = path.file_name().context("path has no file name")?;
    let target = unique_trash_path(vault_root, file_name)?;
    std::fs::rename(path, &target)
        .with_context(|| format!("moving {} to trash", path.display()))?;
    Ok(target)
}

/// Undo for `move_path_to_trash`: moves `trashed` back to `original`,
/// refusing to overwrite something that has since appeared there.
pub fn restore_path(trashed: &Path, original: &Path) -> Result<()> {
    anyhow::ensure!(
        !original.exists(),
        "{} already exists; not overwriting it",
        original.display()
    );
    if let Some(parent) = original.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("recreating folder {}", parent.display()))?;
    }
    std::fs::rename(trashed, original)
        .with_context(|| format!("restoring {} from trash", original.display()))
}

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
    fn move_path_to_trash_avoids_collisions_and_restores() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("doc.pdf"), "first").unwrap();
        std::fs::write(b.join("doc.pdf"), "second").unwrap();

        let t1 = move_path_to_trash(dir.path(), &a.join("doc.pdf")).unwrap();
        let t2 = move_path_to_trash(dir.path(), &b.join("doc.pdf")).unwrap();
        assert_ne!(t1, t2);
        assert_eq!(t2.file_name().unwrap(), "doc (2).pdf");
        assert_eq!(std::fs::read_to_string(&t1).unwrap(), "first");

        restore_path(&t2, &b.join("doc.pdf")).unwrap();
        assert_eq!(std::fs::read_to_string(b.join("doc.pdf")).unwrap(), "second");
        assert!(restore_path(&t1, &b.join("doc.pdf")).is_err());
    }

    #[test]
    fn move_whole_folder_to_trash() {
        let dir = tempdir().unwrap();
        let folder = dir.path().join("Projects");
        std::fs::create_dir_all(&folder).unwrap();
        Note::create(&folder, "Inside", "isi").unwrap();

        let trashed = move_path_to_trash(dir.path(), &folder).unwrap();
        assert!(!folder.exists());
        assert!(trashed.is_dir());
        restore_path(&trashed, &folder).unwrap();
        assert!(folder.is_dir());
    }

    #[test]
    fn no_trash_dir_is_a_noop() {
        let dir = tempdir().unwrap();
        let deleted = purge_expired(dir.path(), DEFAULT_RETENTION).unwrap();
        assert_eq!(deleted, 0);
    }
}
