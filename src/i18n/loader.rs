//! Loads `.ftl` Fluent resource files per locale (§3.6.2). Callers:
//! `i18n::locale_manager`.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{anyhow, Result};
use fluent_bundle::{FluentBundle, FluentResource};
use unic_langid::LanguageIdentifier;

/// A locale's compiled Fluent bundle, ready for message lookups.
pub type Bundle = FluentBundle<FluentResource>;

/// Load every `.ftl` file under `locales_dir/<locale>/` into one bundle
/// for that locale. Returns an error only if the locale directory itself
/// can't be read; a single malformed `.ftl` file is skipped with a log
/// warning rather than failing the whole load.
pub fn load_locale(locales_dir: &Path, locale: &str) -> Result<Bundle> {
    let lang_id: LanguageIdentifier = locale
        .parse()
        .map_err(|e| anyhow!("invalid locale id '{locale}': {e}"))?;
    let mut bundle = FluentBundle::new(vec![lang_id]);
    // Disable BiDi isolation marks around interpolated values (U+2068/U+2069):
    // egui renders plain text and doesn't need them, and they'd otherwise
    // show up as stray characters/width in the UI.
    bundle.set_use_isolating(false);

    let dir = locales_dir.join(locale);
    let entries = std::fs::read_dir(&dir)
        .map_err(|e| anyhow!("reading locale dir {}: {e}", dir.display()))?;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                log::warn!("i18n: skipping unreadable entry in {}: {e}", dir.display());
                continue;
            }
        };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ftl") {
            continue;
        }

        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("i18n: failed to read {}: {e}", path.display());
                continue;
            }
        };

        let resource = match FluentResource::try_new(raw) {
            Ok(r) => r,
            Err((_, errors)) => {
                log::warn!("i18n: failed to parse {}: {errors:?}", path.display());
                continue;
            }
        };

        if let Err(errors) = bundle.add_resource(resource) {
            log::warn!("i18n: failed to add resource {}: {errors:?}", path.display());
        }
    }

    Ok(bundle)
}

/// Load all locales found as subdirectories of `locales_dir` into a map
/// keyed by locale id (e.g. `"id-ID"`).
pub fn load_all(locales_dir: &Path) -> HashMap<String, Bundle> {
    let mut bundles = HashMap::new();
    let Ok(entries) = std::fs::read_dir(locales_dir) else {
        log::warn!("i18n: locales dir {} not found", locales_dir.display());
        return bundles;
    };

    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let Some(locale) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        match load_locale(locales_dir, &locale) {
            Ok(bundle) => {
                bundles.insert(locale, bundle);
            }
            Err(e) => log::warn!("i18n: failed to load locale '{locale}': {e}"),
        }
    }

    bundles
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
    fn loads_bundle_and_resolves_message() {
        let dir = tempdir().unwrap();
        write_ftl(dir.path(), "id-ID", "greeting = Halo, dunia!\n");

        let bundle = load_locale(dir.path(), "id-ID").unwrap();
        let msg = bundle.get_message("greeting").unwrap();
        let pattern = msg.value().unwrap();
        let mut errors = vec![];
        let value = bundle.format_pattern(pattern, None, &mut errors);
        assert_eq!(value, "Halo, dunia!");
        assert!(errors.is_empty());
    }

    #[test]
    fn load_all_finds_every_locale_subdir() {
        let dir = tempdir().unwrap();
        write_ftl(dir.path(), "id-ID", "hello = Halo\n");
        write_ftl(dir.path(), "en-US", "hello = Hello\n");

        let bundles = load_all(dir.path());
        assert_eq!(bundles.len(), 2);
        assert!(bundles.contains_key("id-ID"));
        assert!(bundles.contains_key("en-US"));
    }
}
