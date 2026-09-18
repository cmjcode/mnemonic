//! Daily notes and templates (§Fase 1.5, Obsidian core plugins):
//! `Daily/YYYY-MM-DD.md` created from `Templates/Daily.md` when present,
//! and `{{date}}`/`{{time}}`/`{{title}}` placeholders (with optional
//! `{{date:FORMAT}}` using `chrono` strftime) expanded when a template is
//! inserted. Pure string/path logic; the app does the IO. Callers: `app`.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};

/// Folder (under the vault root) where templates live.
pub const TEMPLATES_DIR: &str = "Templates";
/// Folder (under the vault root) where daily notes live.
pub const DAILY_DIR: &str = "Daily";
/// Template file used for daily notes, if it exists.
pub const DAILY_TEMPLATE: &str = "Daily.md";
/// Obsidian's default daily-note file name format.
pub const DAILY_FORMAT: &str = "%Y-%m-%d";

/// `<root>/Daily/<YYYY-MM-DD>.md` for `now`.
pub fn daily_note_path(root: &Path, now: DateTime<Local>) -> PathBuf {
    root.join(DAILY_DIR)
        .join(format!("{}.md", now.format(DAILY_FORMAT)))
}

/// Title of the daily note for `now` (same as its file stem).
pub fn daily_note_title(now: DateTime<Local>) -> String {
    now.format(DAILY_FORMAT).to_string()
}

/// Expands `{{title}}`, `{{date}}`, `{{time}}`, `{{date:FMT}}` and
/// `{{time:FMT}}` in `template`. Unknown placeholders are left as-is.
pub fn expand(template: &str, title: &str, now: DateTime<Local>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let inner = after[..end].trim();
        let (key, fmt) = match inner.split_once(':') {
            Some((k, f)) => (k.trim(), Some(f.trim())),
            None => (inner, None),
        };
        let replacement = match key.to_ascii_lowercase().as_str() {
            "title" => Some(title.to_string()),
            "date" => Some(now.format(fmt.unwrap_or("%Y-%m-%d")).to_string()),
            "time" => Some(now.format(fmt.unwrap_or("%H:%M")).to_string()),
            _ => None,
        };
        match replacement {
            Some(r) => out.push_str(&r),
            None => out.push_str(&rest[start..start + 2 + end + 2]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// `.md` files in `<root>/Templates`, sorted by name: `(name, path)`.
pub fn list_templates(root: &Path) -> Vec<(String, PathBuf)> {
    let dir = root.join(TEMPLATES_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "md"))
        .filter_map(|p| {
            let name = p.file_stem()?.to_string_lossy().to_string();
            Some((name, p))
        })
        .collect();
    out.sort_by_key(|(name, _)| name.to_lowercase());
    out
}

/// Body of the daily-note template (`Templates/Daily.md`), expanded, or a
/// minimal default heading when there is none. Frontmatter in the
/// template file is dropped — the note gets its own.
pub fn daily_note_body(root: &Path, now: DateTime<Local>) -> String {
    let title = daily_note_title(now);
    let path = root.join(TEMPLATES_DIR).join(DAILY_TEMPLATE);
    match std::fs::read_to_string(&path) {
        Ok(raw) => {
            let (_, body) = super::frontmatter::parse(&raw);
            expand(&body, &title, now)
        }
        Err(_) => format!("# {}\n\n", now.format("%A, %-d %B %Y")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 9, 18, 7, 5, 0).unwrap()
    }

    #[test]
    fn expands_known_placeholders_and_keeps_unknown_ones() {
        let out = expand("# {{title}} — {{date}} {{time}} {{date:%d/%m}} {{nope}} {{", "Harian", at());
        assert_eq!(out, "# Harian — 2026-09-18 07:05 18/09 {{nope}} {{");
    }

    #[test]
    fn daily_paths_follow_obsidian_defaults() {
        let root = Path::new("/vault");
        assert_eq!(daily_note_path(root, at()), PathBuf::from("/vault/Daily/2026-09-18.md"));
        assert_eq!(daily_note_title(at()), "2026-09-18");
    }

    #[test]
    fn daily_body_uses_template_when_present() {
        let dir = tempfile::tempdir().unwrap();
        assert!(daily_note_body(dir.path(), at()).starts_with("# "));
        std::fs::create_dir_all(dir.path().join(TEMPLATES_DIR)).unwrap();
        std::fs::write(
            dir.path().join(TEMPLATES_DIR).join(DAILY_TEMPLATE),
            "---\ntags: [harian]\n---\n## {{date}}\n- [ ] rencana\n",
        )
        .unwrap();
        assert_eq!(daily_note_body(dir.path(), at()), "## 2026-09-18\n- [ ] rencana\n");
        assert_eq!(list_templates(dir.path()).len(), 1);
    }
}
