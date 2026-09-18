//! Persisted user preferences (`<config_dir>/mnemonic/config.toml`): the
//! last opened vault, recently opened vaults, theme, language, and a few
//! layout toggles — so the app reopens exactly the way the user left it.
//! Callers: `notes::vault` (remembers the vault path on open) and
//! `app.rs` (loads on startup, saves whenever a preference changes).
//!
//! Every field is `#[serde(default)]` so an older config file (which only
//! had `vault_path`) still loads, and a corrupt file degrades to defaults
//! instead of failing startup.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// How many vaults the welcome screen's "recent" list remembers.
pub const MAX_RECENT_VAULTS: usize = 6;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub vault_path: Option<String>,
    pub recent_vaults: Vec<String>,
    /// `"dark"` or `"light"`.
    pub theme: String,
    /// Active locale id, e.g. `"id-ID"`. `None` means the app default.
    pub locale: Option<String>,
    pub sidebar_open: bool,
    pub show_outline: bool,
    /// Reorder search results with the local cross-encoder reranker. Off
    /// by default: it downloads an extra model and adds latency.
    pub rerank_search: bool,
    /// `[hotkeys]`: action id → chord (`"Cmd+Shift+K"`), overriding the
    /// defaults in `app::hotkeys::DEFAULT_HOTKEYS` (§Fase 1.8).
    pub hotkeys: BTreeMap<String, String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            vault_path: None,
            recent_vaults: Vec::new(),
            theme: "dark".to_string(),
            locale: None,
            sidebar_open: true,
            show_outline: true,
            rerank_search: false,
            hotkeys: BTreeMap::new(),
        }
    }
}

impl AppSettings {
    /// The chord configured for `action`, or `default` when unset/empty.
    pub fn chord<'a>(&'a self, action: &str, default: &'a str) -> &'a str {
        match self.hotkeys.get(action) {
            Some(c) if !c.trim().is_empty() => c.as_str(),
            _ => default,
        }
    }

    /// Records `root` as the current vault and moves it to the front of
    /// the recent list (deduplicated, capped at `MAX_RECENT_VAULTS`).
    pub fn remember_vault(&mut self, root: &Path) {
        let path = root.to_string_lossy().to_string();
        self.recent_vaults.retain(|p| p != &path);
        self.recent_vaults.insert(0, path.clone());
        self.recent_vaults.truncate(MAX_RECENT_VAULTS);
        self.vault_path = Some(path);
    }

    /// Recent vaults that still exist on disk.
    pub fn existing_recent_vaults(&self) -> Vec<PathBuf> {
        self.recent_vaults
            .iter()
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .collect()
    }
}

fn config_path() -> Result<PathBuf> {
    let dir = dirs::config_dir().context("no config dir available on this platform")?;
    let dir = dir.join("mnemonic");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating config dir {}", dir.display()))?;
    Ok(dir.join("config.toml"))
}

/// Loads settings from disk, falling back to defaults when the file is
/// missing or unreadable.
pub fn load() -> AppSettings {
    let Ok(path) = config_path() else {
        return AppSettings::default();
    };
    load_from(&path)
}

pub fn save(settings: &AppSettings) -> Result<()> {
    save_to(&config_path()?, settings)
}

fn load_from(path: &Path) -> AppSettings {
    match std::fs::read_to_string(path) {
        Ok(raw) => toml::from_str(&raw).unwrap_or_else(|e| {
            log::warn!(
                "settings: ignoring unreadable config {}: {e}",
                path.display()
            );
            AppSettings::default()
        }),
        Err(_) => AppSettings::default(),
    }
}

fn save_to(path: &Path, settings: &AppSettings) -> Result<()> {
    let raw = toml::to_string_pretty(settings)?;
    std::fs::write(path, raw).with_context(|| format!("writing config file {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn round_trips_through_toml() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut settings = AppSettings {
            theme: "light".to_string(),
            locale: Some("en-US".to_string()),
            ..Default::default()
        };
        settings.remember_vault(dir.path());

        save_to(&path, &settings).unwrap();
        assert_eq!(load_from(&path), settings);
    }

    #[test]
    fn legacy_config_with_only_vault_path_still_loads() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "vault_path = \"/tmp/vault\"\n").unwrap();

        let settings = load_from(&path);
        assert_eq!(settings.vault_path.as_deref(), Some("/tmp/vault"));
        assert_eq!(settings.theme, "dark");
        assert!(settings.sidebar_open);
    }

    #[test]
    fn corrupt_config_falls_back_to_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "this is = = not toml").unwrap();
        assert_eq!(load_from(&path), AppSettings::default());
    }

    #[test]
    fn remember_vault_dedupes_and_caps_recent_list() {
        let mut settings = AppSettings::default();
        for i in 0..10 {
            settings.remember_vault(Path::new(&format!("/v{i}")));
        }
        settings.remember_vault(Path::new("/v3"));
        assert_eq!(settings.recent_vaults.len(), MAX_RECENT_VAULTS);
        assert_eq!(settings.recent_vaults[0], "/v3");
        assert_eq!(
            settings
                .recent_vaults
                .iter()
                .filter(|p| *p == "/v3")
                .count(),
            1
        );
        assert_eq!(settings.vault_path.as_deref(), Some("/v3"));
    }
}
