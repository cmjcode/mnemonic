//! `Note` model: CRUD for a single note backed by a `.md` file on disk.
//! Callers: `notes::vault` (scan/create), `core::storage` (index rebuild).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use uuid::Uuid;

use super::frontmatter::{self, FileHints, NoteFrontmatter, NoteType};

/// Placeholder stem when a title sanitizes to nothing.
const UNTITLED_STEM: &str = "Untitled";
/// Longest file stem we generate from a title (bytes, on a char boundary).
const MAX_STEM_BYTES: usize = 120;

/// A note loaded from (or about to be written to) disk.
#[derive(Debug, Clone)]
pub struct Note {
    pub path: PathBuf,
    pub frontmatter: NoteFrontmatter,
    pub body: String,
    /// A `<stem>.canvas` (Obsidian JSON Canvas) diagram layer sits next
    /// to the file (§Fase 3 "diagram-bound note"). Detected at load time
    /// so hot paths never touch the filesystem.
    pub has_sidecar: bool,
}

/// Extension of the diagram sidecar next to a note.
pub const SIDECAR_EXT: &str = "canvas";

impl Note {
    /// Load a note from an existing `.md` file. Never fails on malformed
    /// frontmatter — see `frontmatter::parse`.
    pub fn load(path: &Path) -> Result<Note> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading note file {}", path.display()))?;
        let (mut frontmatter, body) = frontmatter::parse_with_hints(&raw, &FileHints::for_path(path));
        // Obsidian's identity is the file name: a note that names no title
        // in its frontmatter is titled after its file.
        if frontmatter.title.trim().is_empty() {
            frontmatter.title = file_stem(path).unwrap_or_default();
        }
        let has_sidecar = sidecar_path_for(path).exists();
        Ok(Note {
            path: path.to_path_buf(),
            frontmatter,
            body,
            has_sidecar,
        })
    }

    /// Where this note's diagram layer lives (whether or not it exists).
    pub fn sidecar_path(&self) -> PathBuf {
        sidecar_path_for(&self.path)
    }

    /// Files that travel with the note on rename/trash/restore/delete:
    /// its `.canvas` sidecar, when present on disk.
    fn companions(&self) -> Vec<(PathBuf, String)> {
        let sidecar = self.sidecar_path();
        if sidecar.exists() {
            vec![(sidecar, SIDECAR_EXT.to_string())]
        } else {
            Vec::new()
        }
    }

    /// Moves every companion file so it keeps sitting next to `new_path`.
    fn move_companions(&self, new_path: &Path) -> Result<()> {
        for (from, ext) in self.companions() {
            let to = new_path.with_extension(&ext);
            std::fs::rename(&from, &to)
                .with_context(|| format!("moving {} alongside the note", from.display()))?;
        }
        Ok(())
    }

    /// Stable hash of the file's current bytes, for detecting edits made
    /// outside the app between open and save (§6 "Watcher Conflict").
    /// `None` when the file can't be read (e.g. deleted).
    pub fn disk_fingerprint(path: &Path) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        let bytes = std::fs::read(path).ok()?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut h);
        Some(h.finish())
    }

    /// Frontmatter tags plus inline `#tag`s from the body, deduplicated
    /// case-insensitively (frontmatter casing wins).
    pub fn effective_tags(&self) -> Vec<String> {
        let mut tags = self.frontmatter.tags.clone();
        for t in super::tags::inline_tags(&self.body) {
            if !tags.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
                tags.push(t);
            }
        }
        tags
    }

    /// `true` when the note carries `tag` (or a tag nested under it) in
    /// its frontmatter or body.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.effective_tags()
            .iter()
            .any(|t| super::tags::matches_tag(t, tag))
    }

    /// `true` when the file is still named after its MNEMONIC id
    /// (`<uuid>.md`, the pre-Fase 0 convention) rather than its title.
    pub fn has_uuid_file_name(&self) -> bool {
        file_stem(&self.path)
            .and_then(|s| Uuid::parse_str(&s).ok())
            .is_some()
    }

    /// Renames the file so its stem matches the title (Obsidian's
    /// convention: file name = note name), picking a non-colliding name in
    /// the same folder. Returns the previous path when a rename happened.
    /// Trashed notes and notes whose stem already is the title (or a
    /// `(n)` variant of it) are left alone.
    pub fn sync_file_name_with_title(&mut self) -> Result<Option<PathBuf>> {
        if self.frontmatter.trashed {
            return Ok(None);
        }
        let wanted = file_stem_for_title(&self.frontmatter.title);
        let Some(current) = file_stem(&self.path) else {
            return Ok(None);
        };
        if current == wanted || is_numbered_variant(&current, &wanted) {
            return Ok(None);
        }
        let Some(dir) = self.path.parent() else {
            return Ok(None);
        };
        let ext = self
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("md");
        let new_path = super::trash::unique_path_in(dir, std::ffi::OsStr::new(&format!("{wanted}.{ext}")));
        self.move_companions(&new_path)?;
        std::fs::rename(&self.path, &new_path)
            .with_context(|| format!("renaming note file to {}", new_path.display()))?;
        let old = std::mem::replace(&mut self.path, new_path);
        Ok(Some(old))
    }

    /// Create a new note file in `dir` with the given title and body.
    pub fn create(dir: &Path, title: &str, body: &str) -> Result<Note> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating vault dir {}", dir.display()))?;

        let now = Utc::now();
        let mut frontmatter = NoteFrontmatter::default();
        frontmatter.title = title.to_string();
        frontmatter.created = now;
        frontmatter.modified = now;

        let path = unique_note_path(dir, title);

        let note = Note {
            path,
            frontmatter,
            body: body.to_string(),
            has_sidecar: false,
        };
        note.save()?;
        Ok(note)
    }

    /// Create a new infinite canvas whiteboard note file in `dir` with the given title.
    pub fn create_canvas(dir: &Path, title: &str) -> Result<Note> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating vault dir {}", dir.display()))?;

        let now = Utc::now();
        let mut frontmatter = NoteFrontmatter::default();
        frontmatter.title = title.to_string();
        frontmatter.note_type = NoteType::Canvas;
        frontmatter.tags = vec!["canvas".to_string()];
        frontmatter.created = now;
        frontmatter.modified = now;

        let path = unique_note_path(dir, title);

        // A diagram-bound note (§Fase 3): the text lives in the Markdown as
        // an anchored block, the geometry in the `.canvas` sidecar whose
        // node points back at that block.
        let block_id = crate::markdown::blocks::generate_id(&[]);
        let text = "Klik dua kali kartu ini untuk mengubah teksnya — teksnya juga ada di catatan Markdown.";
        let body = crate::markdown::blocks::append_block("", text, &block_id);
        let mut canvas = crate::canvas::CanvasDocument::new(title);
        canvas.add_element(crate::canvas::CanvasElement::StickyNote {
            id: crate::canvas::CanvasElementId::new(),
            pos: [100.0, 100.0],
            size: [260.0, 130.0],
            text: text.to_string(),
            color: crate::canvas::tools::PALETTE_STICKY_YELLOW,
            binding: Some(crate::canvas::BlockBinding::local(block_id)),
        });

        let note = Note {
            path,
            frontmatter,
            body,
            has_sidecar: true,
        };
        note.save()?;
        let owner = note
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string());
        write_atomic(
            &note.sidecar_path(),
            canvas.to_json_canvas_string(owner.as_deref()).as_bytes(),
        )?;
        Ok(note)
    }

    /// Writes `canvas` as this note's `.canvas` sidecar.
    pub fn save_sidecar(&mut self, canvas: &crate::canvas::CanvasDocument, owner_rel_path: Option<&str>) -> Result<()> {
        write_atomic(
            &self.sidecar_path(),
            canvas.to_json_canvas_string(owner_rel_path).as_bytes(),
        )?;
        self.has_sidecar = true;
        Ok(())
    }

    /// Create a new Draw.io diagram note file in `dir` with the given title.
    pub fn create_drawio(dir: &Path, title: &str) -> Result<Note> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating vault dir {}", dir.display()))?;

        let now = Utc::now();
        let mut frontmatter = NoteFrontmatter::default();
        frontmatter.title = title.to_string();
        frontmatter.note_type = NoteType::Canvas;
        frontmatter.tags = vec!["canvas".to_string(), "drawio".to_string()];
        frontmatter.created = now;
        frontmatter.modified = now;

        let path = unique_note_path(dir, title);

        let mut canvas = crate::canvas::CanvasDocument::new(title);
        // Add sample starter flowchart elements
        let start_id = canvas.add_element(crate::canvas::CanvasElement::Shape {
            id: crate::canvas::CanvasElementId::new(),
            kind: crate::canvas::ShapeKind::RoundedRect,
            rect: [100.0, 100.0, 240.0, 160.0],
            stroke_color: [0.23, 0.51, 0.96],
            stroke_width: 2.0,
            fill_color: Some([0.15, 0.20, 0.35]),
            text: "🚀 Mulai / Start".to_string(),
            text_color: None,
            binding: None,
        });

        let decision_id = canvas.add_element(crate::canvas::CanvasElement::Shape {
            id: crate::canvas::CanvasElementId::new(),
            kind: crate::canvas::ShapeKind::Diamond,
            rect: [100.0, 220.0, 240.0, 320.0],
            stroke_color: [0.95, 0.60, 0.07],
            stroke_width: 2.0,
            fill_color: Some([0.28, 0.22, 0.10]),
            text: "Validasi?\nValid?".to_string(),
            text_color: None,
            binding: None,
        });

        let process_id = canvas.add_element(crate::canvas::CanvasElement::Shape {
            id: crate::canvas::CanvasElementId::new(),
            kind: crate::canvas::ShapeKind::Rectangle,
            rect: [320.0, 240.0, 460.0, 300.0],
            stroke_color: [0.13, 0.77, 0.37],
            stroke_width: 2.0,
            fill_color: Some([0.10, 0.25, 0.16]),
            text: "Proses Data".to_string(),
            text_color: None,
            binding: None,
        });

        canvas.add_element(crate::canvas::CanvasElement::Connector {
            id: crate::canvas::CanvasElementId::new(),
            from_elem: Some(start_id),
            to_elem: Some(decision_id),
            from_pos: [170.0, 160.0],
            to_pos: [170.0, 220.0],
            routing: crate::canvas::ConnectorRouting::Straight,
            stroke_color: [0.23, 0.51, 0.96],
            stroke_width: 2.0,
            label: "".to_string(),
            arrow_end: true,
            waypoints: Vec::new(),
        });

        canvas.add_element(crate::canvas::CanvasElement::Connector {
            id: crate::canvas::CanvasElementId::new(),
            from_elem: Some(decision_id),
            to_elem: Some(process_id),
            from_pos: [240.0, 270.0],
            to_pos: [320.0, 270.0],
            routing: crate::canvas::ConnectorRouting::Orthogonal,
            stroke_color: [0.13, 0.77, 0.37],
            stroke_width: 2.0,
            label: "Ya".to_string(),
            arrow_end: true,
            waypoints: Vec::new(),
        });

        let xml = canvas.to_drawio_xml();
        let body = format!("```drawio\n{}\n```\n", xml.trim());

        let note = Note {
            path,
            frontmatter,
            body,
            has_sidecar: false,
        };
        note.save()?;
        Ok(note)
    }

    /// Checks whether this note is a visual Whiteboard Canvas.
    pub fn is_canvas(&self) -> bool {
        self.has_sidecar
            || self.frontmatter.note_type == NoteType::Canvas
            || self.body.contains("```canvas")
            || self.body.contains("```drawio")
            || self.body.starts_with("<?xml")
            || self.body.starts_with("<mxfile")
            || self.body.starts_with("<mxGraphModel")
            || self.path.extension().is_some_and(|ext| ext == "drawio")
            || self.frontmatter.tags.iter().any(|t| {
                t.eq_ignore_ascii_case("whiteboard")
                    || t.eq_ignore_ascii_case("canvas")
                    || t.eq_ignore_ascii_case("drawio")
            })
    }

    /// Write current frontmatter + body back to `self.path`, bumping
    /// `modified`. Atomic: the content goes to a temp file in the same
    /// folder first and is renamed over the note, so a crash mid-write
    /// never leaves a truncated note behind (§3.1.4 "Keamanan Data").
    pub fn save(&self) -> Result<()> {
        let mut fm = self.frontmatter.clone();
        fm.modified = Utc::now();
        let raw = frontmatter::serialize(&fm, &self.body)?;
        write_atomic(&self.path, raw.as_bytes())
    }

    /// Soft-delete: mark `trashed: true` and move the file into `.trash/`
    /// under the given vault root. Per §3.1.4, permanent deletion after a
    /// retention window is handled separately by `notes::trash`.
    pub fn move_to_trash(mut self, vault_root: &Path) -> Result<Note> {
        self.frontmatter.trashed = true;
        let file_name = self
            .path
            .file_name()
            .context("note path has no file name")?;
        let new_path = super::trash::unique_trash_path(vault_root, file_name)?;

        self.save()?; // persist trashed:true at the old path first
        self.move_companions(&new_path)?;
        std::fs::rename(&self.path, &new_path)
            .with_context(|| format!("moving note to trash {}", new_path.display()))?;
        self.path = new_path;
        Ok(self)
    }

    /// Move a trashed note back out of `.trash/` and clear `trashed`
    /// (§3.1.4 "Sampah"). Always restores to `vault_root` directly rather
    /// than its original subfolder, since that original location isn't
    /// tracked — a known simplification.
    pub fn restore_from_trash(self, vault_root: &Path) -> Result<Note> {
        let file_name = self
            .path
            .file_name()
            .context("note path has no file name")?;
        let new_path = super::trash::unique_path_in(vault_root, file_name);
        self.restore_to(new_path)
    }

    /// Moves a trashed note back out of `.trash/` to exactly `target`
    /// (the "Urungkan" undo right after trashing, which knows the original
    /// subfolder), clearing `trashed`. Falls back to a non-colliding name
    /// next to `target` if something has taken its place meanwhile.
    pub fn restore_to(mut self, target: PathBuf) -> Result<Note> {
        self.frontmatter.trashed = false;
        let target = match (target.parent(), target.file_name()) {
            (Some(dir), Some(name)) => {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("recreating folder {}", dir.display()))?;
                super::trash::unique_path_in(dir, name)
            }
            _ => target,
        };

        self.save()?; // persist trashed:false at the old (.trash) path first
        self.move_companions(&target)?;
        std::fs::rename(&self.path, &target)
            .with_context(|| format!("restoring note from trash {}", target.display()))?;
        self.path = target;
        Ok(self)
    }

    /// Permanently delete the note file from disk — the manual "Hapus
    /// Permanen" action in the Sampah view, as opposed to the automatic
    /// 30-day purge in `notes::trash::purge_expired`.
    pub fn delete_permanently(self) -> Result<()> {
        for (companion, _) in self.companions() {
            std::fs::remove_file(&companion)
                .with_context(|| format!("deleting {}", companion.display()))?;
        }
        std::fs::remove_file(&self.path)
            .with_context(|| format!("permanently deleting note file {}", self.path.display()))
    }

    /// Checklist completion for the grid card badge (§3.1.2 "3/5
    /// selesai"): counts `- [ ]`/`- [x]` items in the body. `None` if the
    /// note has no checklist items, so callers can hide the badge.
    pub fn checklist_progress(&self) -> Option<(usize, usize)> {
        let mut total = 0;
        let mut done = 0;
        for line in self.body.lines() {
            let trimmed = line.trim_start();
            let Some(rest) = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
                .or_else(|| trimmed.strip_prefix("+ "))
            else {
                continue;
            };
            if rest.starts_with("[ ]") {
                total += 1;
            } else if rest.starts_with("[x]") || rest.starts_with("[X]") {
                total += 1;
                done += 1;
            }
        }
        if total == 0 {
            None
        } else {
            Some((done, total))
        }
    }
}

/// Writes `bytes` to `path` via a sibling temp file + rename.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("note path has no parent folder")?;
    if !dir.as_os_str().is_empty() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating folder {}", dir.display()))?;
    }
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{file_name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes)
        .with_context(|| format!("writing temp note file {}", tmp.display()))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("replacing note file {}", path.display()));
    }
    Ok(())
}

fn file_stem(path: &Path) -> Option<String> {
    path.file_stem().map(|s| s.to_string_lossy().to_string())
}

/// `<note>.canvas` next to `note_path`.
pub fn sidecar_path_for(note_path: &Path) -> PathBuf {
    note_path.with_extension(SIDECAR_EXT)
}

/// `true` for `Title (2)`, `Title (3)`, … — the collision suffixes
/// `unique_path_in` appends.
fn is_numbered_variant(stem: &str, wanted: &str) -> bool {
    stem.strip_prefix(wanted)
        .and_then(|rest| rest.strip_prefix(" ("))
        .and_then(|rest| rest.strip_suffix(')'))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The file stem a note titled `title` gets: Obsidian's forbidden
/// characters (`* " \ / < > : | ?` and the link-syntax `# ^ [ ]`) and
/// control characters become spaces, whitespace collapses, and a very long
/// or empty title falls back sensibly.
pub fn file_stem_for_title(title: &str) -> String {
    const FORBIDDEN: &[char] = &['*', '"', '\\', '/', '<', '>', ':', '|', '?', '#', '^', '[', ']'];
    let cleaned: String = title
        .chars()
        .map(|c| if FORBIDDEN.contains(&c) || c.is_control() { ' ' } else { c })
        .collect();
    let mut stem = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    // Trailing dots/spaces are illegal on Windows and confusing everywhere.
    while stem.ends_with('.') {
        stem.pop();
    }
    let stem = stem.trim().to_string();
    if stem.is_empty() || stem == "." || stem == ".." {
        return UNTITLED_STEM.to_string();
    }
    if stem.len() > MAX_STEM_BYTES {
        let mut cut = MAX_STEM_BYTES;
        while !stem.is_char_boundary(cut) {
            cut -= 1;
        }
        return stem[..cut].trim_end().to_string();
    }
    stem
}

/// `dir/<title>.md`, or `dir/<title> (n).md` if that name is taken.
pub fn unique_note_path(dir: &Path, title: &str) -> PathBuf {
    let name = format!("{}.md", file_stem_for_title(title));
    super::trash::unique_path_in(dir, std::ffi::OsStr::new(&name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn file_stem_sanitizes_obsidian_forbidden_chars() {
        assert_eq!(file_stem_for_title("Belanja: Mingguan / #1?"), "Belanja Mingguan 1");
        assert_eq!(file_stem_for_title("   "), "Untitled");
        assert_eq!(file_stem_for_title("Trailing..."), "Trailing");
        assert_eq!(file_stem_for_title("Ünïcödé — ok"), "Ünïcödé — ok");
    }

    #[test]
    fn create_names_file_after_title_and_avoids_collisions() {
        let dir = tempdir().unwrap();
        let a = Note::create(dir.path(), "Judul", "a").unwrap();
        let b = Note::create(dir.path(), "Judul", "b").unwrap();
        assert_eq!(a.path.file_name().unwrap(), "Judul.md");
        assert_eq!(b.path.file_name().unwrap(), "Judul (2).md");
        assert!(!a.has_uuid_file_name());
    }

    #[test]
    fn sync_file_name_follows_title_changes() {
        let dir = tempdir().unwrap();
        let mut note = Note::create(dir.path(), "Lama", "isi").unwrap();
        note.frontmatter.title = "Baru".to_string();
        note.save().unwrap();
        let old = note.sync_file_name_with_title().unwrap();
        assert_eq!(old.unwrap().file_name().unwrap(), "Lama.md");
        assert_eq!(note.path.file_name().unwrap(), "Baru.md");
        assert!(note.path.exists());
        // Already in sync: no-op, also for the "(n)" variant.
        assert!(note.sync_file_name_with_title().unwrap().is_none());
        let mut twin = Note::create(dir.path(), "Baru", "x").unwrap();
        assert_eq!(twin.path.file_name().unwrap(), "Baru (2).md");
        assert!(twin.sync_file_name_with_title().unwrap().is_none());
    }

    #[test]
    fn load_plain_obsidian_note_titles_from_file_name() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("Catatan Obsidian.md");
        std::fs::write(&path, "# Heading

isi tanpa frontmatter").unwrap();
        let note = Note::load(&path).unwrap();
        assert_eq!(note.frontmatter.title, "Catatan Obsidian");
        assert_eq!(note.body, "# Heading

isi tanpa frontmatter");
        // Stable across loads.
        assert_eq!(note.frontmatter.id, Note::load(&path).unwrap().frontmatter.id);
    }

    #[test]
    fn save_is_atomic_and_leaves_no_temp_files() {
        let dir = tempdir().unwrap();
        let note = Note::create(dir.path(), "Atomik", "isi").unwrap();
        note.save().unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "tmp"))
            .collect();
        assert!(leftovers.is_empty());
        assert_eq!(Note::load(&note.path).unwrap().body, "isi");
    }

    #[test]
    fn create_then_load_round_trips() {
        let dir = tempdir().unwrap();
        let created = Note::create(dir.path(), "Belanja Mingguan", "- [ ] Beli beras\n").unwrap();

        let loaded = Note::load(&created.path).unwrap();
        assert_eq!(loaded.frontmatter.title, "Belanja Mingguan");
        assert_eq!(loaded.body, "- [ ] Beli beras\n");
        assert_eq!(loaded.frontmatter.id, created.frontmatter.id);
    }

    #[test]
    fn save_bumps_modified_timestamp() {
        let dir = tempdir().unwrap();
        let mut note = Note::create(dir.path(), "Judul", "isi").unwrap();
        let first_modified = note.frontmatter.modified;

        std::thread::sleep(std::time::Duration::from_millis(5));
        note.body = "isi baru".to_string();
        note.save().unwrap();

        let reloaded = Note::load(&note.path).unwrap();
        assert!(reloaded.frontmatter.modified > first_modified);
        assert_eq!(reloaded.body, "isi baru");
    }

    #[test]
    fn move_to_trash_sets_flag_and_relocates_file() {
        let dir = tempdir().unwrap();
        let note = Note::create(dir.path(), "Hapus Saya", "isi").unwrap();
        let original_path = note.path.clone();

        let trashed = note.move_to_trash(dir.path()).unwrap();

        assert!(trashed.frontmatter.trashed);
        assert!(!original_path.exists());
        assert!(trashed.path.exists());
        assert_eq!(trashed.path.parent().unwrap().file_name().unwrap(), ".trash");
    }

    #[test]
    fn trashing_same_named_notes_keeps_both_and_undo_restores_subfolder() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("Projects");
        std::fs::create_dir_all(&sub).unwrap();
        let a = Note::create(dir.path(), "Kembar", "a").unwrap();
        let mut b = Note::create(&sub, "Kembar", "b").unwrap();
        // Give both files the same name (e.g. copied in from elsewhere).
        let original_b = sub.join(a.path.file_name().unwrap());
        std::fs::rename(&b.path, &original_b).unwrap();
        b.path = original_b.clone();

        let ta = a.move_to_trash(dir.path()).unwrap();
        let tb = b.move_to_trash(dir.path()).unwrap();
        assert_ne!(ta.path, tb.path);
        assert!(ta.path.exists() && tb.path.exists());

        let restored = tb.restore_to(original_b.clone()).unwrap();
        assert_eq!(restored.path, original_b);
        assert!(!restored.frontmatter.trashed);
        assert_eq!(Note::load(&original_b).unwrap().body, "b");
    }

    #[test]
    fn restore_from_trash_clears_flag_and_moves_back() {
        let dir = tempdir().unwrap();
        let note = Note::create(dir.path(), "Pulihkan Saya", "isi").unwrap();
        let trashed = note.move_to_trash(dir.path()).unwrap();

        let restored = trashed.restore_from_trash(dir.path()).unwrap();

        assert!(!restored.frontmatter.trashed);
        assert_eq!(restored.path.parent().unwrap(), dir.path());
        assert!(restored.path.exists());
    }

    #[test]
    fn delete_permanently_removes_file() {
        let dir = tempdir().unwrap();
        let note = Note::create(dir.path(), "Musnah", "isi").unwrap();
        let path = note.path.clone();

        note.delete_permanently().unwrap();

        assert!(!path.exists());
    }

    #[test]
    fn checklist_progress_counts_done_and_total() {
        let dir = tempdir().unwrap();
        let note = Note::create(
            dir.path(),
            "Belanja",
            "- [ ] Beras\n- [x] Telur\n- [X] Gula\nbukan checklist\n",
        )
        .unwrap();

        assert_eq!(note.checklist_progress(), Some((2, 3)));
    }

    #[test]
    fn checklist_progress_none_without_checklist_items() {
        let dir = tempdir().unwrap();
        let note = Note::create(dir.path(), "Bebas", "cuma teks biasa").unwrap();
        assert_eq!(note.checklist_progress(), None);
    }

    #[test]
    fn create_canvas_writes_bound_sidecar_and_moves_it_with_the_note() {
        let dir = tempdir().unwrap();
        let canvas_note = Note::create_canvas(dir.path(), "Diagram Arsitektur").unwrap();
        assert!(canvas_note.is_canvas());
        assert!(canvas_note.has_sidecar);
        assert_eq!(canvas_note.frontmatter.note_type, NoteType::Canvas);
        assert!(canvas_note.sidecar_path().exists());
        // The text is a Markdown block with an anchor, mirrored by a bound
        // node in the sidecar.
        let anchors = crate::markdown::blocks::block_anchors(&canvas_note.body);
        assert_eq!(anchors.len(), 1);
        let json = std::fs::read_to_string(canvas_note.sidecar_path()).unwrap();
        assert!(json.contains(&format!("#^{}", anchors[0].id)));
        assert!(json.contains("\"type\": \"file\""));

        let loaded = Note::load(&canvas_note.path).unwrap();
        assert!(loaded.is_canvas() && loaded.has_sidecar);

        // Rename, trash, restore and delete all carry the sidecar along.
        let mut renamed = loaded;
        renamed.frontmatter.title = "Arsitektur Baru".to_string();
        renamed.save().unwrap();
        renamed.sync_file_name_with_title().unwrap();
        assert!(dir.path().join("Arsitektur Baru.canvas").exists());
        assert!(!dir.path().join("Diagram Arsitektur.canvas").exists());
        let trashed = renamed.move_to_trash(dir.path()).unwrap();
        assert!(trashed.sidecar_path().exists());
        let restored = trashed.restore_from_trash(dir.path()).unwrap();
        assert!(restored.sidecar_path().exists());
        let sidecar = restored.sidecar_path();
        restored.delete_permanently().unwrap();
        assert!(!sidecar.exists());
    }
}
