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
//! archived: false
//! trashed: false
//! reminder: null
//! ---
//! body markdown here
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Note type discriminator, mirrors §3.1.2 (Text Note vs Checklist Note).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NoteType {
    #[default]
    Note,
    Checklist,
}

/// Metadata stored in the YAML frontmatter block, per §3.1.1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteFrontmatter {
    pub id: Uuid,
    #[serde(default)]
    pub title: String,
    #[serde(rename = "type", default)]
    pub note_type: NoteType,
    pub created: DateTime<Utc>,
    pub modified: DateTime<Utc>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub trashed: bool,
    #[serde(default)]
    pub reminder: Option<DateTime<Utc>>,
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
            archived: false,
            trashed: false,
            reminder: None,
        }
    }
}

const DELIMITER: &str = "---";

/// Split raw file content into `(frontmatter, body)`.
///
/// If parsing the frontmatter block fails or no frontmatter block is
/// present, a default frontmatter is returned and the *entire* file
/// content is treated as body — this never panics.
pub fn parse(raw: &str) -> (NoteFrontmatter, String) {
    match try_parse(raw) {
        Some((fm, body)) => (fm, body),
        None => {
            log::warn!("frontmatter: no valid block found, falling back to defaults");
            (NoteFrontmatter::default(), raw.to_string())
        }
    }
}

fn try_parse(raw: &str) -> Option<(NoteFrontmatter, String)> {
    let rest = raw.strip_prefix(DELIMITER)?;
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    let end = rest.find("\n---")?;
    let yaml_block = &rest[..end];
    let after = &rest[end + 4..];
    let body = after.strip_prefix('\n').unwrap_or(after);

    match serde_yaml::from_str::<NoteFrontmatter>(yaml_block) {
        Ok(fm) => Some((fm, body.to_string())),
        Err(e) => {
            log::warn!("frontmatter: YAML parse error, falling back to defaults: {e}");
            None
        }
    }
}

/// Serialize frontmatter + body back into full file content.
pub fn serialize(fm: &NoteFrontmatter, body: &str) -> anyhow::Result<String> {
    let yaml = serde_yaml::to_string(fm)?;
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
        assert_eq!(parsed_body, body);
    }

    #[test]
    fn corrupt_frontmatter_falls_back_without_panicking() {
        let raw = "---\nid: [this is not valid: yaml: at all\n---\nbody text";
        let (fm, body) = parse(raw);
        // Falls back to defaults; body becomes the whole raw content since
        // we cannot trust where the real body boundary was.
        assert_eq!(fm.title, "");
        assert_eq!(body, raw);
    }

    #[test]
    fn missing_frontmatter_falls_back() {
        let raw = "just plain markdown, no frontmatter at all";
        let (fm, body) = parse(raw);
        assert_eq!(fm.title, "");
        assert_eq!(body, raw);
    }
}
