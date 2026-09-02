//! Active-locale state, centralized lookup `t(key, args)`, and the
//! fallback chain `active -> en-US -> raw key` (§3.6.3, §3.6.4).
//! Callers: `app.rs` and all `ui/*.rs` views (once they exist).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use fluent_bundle::{FluentArgs, FluentValue};

use super::loader::{self, Bundle};

pub const DEFAULT_LOCALE: &str = "id-ID";
pub const FALLBACK_LOCALE: &str = "en-US";

/// Holds every loaded locale bundle plus which one is currently active.
pub struct LocaleManager {
    #[allow(dead_code)]
    locales_dir: PathBuf,
    bundles: HashMap<String, Bundle>,
    active: String,
}

impl LocaleManager {
    /// Load all locales under `locales_dir` and activate `DEFAULT_LOCALE`
    /// (falling back to whatever loaded first if the default is missing).
    pub fn load(locales_dir: &Path) -> LocaleManager {
        let bundles = loader::load_all(locales_dir);
        let active = if bundles.contains_key(DEFAULT_LOCALE) {
            DEFAULT_LOCALE.to_string()
        } else {
            bundles
                .keys()
                .next()
                .cloned()
                .unwrap_or_else(|| DEFAULT_LOCALE.to_string())
        };

        LocaleManager {
            locales_dir: locales_dir.to_path_buf(),
            bundles,
            active,
        }
    }

    pub fn active_locale(&self) -> &str {
        &self.active
    }

    /// Switch the active locale at runtime (§3.6.3). No-op if the locale
    /// isn't loaded.
    pub fn set_active(&mut self, locale: &str) {
        if self.bundles.contains_key(locale) {
            self.active = locale.to_string();
        } else {
            log::warn!("i18n: cannot activate unloaded locale '{locale}'");
        }
    }

    /// Central lookup used by all UI string calls: `active -> en-US ->
    /// raw key` (§3.6.4). `args` are `(name, value)` pairs for
    /// interpolation/pluralization placeholders.
    pub fn t(&self, key: &str, args: &[(&str, &str)]) -> String {
        for locale in [self.active.as_str(), FALLBACK_LOCALE] {
            if let Some(bundle) = self.bundles.get(locale) {
                if let Some(value) = resolve(bundle, key, args) {
                    return value;
                }
            }
        }
        // Nothing matched in any locale: return the raw key so missing
        // translations are visually obvious during QA (§3.6.4).
        key.to_string()
    }

    #[cfg(test)]
    fn reload(&mut self) {
        self.bundles = loader::load_all(&self.locales_dir);
    }
}

fn resolve(bundle: &Bundle, key: &str, args: &[(&str, &str)]) -> Option<String> {
    let msg = bundle.get_message(key)?;
    let pattern = msg.value()?;

    let fluent_args = if args.is_empty() {
        None
    } else {
        let mut a = FluentArgs::new();
        for (name, value) in args {
            a.set(*name, FluentValue::from(*value));
        }
        Some(a)
    };

    let mut errors = vec![];
    let value = bundle.format_pattern(pattern, fluent_args.as_ref(), &mut errors);
    if !errors.is_empty() {
        log::warn!("i18n: formatting errors for key '{key}': {errors:?}");
    }
    Some(value.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_ftl(dir: &Path, locale: &str, content: &str) {
        let locale_dir = dir.join(locale);
        std::fs::create_dir_all(&locale_dir).unwrap();
        std::fs::write(locale_dir.join("main.ftl"), content).unwrap();
    }

    #[test]
    fn defaults_to_id_id_when_available() {
        let dir = tempdir().unwrap();
        write_ftl(dir.path(), "id-ID", "hello = Halo\n");
        write_ftl(dir.path(), "en-US", "hello = Hello\n");

        let manager = LocaleManager::load(dir.path());
        assert_eq!(manager.active_locale(), "id-ID");
        assert_eq!(manager.t("hello", &[]), "Halo");
    }

    #[test]
    fn falls_back_to_en_us_when_key_missing_in_active_locale() {
        let dir = tempdir().unwrap();
        write_ftl(dir.path(), "id-ID", "only-in-id = Cuma di ID\n");
        write_ftl(dir.path(), "en-US", "hello = Hello\n");

        let manager = LocaleManager::load(dir.path());
        assert_eq!(manager.t("hello", &[]), "Hello");
    }

    #[test]
    fn falls_back_to_raw_key_when_missing_everywhere() {
        let dir = tempdir().unwrap();
        write_ftl(dir.path(), "id-ID", "hello = Halo\n");
        write_ftl(dir.path(), "en-US", "hello = Hello\n");

        let manager = LocaleManager::load(dir.path());
        assert_eq!(manager.t("does-not-exist", &[]), "does-not-exist");
    }

    #[test]
    fn runtime_switch_changes_active_locale() {
        let dir = tempdir().unwrap();
        write_ftl(dir.path(), "id-ID", "hello = Halo\n");
        write_ftl(dir.path(), "en-US", "hello = Hello\n");

        let mut manager = LocaleManager::load(dir.path());
        manager.set_active("en-US");
        assert_eq!(manager.active_locale(), "en-US");
        assert_eq!(manager.t("hello", &[]), "Hello");
    }

    #[test]
    fn interpolates_args() {
        let dir = tempdir().unwrap();
        write_ftl(dir.path(), "id-ID", "greet-name = Halo, { $name }!\n");
        write_ftl(dir.path(), "en-US", "greet-name = Hello, { $name }!\n");

        let manager = LocaleManager::load(dir.path());
        assert_eq!(manager.t("greet-name", &[("name", "Budi")]), "Halo, Budi!");
    }

    #[test]
    fn reload_picks_up_new_keys() {
        let dir = tempdir().unwrap();
        write_ftl(dir.path(), "id-ID", "hello = Halo\n");
        write_ftl(dir.path(), "en-US", "hello = Hello\n");

        let mut manager = LocaleManager::load(dir.path());
        assert_eq!(manager.t("new-key", &[]), "new-key");

        write_ftl(dir.path(), "id-ID", "hello = Halo\nnew-key = Baru\n");
        manager.reload();
        assert_eq!(manager.t("new-key", &[]), "Baru");
    }
}
