//! YAML frontmatter parsing & serialization for note `.md` files.
//!
//! A note file looks like:
//! ```md
//! ---
//! id: 8f3a1c2e-91b4-4d2f-9a7e-1234567890ab
//! title: Belanja Mingguan
//! type: checklist
//! created: 2026-08-20T09:15:00+07:00
//! modified: 2026-08-23T10:02:00+07:00
//! pinned: true
//! color: yellow
//! tags: [rumah, belanja]
//! aliases: [groceries]
//! archived: false
//! trashed: false
//! ---
//! body markdown here
//! ```
//!
//! Obsidian compatibility (§Fase 0 "paritas file"): every key MNEMONIC
//! doesn't know (`cssclasses`, `publish`, user properties, …) is kept
//! verbatim in `extra` and written back on save, so touching a note never
//! strips metadata another tool put there. Notes without `id`/`created`/
//! `modified` (plain Obsidian notes) still parse: the id is derived
//! deterministically from the file path so it is stable across rescans,
//! and timestamps fall back to the file's metadata.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use uuid::Uuid;

/// Note type discriminator, mirrors §3.1.2 (Text Note vs Checklist Note) & Canvas Whiteboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NoteType {
    #[default]
    Note,
    Checklist,
    Canvas,
}

impl NoteType {
    fn from_str(s: &str) -> Option<NoteType> {
        match s.trim().to_ascii_lowercase().as_str() {
            "note" => Some(NoteType::Note),
            "checklist" => Some(NoteType::Checklist),
            "canvas" => Some(NoteType::Canvas),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            NoteType::Note => "note",
            NoteType::Checklist => "checklist",
            NoteType::Canvas => "canvas",
        }
    }
}

/// Metadata stored in the YAML frontmatter block, per §3.1.1.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteFrontmatter {
    pub id: Uuid,
    pub title: String,
    pub note_type: NoteType,
    pub created: DateTime<Utc>,
    pub modified: DateTime<Utc>,
    pub pinned: bool,
    pub color: Option<String>,
    pub tags: Vec<String>,
    /// Obsidian `aliases`: alternative names a `[[link]]` may use.
    pub aliases: Vec<String>,
    pub archived: bool,
    pub trashed: bool,
    pub reminder: Option<DateTime<Utc>>,
    /// Every frontmatter key MNEMONIC doesn't model, preserved verbatim
    /// (Obsidian properties, other tools' metadata). Written back after
    /// the known keys on save.
    pub extra: BTreeMap<String, Value>,
    /// Set when the on-disk frontmatter block was not valid YAML: the raw
    /// block, written back untouched on save so a save never destroys
    /// (or duplicates) what the user had. Metadata edits made in the app
    /// are then not persisted until the block is repaired.
    pub unparsed_header: Option<String>,
}

impl Default for NoteFrontmatter {
    /// Fallback frontmatter used when a file has no/corrupt frontmatter,
    /// so a parse failure never panics — see §6 risk "Integritas Frontmatter YAML".
    fn default() -> Self {
        let now = Utc::now();
        NoteFrontmatter {
            id: Uuid::new_v4(),
            title: String::new(),
            note_type: NoteType::Note,
            created: now,
            modified: now,
            pinned: false,
            color: None,
            tags: Vec::new(),
            aliases: Vec::new(),
            archived: false,
            trashed: false,
            reminder: None,
            extra: BTreeMap::new(),
            unparsed_header: None,
        }
    }
}

/// Facts about the file a frontmatter block came from, used to fill in
/// what the block itself doesn't say (plain Obsidian notes).
#[derive(Debug, Clone, Default)]
pub struct FileHints<'a> {
    pub path: Option<&'a Path>,
    pub created: Option<DateTime<Utc>>,
    pub modified: Option<DateTime<Utc>>,
}

impl<'a> FileHints<'a> {
    /// Hints read from the file at `path` (timestamps from the filesystem).
    pub fn for_path(path: &'a Path) -> FileHints<'a> {
        let meta = std::fs::metadata(path).ok();
        let to_dt = |t: std::io::Result<std::time::SystemTime>| t.ok().map(DateTime::<Utc>::from);
        FileHints {
            path: Some(path),
            created: meta.as_ref().and_then(|m| to_dt(m.created())),
            modified: meta.as_ref().and_then(|m| to_dt(m.modified())),
        }
    }
}

/// A deterministic note id for a file that carries none in its
/// frontmatter — stable across rescans so index rows and backlinks keep
/// pointing at the same note. UUIDv5 over the (lossy) path string.
pub fn stable_id_for_path(path: &Path) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, path.to_string_lossy().as_bytes())
}

const DELIMITER: &str = "---";

/// Split raw file content into `(frontmatter, body)` without file hints.
/// Prefer `parse_with_hints` when the path is known.
pub fn parse(raw: &str) -> (NoteFrontmatter, String) {
    parse_with_hints(raw, &FileHints::default())
}

/// Split raw file content into `(frontmatter, body)`.
///
/// Never panics: a missing block yields default frontmatter and the whole
/// file as body; an unparseable block keeps the raw header in
/// `unparsed_header` (so it survives a save) and still separates the body
/// correctly.
pub fn parse_with_hints(raw: &str, hints: &FileHints<'_>) -> (NoteFrontmatter, String) {
    let Some((yaml_block, body)) = split_block(raw) else {
        let mut fm = NoteFrontmatter::default();
        apply_hints(&mut fm, hints, true, true, true);
        return (fm, raw.to_string());
    };

    match serde_yaml::from_str::<Value>(yaml_block) {
        Ok(Value::Mapping(map)) => {
            let (fm, missing) = from_mapping(map);
            let mut fm = fm;
            apply_hints(&mut fm, hints, missing.id, missing.created, missing.modified);
            (fm, body.to_string())
        }
        Ok(Value::Null) => {
            // `---\n---` — an empty block.
            let mut fm = NoteFrontmatter::default();
            apply_hints(&mut fm, hints, true, true, true);
            (fm, body.to_string())
        }
        Ok(other) => {
            log::warn!("frontmatter: block is not a mapping ({other:?}), keeping it verbatim");
            (unparsed(yaml_block, hints), body.to_string())
        }
        Err(e) => {
            log::warn!("frontmatter: YAML parse error, keeping block verbatim: {e}");
            (unparsed(yaml_block, hints), body.to_string())
        }
    }
}

fn unparsed(yaml_block: &str, hints: &FileHints<'_>) -> NoteFrontmatter {
    let mut fm = NoteFrontmatter {
        unparsed_header: Some(yaml_block.to_string()),
        ..NoteFrontmatter::default()
    };
    apply_hints(&mut fm, hints, true, true, true);
    fm
}

fn apply_hints(
    fm: &mut NoteFrontmatter,
    hints: &FileHints<'_>,
    id_missing: bool,
    created_missing: bool,
    modified_missing: bool,
) {
    if id_missing && let Some(path) = hints.path {
        fm.id = stable_id_for_path(path);
    }
    if modified_missing && let Some(m) = hints.modified {
        fm.modified = m;
    }
    if created_missing {
        // Filesystem birth time when available, else the modified time —
        // never later than `modified`.
        fm.created = hints.created.or(hints.modified).unwrap_or(fm.created);
        if fm.created > fm.modified {
            fm.created = fm.modified;
        }
    }
}

/// Splits `raw` into the YAML text between the opening `---` line and the
/// first closing `---` (or `...`) *line*, plus the body after it. Returns
/// `None` when the file doesn't start with a frontmatter block.
fn split_block(raw: &str) -> Option<(&str, &str)> {
    let first_line_end = raw.find('\n').unwrap_or(raw.len());
    if raw[..first_line_end].trim_end_matches('\r') != DELIMITER {
        return None;
    }
    if first_line_end == raw.len() {
        return None;
    }
    let rest = &raw[first_line_end + 1..];
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == DELIMITER || trimmed == "..." {
            let yaml_block = &rest[..offset];
            let body = &rest[offset + line.len()..];
            return Some((yaml_block, body));
        }
        offset += line.len();
    }
    None
}

struct Missing {
    id: bool,
    created: bool,
    modified: bool,
}

fn from_mapping(mut map: Mapping) -> (NoteFrontmatter, Missing) {
    let mut fm = NoteFrontmatter::default();
    let mut missing = Missing {
        id: true,
        created: true,
        modified: true,
    };

    if let Some(id) = take_value::<Uuid>(&mut map, "id") {
        fm.id = id;
        missing.id = false;
    }
    if let Some(title) = take_scalar_string(&mut map, "title") {
        fm.title = title;
    }
    if let Some(v) = map.remove(Value::from("type")) {
        match v.as_str().and_then(NoteType::from_str) {
            Some(t) => fm.note_type = t,
            // Somebody else's `type: article` — keep it, ours stays default.
            None => {
                fm.extra.insert("type".to_string(), v);
            }
        }
    }
    if let Some(c) = take_value::<DateTime<Utc>>(&mut map, "created") {
        fm.created = c;
        missing.created = false;
    }
    if let Some(m) = take_value::<DateTime<Utc>>(&mut map, "modified") {
        fm.modified = m;
        missing.modified = false;
    }
    if let Some(p) = take_value::<bool>(&mut map, "pinned") {
        fm.pinned = p;
    }
    if let Some(c) = take_scalar_string(&mut map, "color") {
        fm.color = Some(c).filter(|c| !c.is_empty());
    }
    fm.tags = take_string_list(&mut map, "tags")
        .into_iter()
        .map(|t| t.trim_start_matches('#').to_string())
        .filter(|t| !t.is_empty())
        .collect();
    fm.aliases = take_string_list(&mut map, "aliases");
    if let Some(a) = take_value::<bool>(&mut map, "archived") {
        fm.archived = a;
    }
    if let Some(t) = take_value::<bool>(&mut map, "trashed") {
        fm.trashed = t;
    }
    if let Some(r) = map.remove(Value::from("reminder")) {
        fm.reminder = serde_yaml::from_value::<Option<DateTime<Utc>>>(r).unwrap_or(None);
    }

    for (k, v) in map {
        let key = match k {
            Value::String(s) => s,
            other => serde_yaml::to_string(&other)
                .map(|s| s.trim().to_string())
                .unwrap_or_default(),
        };
        if !key.is_empty() {
            fm.extra.insert(key, v);
        }
    }
    (fm, missing)
}

/// Removes `key` and deserializes it as `T`; a value of the wrong shape
/// is logged and dropped rather than failing the whole block.
fn take_value<T: serde::de::DeserializeOwned>(map: &mut Mapping, key: &str) -> Option<T> {
    let v = map.remove(Value::from(key))?;
    if v.is_null() {
        return None;
    }
    match serde_yaml::from_value::<T>(v) {
        Ok(t) => Some(t),
        Err(e) => {
            log::warn!("frontmatter: ignoring `{key}` with unexpected value: {e}");
            None
        }
    }
}

/// `title: 42` or `title: Some text` both become a string.
fn take_scalar_string(map: &mut Mapping, key: &str) -> Option<String> {
    let v = map.remove(Value::from(key))?;
    scalar_to_string(&v)
}

fn scalar_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Accepts Obsidian's list forms: `[a, b]`, a block sequence, or a single
/// string `a, b` / `a b`.
fn take_string_list(map: &mut Mapping, key: &str) -> Vec<String> {
    match map.remove(Value::from(key)) {
        Some(Value::Sequence(seq)) => seq
            .iter()
            .filter_map(scalar_to_string)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(s)) => s
            .split([',', ' '])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        _ => Vec::new(),
    }
}

/// The YAML mapping written on save: known keys in a fixed order, then
/// every preserved extra key.
fn to_mapping(fm: &NoteFrontmatter) -> Mapping {
    let mut map = Mapping::new();
    map.insert("id".into(), Value::String(fm.id.to_string()));
    map.insert("title".into(), Value::String(fm.title.clone()));
    if !fm.extra.contains_key("type") {
        map.insert("type".into(), Value::String(fm.note_type.as_str().to_string()));
    }
    map.insert("created".into(), Value::String(fm.created.to_rfc3339()));
    map.insert("modified".into(), Value::String(fm.modified.to_rfc3339()));
    map.insert("pinned".into(), Value::Bool(fm.pinned));
    if let Some(color) = &fm.color {
        map.insert("color".into(), Value::String(color.clone()));
    }
    map.insert(
        "tags".into(),
        Value::Sequence(fm.tags.iter().cloned().map(Value::String).collect()),
    );
    if !fm.aliases.is_empty() {
        map.insert(
            "aliases".into(),
            Value::Sequence(fm.aliases.iter().cloned().map(Value::String).collect()),
        );
    }
    map.insert("archived".into(), Value::Bool(fm.archived));
    map.insert("trashed".into(), Value::Bool(fm.trashed));
    if let Some(r) = fm.reminder {
        map.insert("reminder".into(), Value::String(r.to_rfc3339()));
    }
    for (k, v) in &fm.extra {
        map.insert(Value::String(k.clone()), v.clone());
    }
    map
}

impl Serialize for NoteFrontmatter {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        to_mapping(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for NoteFrontmatter {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = Mapping::deserialize(deserializer)?;
        Ok(from_mapping(map).0)
    }
}

/// Serialize frontmatter + body back into full file content.
pub fn serialize(fm: &NoteFrontmatter, body: &str) -> anyhow::Result<String> {
    if let Some(raw) = &fm.unparsed_header {
        let raw = raw.strip_suffix('\n').unwrap_or(raw);
        return Ok(format!("{DELIMITER}\n{raw}\n{DELIMITER}\n{body}"));
    }
    let yaml = serde_yaml::to_string(&to_mapping(fm))?;
    Ok(format!("{DELIMITER}\n{yaml}{DELIMITER}\n{body}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let fm = NoteFrontmatter {
            title: "Belanja Mingguan".to_string(),
            tags: vec!["rumah".to_string(), "belanja".to_string()],
            pinned: true,
            ..NoteFrontmatter::default()
        };
        let body = "- [ ] Beli beras 5kg\n- [x] Bayar listrik\n";
        let raw = serialize(&fm, body).unwrap();

        let (parsed_fm, parsed_body) = parse(&raw);
        assert_eq!(parsed_fm.title, "Belanja Mingguan");
        assert_eq!(parsed_fm.tags, vec!["rumah", "belanja"]);
        assert!(parsed_fm.pinned);
        assert_eq!(parsed_fm.id, fm.id);
        assert_eq!(parsed_body, body);
    }

    #[test]
    fn corrupt_frontmatter_keeps_header_and_separates_body() {
        let raw = "---\nid: [this is not valid: yaml: at all\n---\nbody text";
        let (fm, body) = parse(raw);
        assert_eq!(fm.title, "");
        assert_eq!(body, "body text");
        assert_eq!(
            fm.unparsed_header.as_deref(),
            Some("id: [this is not valid: yaml: at all\n")
        );
        // Saving writes the broken block back untouched, never a second header.
        let out = serialize(&fm, "body text").unwrap();
        assert_eq!(out, raw);
    }

    #[test]
    fn missing_frontmatter_falls_back() {
        let raw = "just plain markdown, no frontmatter at all";
        let (fm, body) = parse(raw);
        assert_eq!(fm.title, "");
        assert_eq!(body, raw);
    }

    #[test]
    fn unknown_keys_survive_a_save() {
        let raw = "---\ntitle: Obsidian Note\naliases: [obs, note]\ncssclasses: [wide]\npublish: true\nrating: 4\n---\nhello";
        let (fm, body) = parse(raw);
        assert_eq!(fm.aliases, vec!["obs", "note"]);
        assert_eq!(fm.extra.get("publish"), Some(&Value::Bool(true)));
        assert_eq!(fm.extra.get("rating"), Some(&Value::from(4)));
        assert_eq!(body, "hello");

        let out = serialize(&fm, &body).unwrap();
        let (again, _) = parse(&out);
        assert_eq!(again.extra, fm.extra);
        assert_eq!(again.aliases, fm.aliases);
        assert_eq!(again.id, fm.id);
    }

    #[test]
    fn foreign_type_value_is_preserved_not_dropped() {
        let raw = "---\ntitle: X\ntype: article\n---\n";
        let (fm, _) = parse(raw);
        assert_eq!(fm.note_type, NoteType::Note);
        assert_eq!(fm.extra.get("type"), Some(&Value::from("article")));
        let out = serialize(&fm, "").unwrap();
        assert_eq!(out.matches("type:").count(), 1);
        assert!(out.contains("type: article"));
    }

    #[test]
    fn obsidian_note_without_ids_gets_stable_id_from_path() {
        let raw = "---\ntags: a, b\n---\nbody";
        let path = Path::new("/vault/Folder/Judul.md");
        let hints = FileHints {
            path: Some(path),
            created: None,
            modified: None,
        };
        let (a, body) = parse_with_hints(raw, &hints);
        let (b, _) = parse_with_hints(raw, &hints);
        assert_eq!(a.id, b.id);
        assert_eq!(a.id, stable_id_for_path(path));
        assert_eq!(a.tags, vec!["a", "b"]);
        assert_eq!(body, "body");
    }

    #[test]
    fn tags_accept_hash_prefix_and_block_sequences() {
        let raw = "---\ntags:\n  - '#projek'\n  - rumah/dapur\n---\n";
        let (fm, _) = parse(raw);
        assert_eq!(fm.tags, vec!["projek", "rumah/dapur"]);
    }

    #[test]
    fn closing_delimiter_must_be_a_whole_line() {
        let raw = "---\ntitle: A\nnote: |\n  ----\n  still yaml\n---\nbody";
        let (fm, body) = parse(raw);
        assert_eq!(fm.title, "A");
        assert_eq!(body, "body");
        assert!(fm.extra.contains_key("note"));
    }

    #[test]
    fn empty_block_is_not_an_error() {
        let (fm, body) = parse("---\n---\nbody");
        assert!(fm.unparsed_header.is_none());
        assert_eq!(body, "body");
    }
}
