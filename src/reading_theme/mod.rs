//! Reading themes (§3.2.3, §3.2.5): the colours of a rendered note —
//! headings, links, code, quotes, tables, callouts — in the Live editor
//! *and* in HTML/PDF export and print, so a printed note looks like it
//! does on screen.
//!
//! A theme is a declarative TOML file (no code runs, so themes are safe to
//! share): `name`, optional `author`/`description`/`base`, and `[light]`,
//! `[dark]` and optional `[print]` colour tables. Any colour left out is
//! inherited from `base` (default: the built-in `mnemonic` theme); `[print]`
//! defaults to the resolved `[light]` colours. Built-ins ship in `themes/`;
//! user plugins are `*.toml` files in `<config_dir>/mnemonic/themes/` or
//! `<vault>/.mnemonic/themes/` (see `theme_dirs`, `docs/themes.md`). A
//! broken file is reported and skipped, never fatal.
//!
//! Pure and egui-free apart from the `Color` → `egui::Color32` conversion.
//! Callers: `markdown::renderer` (Live view), `export` (HTML/PDF),
//! `app` (theme picker), `api` (CLI/MCP `themes list`, `notes export`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// Theme used when none is chosen or the chosen one is missing.
pub const DEFAULT_THEME: &str = "mnemonic";

/// Frontmatter key that picks a theme for one note (`theme: pelangi`).
pub const FRONTMATTER_KEY: &str = "theme";

/// An sRGB colour with alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    /// Parses `#rgb`, `#rrggbb` or `#rrggbbaa` (the `#` is optional).
    pub fn parse(s: &str) -> Option<Color> {
        let hex = s.trim().trim_start_matches('#');
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        match hex.len() {
            3 => {
                let nib = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok().map(|v| v * 17);
                Some(Color::rgb(nib(0)?, nib(1)?, nib(2)?))
            }
            6 => Some(Color::rgb(byte(0)?, byte(2)?, byte(4)?)),
            8 => Some(Color { r: byte(0)?, g: byte(2)?, b: byte(4)?, a: byte(6)? }),
            _ => None,
        }
    }

    /// `#rrggbb`, or `#rrggbbaa` when not opaque.
    pub fn hex(self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }
}

impl From<Color> for egui::Color32 {
    fn from(c: Color) -> egui::Color32 {
        egui::Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
    }
}

/// Declares `ThemeColors` (every field required) and its partial TOML
/// form `RawColors` from one field list, so the two can't drift apart.
macro_rules! theme_colors {
    ($($(#[$doc:meta])* $field:ident),* $(,)?) => {
        /// One fully-resolved colour set (a theme's light, dark or print variant).
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct ThemeColors {
            $($(#[$doc])* pub $field: Color,)*
        }

        /// A variant table as written in a theme file: every colour optional.
        #[derive(Debug, Default, Clone, Deserialize)]
        #[serde(default)]
        struct RawColors {
            $($field: Option<String>,)*
            /// Unknown keys, reported as warnings (typos).
            #[serde(flatten)]
            unknown: BTreeMap<String, toml::Value>,
        }

        impl RawColors {
            /// `base` with every colour this table sets replaced.
            fn resolve(&self, base: &ThemeColors, section: &str, warnings: &mut Vec<String>) -> ThemeColors {
                let mut out = *base;
                $(
                    if let Some(raw) = &self.$field {
                        match Color::parse(raw) {
                            Some(c) => out.$field = c,
                            None => warnings.push(format!(
                                "[{section}] {} = \"{raw}\" is not a #rrggbb colour", stringify!($field)
                            )),
                        }
                    }
                )*
                for key in self.unknown.keys() {
                    warnings.push(format!("[{section}] unknown key `{key}`"));
                }
                out
            }

            fn has_any(&self) -> bool {
                false $(|| self.$field.is_some())*
            }
        }

        impl ThemeColors {
            /// Colour field names, as written in theme files.
            pub const FIELDS: &'static [&'static str] = &[$(stringify!($field)),*];

            /// `(field name, colour)` for every colour, in declaration order.
            pub fn entries(&self) -> Vec<(&'static str, Color)> {
                vec![$((stringify!($field), self.$field)),*]
            }
        }
    };
}

theme_colors! {
    /// Page background.
    background,
    /// Body text.
    text,
    /// Secondary text: quotes, captions.
    muted,
    /// Bold text and list bullets.
    strong,
    /// Links and wikilinks.
    link,
    /// Inline `#tags`.
    tag,
    h1, h2, h3, h4, h5, h6,
    /// Inline code and code blocks: text and background.
    code_text,
    code_bg,
    /// The bar left of a blockquote.
    quote_bar,
    /// Horizontal rules.
    rule,
    table_border,
    table_header_bg,
    /// `==highlight==` background.
    highlight_bg,
    /// Checked checkboxes.
    checkbox,
    /// Callout accents (`> [!note]`, …); other callout types map onto these.
    callout_note,
    callout_tip,
    callout_important,
    callout_warning,
    callout_caution,
    callout_quote,
}

impl ThemeColors {
    /// Heading colour for level 1–6.
    pub fn heading(&self, level: u8) -> Color {
        match level {
            1 => self.h1,
            2 => self.h2,
            3 => self.h3,
            4 => self.h4,
            5 => self.h5,
            _ => self.h6,
        }
    }

    /// Accent for a callout type, following Obsidian's aliases
    /// (`info`/`todo` → note, `hint`/`success` → tip, `danger`/`bug` → caution …).
    pub fn callout(&self, kind: &str) -> Color {
        match kind.to_ascii_lowercase().as_str() {
            "tip" | "hint" | "success" | "check" | "done" => self.callout_tip,
            "important" | "example" | "abstract" | "summary" | "tldr" => self.callout_important,
            "warning" | "attention" | "question" | "help" | "faq" => self.callout_warning,
            "caution" | "danger" | "error" | "failure" | "fail" | "missing" | "bug" => self.callout_caution,
            "quote" | "cite" => self.callout_quote,
            _ => self.callout_note,
        }
    }
}

/// Built-in `mnemonic` light colours: the app's own light palette.
pub const MNEMONIC_LIGHT: ThemeColors = ThemeColors {
    background: Color::rgb(248, 248, 250),
    text: Color::rgb(29, 30, 34),
    muted: Color::rgb(88, 91, 99),
    strong: Color::rgb(29, 30, 34),
    link: Color::rgb(47, 111, 235),
    tag: Color::rgb(47, 111, 235),
    h1: Color::rgb(29, 30, 34),
    h2: Color::rgb(29, 30, 34),
    h3: Color::rgb(29, 30, 34),
    h4: Color::rgb(29, 30, 34),
    h5: Color::rgb(88, 91, 99),
    h6: Color::rgb(88, 91, 99),
    code_text: Color::rgb(29, 30, 34),
    code_bg: Color::rgb(236, 237, 241),
    quote_bar: Color::rgb(204, 206, 213),
    rule: Color::rgb(224, 225, 230),
    table_border: Color::rgb(224, 225, 230),
    table_header_bg: Color::rgb(242, 242, 245),
    highlight_bg: Color::rgb(255, 240, 150),
    checkbox: Color::rgb(47, 111, 235),
    callout_note: Color::rgb(10, 80, 210),
    callout_tip: Color::rgb(0, 130, 20),
    callout_important: Color::rgb(150, 30, 140),
    callout_warning: Color::rgb(200, 120, 0),
    callout_caution: Color::rgb(220, 0, 0),
    callout_quote: Color::rgb(120, 124, 134),
};

/// Built-in `mnemonic` dark colours: the app's own dark palette.
pub const MNEMONIC_DARK: ThemeColors = ThemeColors {
    background: Color::rgb(22, 23, 26),
    text: Color::rgb(236, 236, 238),
    muted: Color::rgb(163, 166, 174),
    strong: Color::rgb(236, 236, 238),
    link: Color::rgb(98, 148, 250),
    tag: Color::rgb(98, 148, 250),
    h1: Color::rgb(236, 236, 238),
    h2: Color::rgb(236, 236, 238),
    h3: Color::rgb(236, 236, 238),
    h4: Color::rgb(236, 236, 238),
    h5: Color::rgb(163, 166, 174),
    h6: Color::rgb(163, 166, 174),
    code_text: Color::rgb(236, 236, 238),
    code_bg: Color::rgb(35, 36, 41),
    quote_bar: Color::rgb(62, 65, 73),
    rule: Color::rgb(46, 48, 54),
    table_border: Color::rgb(46, 48, 54),
    table_header_bg: Color::rgb(28, 29, 33),
    highlight_bg: Color::rgb(92, 79, 26),
    checkbox: Color::rgb(98, 148, 250),
    callout_note: Color::rgb(88, 150, 255),
    callout_tip: Color::rgb(63, 185, 123),
    callout_important: Color::rgb(200, 120, 230),
    callout_warning: Color::rgb(232, 169, 58),
    callout_caution: Color::rgb(240, 97, 109),
    callout_quote: Color::rgb(140, 145, 158),
};

/// Where a theme came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeSource {
    BuiltIn,
    File(PathBuf),
}

/// A loaded, fully resolved reading theme.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadingTheme {
    /// File stem (`pelangi` for `pelangi.toml`), lower-cased: what settings
    /// and frontmatter refer to.
    pub id: String,
    pub name: String,
    pub author: Option<String>,
    pub description: Option<String>,
    pub light: ThemeColors,
    pub dark: ThemeColors,
    /// Used for print and HTML/PDF export.
    pub print: ThemeColors,
    pub source: ThemeSource,
}

impl ReadingTheme {
    /// The built-in default theme.
    pub fn mnemonic() -> ReadingTheme {
        ReadingTheme {
            id: DEFAULT_THEME.into(),
            name: "Mnemonic".into(),
            author: Some("MNEMONIC".into()),
            description: Some("Calm, neutral colours matching the app.".into()),
            light: MNEMONIC_LIGHT,
            dark: MNEMONIC_DARK,
            print: MNEMONIC_LIGHT,
            source: ThemeSource::BuiltIn,
        }
    }

    /// The on-screen colours for light or dark mode.
    pub fn colors(&self, dark: bool) -> &ThemeColors {
        if dark { &self.dark } else { &self.light }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ThemeFile {
    name: Option<String>,
    author: Option<String>,
    description: Option<String>,
    /// Id of an already-loaded theme to inherit unset colours from.
    base: Option<String>,
    light: RawColors,
    dark: RawColors,
    print: Option<RawColors>,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

/// Parses one theme file. `lookup` resolves `base = "…"`; `id` is the file
/// stem. Returns the theme plus non-fatal warnings (bad colours, unknown
/// keys), or an error when the TOML itself is invalid.
pub fn parse_theme(
    id: &str,
    raw: &str,
    source: ThemeSource,
    lookup: &dyn Fn(&str) -> Option<ReadingTheme>,
) -> Result<(ReadingTheme, Vec<String>)> {
    let file: ThemeFile = toml::from_str(raw).context("invalid theme TOML")?;
    let mut warnings: Vec<String> = file.unknown.keys().map(|k| format!("unknown key `{k}`")).collect();
    let base = match file.base.as_deref() {
        Some(b) if b.eq_ignore_ascii_case(id) => bail!("theme `{id}` cannot use itself as base"),
        Some(b) => lookup(b).unwrap_or_else(|| {
            warnings.push(format!("base theme `{b}` not found, using `{DEFAULT_THEME}`"));
            ReadingTheme::mnemonic()
        }),
        None => ReadingTheme::mnemonic(),
    };
    let light = file.light.resolve(&base.light, "light", &mut warnings);
    let dark = file.dark.resolve(&base.dark, "dark", &mut warnings);
    // Without its own [print], a theme prints with its light colours — or
    // the base's print colours when it only restyles dark mode.
    let print_base = if file.light.has_any() { light } else { base.print };
    let print = match &file.print {
        Some(p) => p.resolve(&print_base, "print", &mut warnings),
        None => print_base,
    };
    Ok((
        ReadingTheme {
            id: id.to_string(),
            name: file.name.unwrap_or_else(|| id.to_string()),
            author: file.author,
            description: file.description,
            light,
            dark,
            print,
            source,
        },
        warnings,
    ))
}

/// Built-in themes other than `mnemonic`, as `(id, toml)`. They double as
/// examples for plugin authors.
const BUILT_IN: &[(&str, &str)] = &[
    ("pelangi", include_str!("../../themes/pelangi.toml")),
    ("ocean", include_str!("../../themes/ocean.toml")),
    ("sunset", include_str!("../../themes/sunset.toml")),
    ("forest", include_str!("../../themes/forest.toml")),
    ("print-classic", include_str!("../../themes/print-classic.toml")),
];

/// Every available theme: built-ins, then plugins (a plugin with a
/// built-in's id replaces it).
#[derive(Debug, Clone)]
pub struct ThemeRegistry {
    themes: Vec<ReadingTheme>,
    /// Files that failed to load or loaded with warnings, as
    /// `"<file>: <message>"` — shown to the user, never fatal.
    pub problems: Vec<String>,
}

impl Default for ThemeRegistry {
    fn default() -> Self {
        ThemeRegistry::load(&[])
    }
}

impl ThemeRegistry {
    /// Built-ins plus every `*.toml` in `dirs` (missing dirs are skipped;
    /// later dirs override earlier ones by id).
    pub fn load(dirs: &[PathBuf]) -> ThemeRegistry {
        let mut reg = ThemeRegistry { themes: vec![ReadingTheme::mnemonic()], problems: Vec::new() };
        for (id, raw) in BUILT_IN {
            if let Err(e) = reg.parse_into(id, raw, ThemeSource::BuiltIn) {
                log::warn!("reading_theme: built-in `{id}` is broken: {e:#}");
            }
        }
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            let mut files: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("toml")))
                .collect();
            files.sort();
            for path in files {
                if let Err(e) = reg.load_file(&path) {
                    log::warn!("reading_theme: {}: {e:#}", path.display());
                    reg.problems.push(format!("{}: {e:#}", path.display()));
                }
            }
        }
        reg
    }

    fn load_file(&mut self, path: &Path) -> Result<()> {
        let id = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .filter(|s| !s.is_empty())
            .context("theme file has no name")?;
        let raw = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        self.parse_into(&id, &raw, ThemeSource::File(path.to_path_buf()))
    }

    fn parse_into(&mut self, id: &str, raw: &str, source: ThemeSource) -> Result<()> {
        let label = match &source {
            ThemeSource::File(p) => p.display().to_string(),
            ThemeSource::BuiltIn => format!("built-in {id}"),
        };
        let (theme, warnings) = parse_theme(id, raw, source, &|base| self.find(base).cloned())?;
        for w in warnings {
            log::warn!("reading_theme: {label}: {w}");
            self.problems.push(format!("{label}: {w}"));
        }
        match self.themes.iter_mut().find(|t| t.id == theme.id) {
            Some(existing) => *existing = theme,
            None => self.themes.push(theme),
        }
        Ok(())
    }

    fn find(&self, id: &str) -> Option<&ReadingTheme> {
        self.themes.iter().find(|t| t.id.eq_ignore_ascii_case(id.trim()))
    }

    /// The theme `id`, or the default theme when it doesn't exist.
    pub fn get(&self, id: &str) -> &ReadingTheme {
        self.find(id).unwrap_or(&self.themes[0])
    }

    /// Whether a theme with this id is loaded.
    pub fn contains(&self, id: &str) -> bool {
        self.find(id).is_some()
    }

    pub fn list(&self) -> &[ReadingTheme] {
        &self.themes
    }
}

/// Folders searched for theme plugins: the per-user config folder, then the
/// vault's own (so a vault can carry its themes; it wins on equal ids).
pub fn theme_dirs(vault_root: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(config) = dirs::config_dir() {
        dirs.push(config.join("mnemonic").join("themes"));
    }
    if let Some(root) = vault_root {
        dirs.push(root.join(".mnemonic").join("themes"));
    }
    dirs
}

/// The theme a note asks for in its frontmatter (`theme: ocean`), if any.
pub fn note_theme(extra: &BTreeMap<String, serde_yaml::Value>) -> Option<&str> {
    extra.get(FRONTMATTER_KEY).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_parse_and_print() {
        assert_eq!(Color::parse("#fa0"), Some(Color::rgb(255, 170, 0)));
        assert_eq!(Color::parse("1e88e5").map(Color::hex).as_deref(), Some("#1e88e5"));
        assert_eq!(Color::parse("#11223380").map(|c| c.a), Some(0x80));
        assert_eq!(Color::parse("#12345"), None);
        assert_eq!(Color::parse("red"), None);
    }

    #[test]
    fn built_ins_load_without_warnings() {
        let reg = ThemeRegistry::load(&[]);
        assert!(reg.problems.is_empty(), "{:?}", reg.problems);
        for id in ["mnemonic", "pelangi", "ocean", "sunset", "forest", "print-classic"] {
            assert!(reg.contains(id), "missing {id}");
        }
        assert_eq!(reg.get("tidak-ada").id, DEFAULT_THEME);
        // Pelangi really is colourful: every heading level differs.
        let p = &reg.get("pelangi").light;
        let hs: std::collections::HashSet<_> = (1..=6).map(|l| p.heading(l)).collect();
        assert_eq!(hs.len(), 6);
    }

    #[test]
    fn partial_theme_inherits_and_reports_mistakes() {
        let raw = "name = \"Coba\"\nwarna = 1\n[light]\nh1 = \"#ff0000\"\nlink = \"biru\"\nh7 = \"#000\"\n[print]\nbackground = \"#ffffff\"\n";
        let (t, warnings) = parse_theme("coba", raw, ThemeSource::BuiltIn, &|_| None).unwrap();
        assert_eq!(t.name, "Coba");
        assert_eq!(t.light.h1, Color::rgb(255, 0, 0));
        assert_eq!(t.light.link, MNEMONIC_LIGHT.link, "bad colour keeps the base value");
        assert_eq!(t.dark, MNEMONIC_DARK, "no [dark] table: inherited");
        assert_eq!(t.print.h1, Color::rgb(255, 0, 0), "print starts from light");
        assert_eq!(t.print.background, Color::rgb(255, 255, 255));
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(parse_theme("x", "light = 3", ThemeSource::BuiltIn, &|_| None).is_err());
        assert!(parse_theme("x", "base = \"X\"", ThemeSource::BuiltIn, &|_| None).is_err());
    }

    #[test]
    fn plugin_files_override_and_broken_ones_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Kopi.toml"), "name = \"Kopi\"\nbase = \"sunset\"\n[dark]\nh2 = \"#abcdef\"\n").unwrap();
        std::fs::write(dir.path().join("rusak.toml"), "[light\n").unwrap();
        std::fs::write(dir.path().join("abaikan.txt"), "x").unwrap();
        let reg = ThemeRegistry::load(&[dir.path().to_path_buf(), dir.path().join("tidak-ada")]);
        let kopi = reg.get("kopi");
        assert_eq!(kopi.name, "Kopi");
        assert_eq!(kopi.dark.h2, Color::rgb(0xab, 0xcd, 0xef));
        assert_eq!(kopi.light, reg.get("sunset").light, "inherits its base");
        assert_eq!(kopi.print, reg.get("sunset").print, "only dark changed: base print kept");
        assert!(!reg.contains("rusak"));
        assert_eq!(reg.problems.len(), 1, "{:?}", reg.problems);
        assert!(reg.problems[0].contains("rusak.toml"));
    }

    #[test]
    fn callout_aliases_and_frontmatter_override() {
        let c = MNEMONIC_LIGHT;
        assert_eq!(c.callout("DANGER"), c.callout_caution);
        assert_eq!(c.callout("info"), c.callout_note);
        let mut extra = BTreeMap::new();
        extra.insert("theme".to_string(), serde_yaml::Value::String(" ocean ".into()));
        assert_eq!(note_theme(&extra), Some("ocean"));
    }
}
