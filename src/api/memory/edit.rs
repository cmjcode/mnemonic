//! Piecewise reads and edits (§3.10.2–3.10.3): `read_note` of one
//! section/block with the whole note's outline and `content_hash`,
//! `append_note` (end of note or of a section/block) and `patch_note`
//! (replace a section/block, or an exact `old_str`). Every edit reloads
//! the note from disk, honours `if_hash` and records the agent.
//! Callers: `api::memory::tools`, `api::mcp`, `src/bin/mnemonic-cli`.

use anyhow::{Context, Result, bail};

use super::{file_hash, outline_of, part_spec, selection_out};
use crate::api::VaultService;
use crate::api::types::*;
use crate::markdown::outline;
use crate::markdown::wikilink;

/// Byte range of lines `[start, end)` (0-based) in `body`.
fn line_byte_range(body: &str, start: usize, end: usize) -> (usize, usize) {
    let mut offsets = vec![0];
    for (i, b) in body.bytes().enumerate() {
        if b == b'\n' {
            offsets.push(i + 1);
        }
    }
    let at = |line: usize| offsets.get(line).copied().unwrap_or(body.len()).min(body.len());
    (at(start), at(end))
}

/// `text` in `body`'s line-ending style.
fn with_eol_of(body: &str, text: &str) -> String {
    if body.contains("\r\n") && !text.contains("\r\n") {
        text.replace('\n', "\r\n")
    } else {
        text.to_string()
    }
}

impl VaultService {
    /// A note — whole, or one section (`Heading`, `Parent#Child`, `^id`)
    /// or anchored block — fresh from disk, with its outline and the
    /// `content_hash` to pass back as `if_hash`. `links` covers the
    /// returned text only.
    pub fn read_note_part(&mut self, req: &ReadNoteRequest) -> Result<NoteDetail> {
        let idx = self.resolve_index(&req.r#ref)?;
        self.reload_note(idx)?;
        let note = &self.vault.notes[idx];
        let content_hash = file_hash(&note.path)?;
        let (body, selection) = match part_spec(req.section.as_deref(), req.block.as_deref()) {
            Some(spec) if !note.is_canvas() => {
                let sel = outline::select(&note.body, &spec)
                    .with_context(|| format!("in {}", self.rel(&note.path)))?;
                (outline::selection_text(&note.body, &sel), Some(selection_out(&sel)))
            }
            Some(_) => bail!("`{}` is a canvas note; it has no sections", self.rel(&note.path)),
            None => (note.body.clone(), None),
        };
        let mut links = Vec::new();
        if !note.is_canvas() {
            for occ in wikilink::parse_wikilinks(&body) {
                if !links.contains(&occ.link.target) {
                    links.push(occ.link.target);
                }
            }
        }
        Ok(NoteDetail {
            summary: self.summary(note),
            outline: if note.is_canvas() { Vec::new() } else { outline_of(&note.body) },
            extra: extra_to_json(&note.frontmatter.extra),
            body,
            links,
            content_hash,
            selection,
        })
    }

    /// Appends `text` to the note (or inside `section`), creating the note
    /// when asked and it doesn't exist.
    pub fn append_note(&mut self, req: &AppendNoteRequest) -> Result<EditNoteResult> {
        if req.text.trim().is_empty() {
            bail!("nothing to append: `text` is empty");
        }
        if req.create_if_missing && self.is_missing(&req.r#ref) {
            if req.section.is_some() {
                bail!("note `{}` does not exist, so it has no section to append to", req.r#ref.trim());
            }
            let mut warnings = Vec::new();
            let note = self.create_note_with_warnings(
                &CreateNoteRequest {
                    title: req.r#ref.trim().to_string(),
                    body: req.text.trim_end().to_string() + "\n",
                    folder: req.folder.clone(),
                    tags: req.tags.clone(),
                    agent: req.agent.clone(),
                },
                &mut warnings,
            )?;
            let content_hash = file_hash(&self.root().join(&note.path))?;
            return Ok(EditNoteResult {
                note,
                content_hash,
                changed: true,
                created: true,
                selection: None,
                replacements: 0,
                warnings,
            });
        }
        let idx = self.open_for_edit(&req.r#ref, req.if_hash.as_deref())?;
        let body = self.vault.notes[idx].body.clone();
        let text = with_eol_of(&body, &req.text);
        let (new_body, selection) = match req.section.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(spec) => {
                let sel = outline::select(&body, spec)?;
                (outline::append_to_selection(&body, &sel, &text), Some(selection_out(&sel)))
            }
            None => (outline::append_to_end(&body, &text), None),
        };
        self.finish_edit(idx, new_body, req.agent.as_deref(), selection, 0)
    }

    /// Replaces one part of a note: a section's content (heading kept), a
    /// block's text (anchor kept), or an exact `old_str` (unique unless
    /// `replace_all`; looked up inside the section/block when one is given).
    pub fn patch_note(&mut self, req: &PatchNoteRequest) -> Result<EditNoteResult> {
        let spec = part_spec(req.section.as_deref(), req.block.as_deref());
        if spec.is_none() && req.old_str.is_none() {
            bail!("patch_note needs `section`, `block` and/or `old_str`; use write_note to replace the whole body");
        }
        let idx = self.open_for_edit(&req.r#ref, req.if_hash.as_deref())?;
        let body = self.vault.notes[idx].body.clone();
        let sel = spec.as_deref().map(|s| outline::select(&body, s)).transpose()?;
        let (new_body, replacements) = match &req.old_str {
            Some(old) => {
                if old.is_empty() {
                    bail!("`old_str` is empty");
                }
                let old = with_eol_of(&body, old);
                let new = with_eol_of(&body, &req.new_str);
                let (start, end) = match &sel {
                    Some(s) => line_byte_range(&body, s.start, s.end),
                    None => (0, body.len()),
                };
                let region = &body[start..end];
                let place = sel.as_ref().map_or("the note".to_string(), |s| format!("`{}`", s.label));
                let count = region.matches(old.as_str()).count();
                if count == 0 {
                    bail!("`old_str` not found in {place}; read the note again — it may have changed");
                }
                if count > 1 && !req.replace_all {
                    bail!("`old_str` occurs {count} times in {place}; include more surrounding text or set replace_all");
                }
                let replaced = if req.replace_all {
                    region.replace(old.as_str(), &new)
                } else {
                    region.replacen(old.as_str(), &new, 1)
                };
                (format!("{}{replaced}{}", &body[..start], &body[end..]), if req.replace_all { count } else { 1 })
            }
            None => {
                let sel = sel.as_ref().context("patch target")?;
                (outline::replace_selection(&body, sel, &with_eol_of(&body, &req.new_str)), 0)
            }
        };
        let selection = sel.as_ref().map(selection_out);
        self.finish_edit(idx, new_body, req.agent.as_deref(), selection, replacements)
    }

    /// Saves `new_body` into note `idx` unless nothing changed.
    fn finish_edit(
        &mut self,
        idx: usize,
        new_body: String,
        agent: Option<&str>,
        selection: Option<SelectionOut>,
        replacements: usize,
    ) -> Result<EditNoteResult> {
        let mut warnings = Vec::new();
        if new_body == self.vault.notes[idx].body {
            let note = &self.vault.notes[idx];
            return Ok(EditNoteResult {
                note: self.summary(note),
                content_hash: file_hash(&note.path)?,
                changed: false,
                created: false,
                selection,
                replacements,
                warnings,
            });
        }
        self.vault.notes[idx].body = new_body;
        warnings.extend(self.stamp_agent(idx, agent, false));
        let (note, content_hash) = self.commit_note(idx, &mut warnings)?;
        Ok(EditNoteResult {
            note,
            content_hash,
            changed: true,
            created: false,
            selection,
            replacements,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::Note;
    use tempfile::tempdir;

    const BODY: &str = "# Proyek\nVisi.\n\n## Status\n- a\n\n## Tim\nOrang.\n";

    fn svc() -> (tempfile::TempDir, VaultService) {
        let dir = tempdir().unwrap();
        Note::create(dir.path(), "Proyek", BODY).unwrap();
        let svc = VaultService::open(dir.path()).unwrap();
        (dir, svc)
    }

    #[test]
    fn read_section_returns_part_outline_and_hash() {
        let (_d, mut svc) = svc();
        let whole = svc.read_note("Proyek").unwrap();
        assert_eq!(whole.body, BODY);
        assert_eq!(whole.outline.len(), 3);
        assert_eq!(whole.outline[1].path, "Proyek#Status");
        assert!(whole.selection.is_none());
        let part = svc
            .read_note_part(&ReadNoteRequest { r#ref: "Proyek".into(), section: Some("status".into()), block: None })
            .unwrap();
        assert_eq!(part.body, "## Status\n- a");
        assert_eq!(part.selection.as_ref().unwrap().line, 4);
        assert_eq!(part.content_hash, whole.content_hash);
    }

    #[test]
    fn append_and_patch_edit_one_part_and_record_the_agent() {
        let (d, mut svc) = svc();
        let hash = svc.read_note("Proyek").unwrap().content_hash;
        let res = svc
            .append_note(&AppendNoteRequest {
                r#ref: "Proyek".into(),
                text: "- b".into(),
                section: Some("Status".into()),
                if_hash: Some(hash.clone()),
                agent: Some("tester".into()),
                ..Default::default()
            })
            .unwrap();
        assert!(res.changed);
        assert_ne!(res.content_hash, hash);
        let raw = std::fs::read_to_string(d.path().join("Proyek.md")).unwrap();
        assert!(raw.contains("## Status\n- a\n- b\n\n## Tim"), "{raw}");
        assert!(raw.contains("updated_by: tester"), "{raw}");

        // The old hash is now stale: the write is refused.
        let stale = svc.patch_note(&PatchNoteRequest {
            r#ref: "Proyek".into(),
            section: Some("Tim".into()),
            new_str: "Budi.".into(),
            if_hash: Some(hash),
            ..Default::default()
        });
        assert!(stale.unwrap_err().to_string().contains("changed since it was read"));

        let res = svc
            .patch_note(&PatchNoteRequest {
                r#ref: "Proyek".into(),
                section: Some("Tim".into()),
                new_str: "Budi.".into(),
                if_hash: Some(res.content_hash),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(res.selection.unwrap().label, "Proyek#Tim");
        let res = svc
            .patch_note(&PatchNoteRequest {
                r#ref: "Proyek".into(),
                old_str: Some("Visi.".into()),
                new_str: "Visi baru.".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(res.replacements, 1);
        let raw = std::fs::read_to_string(d.path().join("Proyek.md")).unwrap();
        assert!(raw.ends_with("# Proyek\nVisi baru.\n\n## Status\n- a\n- b\n\n## Tim\nBudi.\n"), "{raw}");

        let dup = svc.patch_note(&PatchNoteRequest {
            r#ref: "Proyek".into(),
            old_str: Some("- ".into()),
            new_str: "* ".into(),
            ..Default::default()
        });
        assert!(dup.unwrap_err().to_string().contains("occurs 2 times"));
        let none = svc.patch_note(&PatchNoteRequest { r#ref: "Proyek".into(), ..Default::default() });
        assert!(none.is_err());
    }

    #[test]
    fn edits_see_changes_made_on_disk_meanwhile() {
        let (d, mut svc) = svc();
        // Another process (the app) rewrites the note after the service loaded it.
        let path = d.path().join("Proyek.md");
        let raw = std::fs::read_to_string(&path).unwrap().replace("Orang.", "Orang dari app.");
        std::fs::write(&path, raw).unwrap();
        svc.append_note(&AppendNoteRequest { r#ref: "Proyek".into(), text: "Akhir.".into(), ..Default::default() })
            .unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("Orang dari app.\n\nAkhir.\n"), "{raw}");
    }

    #[test]
    fn append_creates_missing_note_in_folder() {
        let (d, mut svc) = svc();
        let res = svc
            .append_note(&AppendNoteRequest {
                r#ref: "Log Harian".into(),
                text: "- mulai".into(),
                create_if_missing: true,
                folder: Some("Jurnal".into()),
                agent: Some("tester".into()),
                ..Default::default()
            })
            .unwrap();
        assert!(res.created);
        assert_eq!(res.note.path, "Jurnal/Log Harian.md");
        let raw = std::fs::read_to_string(d.path().join("Jurnal/Log Harian.md")).unwrap();
        assert!(raw.contains("created_by: tester") && raw.ends_with("- mulai\n"), "{raw}");
        assert_eq!(res.content_hash, file_hash(&d.path().join("Jurnal/Log Harian.md")).unwrap());
    }
}
