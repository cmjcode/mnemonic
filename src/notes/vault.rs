//! Vault selection & initial scan. A "vault" is a user-chosen folder on
//! disk containing `.md` note files (§3.1.1). Callers: `app.rs`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::note::Note;

/// Persisted app config — currently just remembers the last opened vault
/// path so the user doesn't have to re-pick it on every launch.
#[derive(Debug, Default, Serialize, Deserialize)]
struct AppConfig {
    vault_path: Option<String>,
}

fn config_path() -> Result<PathBuf> {
    let dir = dirs::config_dir().context("no config dir available on this platform")?;
    let dir = dir.join("lontar");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating config dir {}", dir.display()))?;
    Ok(dir.join("config.toml"))
}

fn load_config() -> AppConfig {
    let Ok(path) = config_path() else {
        return AppConfig::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(raw) => toml::from_str(&raw).unwrap_or_default(),
        Err(_) => AppConfig::default(),
    }
}

fn save_config(cfg: &AppConfig) -> Result<()> {
    let path = config_path()?;
    let raw = toml::to_string_pretty(cfg)?;
    std::fs::write(&path, raw)
        .with_context(|| format!("writing config file {}", path.display()))?;
    Ok(())
}

/// An open vault: a root folder plus the notes currently loaded from it.
pub struct Vault {
    pub root: PathBuf,
    pub notes: Vec<Note>,
}

impl Vault {
    /// Load the previously remembered vault path, if any, and scan it.
    pub fn load_last() -> Option<Result<Vault>> {
        let cfg = load_config();
        cfg.vault_path.map(|p| Vault::open(PathBuf::from(p)))
    }

    /// Open (or create) a vault at `root`, remember it for next launch,
    /// and perform an initial recursive scan for `.md` files.
    pub fn open(root: PathBuf) -> Result<Vault> {
        std::fs::create_dir_all(&root)
            .with_context(|| format!("creating vault root {}", root.display()))?;

        save_config(&AppConfig {
            vault_path: Some(root.to_string_lossy().to_string()),
        })?;

        let notes = scan(&root)?;
        Ok(Vault { root, notes })
    }

    /// Re-scan the vault root from disk, refreshing `self.notes`.
    /// Used by the file watcher and manual refresh.
    pub fn rescan(&mut self) -> Result<()> {
        self.notes = scan(&self.root)?;
        Ok(())
    }
}

/// Recursively find all `.md` files under `root` (skipping the `.trash`
/// folder, per §3.1.4) and load them as notes. A single unreadable file
/// is logged and skipped rather than failing the whole scan.
fn scan(root: &Path) -> Result<Vec<Note>> {
    let mut notes = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".trash")
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                log::warn!("vault scan: skipping unreadable entry: {e}");
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        match Note::load(entry.path()) {
            Ok(note) => notes.push(note),
            Err(e) => log::warn!("vault scan: failed to load {}: {e}", entry.path().display()),
        }
    }
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn scan_finds_only_markdown_files_and_skips_trash() {
        let dir = tempdir().unwrap();
        Note::create(dir.path(), "Note A", "isi a").unwrap();
        Note::create(dir.path(), "Note B", "isi b").unwrap();
        std::fs::write(dir.path().join("not_a_note.txt"), "abaikan").unwrap();

        let trash_dir = dir.path().join(".trash");
        std::fs::create_dir_all(&trash_dir).unwrap();
        Note::create(&trash_dir, "Trashed", "isi").unwrap();

        let notes = scan(dir.path()).unwrap();
        assert_eq!(notes.len(), 2);
    }
}
