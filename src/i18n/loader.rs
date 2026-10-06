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
/// Locales compiled into the binary, used when no `locales/` directory is
/// found next to the executable or working directory (e.g. a release
/// build launched from Finder), so the UI never degrades to raw keys.
pub const EMBEDDED_LOCALES: &[(&str, &str)] = &[
    ("id-ID", include_str!("../../locales/id-ID/main.ftl")),
    ("en-US", include_str!("../../locales/en-US/main.ftl")),
];

fn new_bundle(locale: &str) -> Result<Bundle> {
    let lang_id: LanguageIdentifier = locale
        .parse()
        .map_err(|e| anyhow!("invalid locale id '{locale}': {e}"))?;
    let mut bundle = FluentBundle::new(vec![lang_id]);
    // Disable BiDi isolation marks around interpolated values (U+2068/U+2069):
    // egui renders plain text and doesn't need them, and they'd otherwise
    // show up as stray characters/width in the UI.
    bundle.set_use_isolating(false);
    Ok(bundle)
}

/// Builds every `EMBEDDED_LOCALES` bundle.
pub fn load_embedded() -> HashMap<String, Bundle> {
    let mut bundles = HashMap::new();
    for (locale, source) in EMBEDDED_LOCALES {
        let Ok(mut bundle) = new_bundle(locale) else {
            continue;
        };
        match FluentResource::try_new((*source).to_string()) {
            Ok(resource) => {
                if let Err(errors) = bundle.add_resource(resource) {
                    log::warn!("i18n: embedded {locale} has duplicate messages: {errors:?}");
                }
                bundles.insert((*locale).to_string(), bundle);
            }
            Err((_, errors)) => log::warn!("i18n: embedded {locale} failed to parse: {errors:?}"),
        }
    }
    bundles
}

pub fn load_locale(locales_dir: &Path, locale: &str) -> Result<Bundle> {
    let mut bundle = new_bundle(locale)?;

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

    #[test]
    fn embedded_locales_parse_cleanly() {
        for (locale, source) in EMBEDDED_LOCALES {
            assert!(
                FluentResource::try_new((*source).to_string()).is_ok(),
                "{locale} has Fluent syntax errors"
            );
        }
        assert_eq!(load_embedded().len(), EMBEDDED_LOCALES.len());
    }

    /// Every message defined in one locale must exist in the other, so
    /// switching language never leaves a half-translated UI.
    #[test]
    fn all_locales_define_the_same_message_ids() {
        use fluent_syntax::ast::Entry;
        let ids = |source: &str| -> std::collections::BTreeSet<String> {
            let resource = fluent_syntax::parser::parse(source).unwrap_or_else(|(r, _)| r);
            resource
                .body
                .iter()
                .filter_map(|e| match e {
                    Entry::Message(m) => Some(m.id.name.to_string()),
                    _ => None,
                })
                .collect()
        };
        let (base_locale, base_source) = EMBEDDED_LOCALES[0];
        let base = ids(base_source);
        for (locale, source) in &EMBEDDED_LOCALES[1..] {
            let other = ids(source);
            let missing: Vec<_> = base.difference(&other).collect();
            let extra: Vec<_> = other.difference(&base).collect();
            assert!(
                missing.is_empty() && extra.is_empty(),
                "{locale} vs {base_locale}: missing {missing:?}, extra {extra:?}"
            );
        }
    }
}
