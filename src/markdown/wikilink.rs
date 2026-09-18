//! Wikilink `[[Title]]` extraction & resolution, plus the backlink graph
//! (§3.2.2). Supports the Obsidian forms `[[Title]]`, `[[Title|alias]]`,
//! `[[Title#Heading]]` and `[[file.pdf#page=3]]`. Pure text-processing
//! lives here so it stays unit-testable without an `egui::Ui`; the link
//! table built from it lives in `core::storage`, click handling in
//! `markdown::renderer`. Callers: `markdown::renderer`, `core::storage`,
//! `app`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::notes::Note;

/// Longest context line kept per link occurrence (chars) — enough for a
/// readable backlink preview without storing whole paragraphs.
const CONTEXT_MAX_CHARS: usize = 160;

/// One parsed `[[...]]` reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLink {
    /// Note title or file name the link points at (heading/alias removed).
    pub target: String,
    /// Text after `#`: a heading, or `page=N` for PDFs.
    pub heading: Option<String>,
    /// Display text after `|`.
    pub alias: Option<String>,
}

impl WikiLink {
    /// Parses the text between `[[` and `]]`.
    pub fn parse(inner: &str) -> WikiLink {
        let (reference, alias) = match inner.split_once('|') {
            Some((r, a)) => (r, Some(a.trim()).filter(|a| !a.is_empty())),
            None => (inner, None),
        };
        let (target, heading) = match reference.split_once('#') {
            Some((t, h)) => (t, Some(h.trim()).filter(|h| !h.is_empty())),
            None => (reference, None),
        };
        WikiLink {
            target: target.trim().to_string(),
            heading: heading.map(str::to_string),
            alias: alias.map(str::to_string),
        }
    }

    /// `Target` or `Target#Heading` — the link reference without its alias,
    /// as used for click destinations.
    pub fn reference(&self) -> String {
        match &self.heading {
            Some(h) => format!("{}#{h}", self.target),
            None => self.target.clone(),
        }
    }

    /// `true` when the target names a PDF file rather than a note.
    pub fn is_pdf(&self) -> bool {
        self.target.to_lowercase().ends_with(".pdf")
    }

    /// 1-based page from a `#page=N` suffix.
    pub fn page(&self) -> Option<usize> {
        self.heading
            .as_deref()?
            .strip_prefix("page=")?
            .trim()
            .parse()
            .ok()
            .filter(|p| *p > 0)
    }
}

/// A wikilink together with where it occurs in the note body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkOccurrence {
    pub link: WikiLink,
    /// 0-based line index in the body.
    pub line: usize,
    /// The (trimmed, truncated) line containing the link.
    pub context: String,
}

/// Case-insensitive lookup key for titles and link targets.
pub fn title_key(title: &str) -> String {
    title.trim().to_lowercase()
}

/// Every wikilink in `body` with its line and context, in order of
/// appearance (duplicates included). `![[embed]]`s are not note links and
/// links inside fenced code blocks (``` or ~~~) are ignored.
pub fn parse_wikilinks(body: &str) -> Vec<LinkOccurrence> {
    let mut out = Vec::new();
    for_each_prose_line(body, |line_idx, line| {
        find_wikilinks_in_line(line, |inner, is_embed| {
            let link = WikiLink::parse(inner);
            if !is_embed && !link.target.is_empty() {
                out.push(LinkOccurrence {
                    link,
                    line: line_idx,
                    context: context_snippet(line),
                });
            }
        });
    });
    out
}

/// Extract every wikilink target title referenced in `body`, in order of
/// appearance (duplicates included), with any `#heading` and `|alias`
/// stripped. See `parse_wikilinks` for what is skipped.
pub fn extract_wikilinks(body: &str) -> Vec<String> {
    parse_wikilinks(body)
        .into_iter()
        .map(|o| o.link.target)
        .collect()
}

/// Calls `f(line_index, line)` for every line outside fenced code blocks.
fn for_each_prose_line(body: &str, mut f: impl FnMut(usize, &str)) {
    let mut in_fence = false;
    for (i, line) in body.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            f(i, line);
        }
    }
}

fn context_snippet(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.chars().count() <= CONTEXT_MAX_CHARS {
        return trimmed.to_string();
    }
    let mut s: String = trimmed.chars().take(CONTEXT_MAX_CHARS).collect();
    s.push('…');
    s
}

/// Scans a single line for `[[...]]` occurrences, invoking `on_match` with
/// the raw inner text and whether it was an embed (`![[`).
pub(super) fn find_wikilinks_in_line(line: &str, mut on_match: impl FnMut(&str, bool)) {
    for (start, end) in wikilink_spans(line) {
        let is_embed = start > 0 && line.as_bytes()[start - 1] == b'!';
        on_match(&line[start + 2..end - 2], is_embed);
    }
}

/// Byte ranges `[start, end)` of each complete `[[...]]` in `line`,
/// brackets included.
fn wikilink_spans(line: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut offset = 0;
    while let Some(rel_start) = line[offset..].find("[[") {
        let start = offset + rel_start;
        let Some(rel_end) = line[start + 2..].find("]]") else {
            break;
        };
        let end = start + 2 + rel_end + 2;
        spans.push((start, end));
        offset = end;
    }
    spans
}

/// `line` with each `[[Target#Heading|Alias]]` replaced by the text a
/// reader sees (the alias, else the target) — for link context previews.
pub fn display_text(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut last = 0;
    for (start, end) in wikilink_spans(line) {
        let link = WikiLink::parse(&line[start + 2..end - 2]);
        let embed = start > 0 && line.as_bytes()[start - 1] == b'!';
        out.push_str(&line[last..if embed { start - 1 } else { start }]);
        out.push_str(link.alias.as_deref().unwrap_or(&link.target));
        last = end;
    }
    out.push_str(&line[last..]);
    out
}

/// Rewrites every link and embed to `old_title` (case-insensitive) so it
/// points at `new_title`, keeping `#heading`, `|alias` and line endings
/// intact — used when a note is renamed. Fenced code is left alone.
/// Returns `None` when nothing changed.
pub fn rewrite_link_target(body: &str, old_title: &str, new_title: &str) -> Option<String> {
    let old_key = title_key(old_title);
    if old_key.is_empty() || old_title.trim() == new_title.trim() {
        return None;
    }
    let mut out = String::with_capacity(body.len());
    let mut changed = false;
    let mut in_fence = false;
    for line in body.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            out.push_str(line);
            continue;
        }
        if in_fence {
            out.push_str(line);
            continue;
        }
        let mut last = 0;
        for (start, end) in wikilink_spans(line) {
            let inner = &line[start + 2..end - 2];
            let link = WikiLink::parse(inner);
            if title_key(&link.target) != old_key {
                continue;
            }
            // Replace just the target portion so heading/alias survive.
            let target_len = inner.find(['#', '|']).unwrap_or(inner.len());
            out.push_str(&line[last..start + 2]);
            out.push_str(new_title.trim());
            out.push_str(&inner[target_len..]);
            out.push_str("]]");
            last = end;
            changed = true;
        }
        out.push_str(&line[last..]);
    }
    changed.then_some(out)
}

/// Byte range of the first whole-word, unlinked occurrence of `needle`
/// (already lowercased) in `lower` (a lowercased line).
fn find_unlinked(lower: &str, needle: &str) -> Option<(usize, usize)> {
    let spans = wikilink_spans(lower);
    let mut offset = 0;
    while let Some(rel) = lower[offset..].find(needle) {
        let start = offset + rel;
        let end = start + needle.len();
        offset = end;
        let before_ok = lower[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after_ok = lower[end..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        let inside_link = spans.iter().any(|(s, e)| start >= *s && end <= *e);
        if before_ok && after_ok && !inside_link {
            return Some((start, end));
        }
    }
    None
}

/// Lines where `title` appears as plain text (whole words, case-
/// insensitive) without being linked — Obsidian's "unlinked mentions".
/// Very short titles (< 3 chars) are ignored since they match everywhere.
pub fn unlinked_mentions(body: &str, title: &str) -> Vec<LinkOccurrence> {
    let needle = title_key(title);
    if needle.chars().count() < 3 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for_each_prose_line(body, |line_idx, line| {
        if find_unlinked(&line.to_lowercase(), &needle).is_some() {
            out.push(LinkOccurrence {
                link: WikiLink {
                    target: title.trim().to_string(),
                    heading: None,
                    alias: None,
                },
                line: line_idx,
                context: context_snippet(line),
            });
        }
    });
    out
}

/// Turns the first unlinked mention of `title` on `line` into a
/// `[[title]]` link (keeping the original casing as an alias when it
/// differs). Returns `None` if the line has no such mention anymore.
pub fn link_mention_on_line(body: &str, line: usize, title: &str) -> Option<String> {
    let needle = title_key(title);
    if needle.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(body.len() + 4);
    let mut replaced = false;
    for (i, raw) in body.split_inclusive('\n').enumerate() {
        let lower = raw.to_lowercase();
        // Offsets only map back safely when lowercasing kept byte lengths.
        let hit = (i == line && lower.len() == raw.len())
            .then(|| find_unlinked(&lower, &needle))
            .flatten();
        let Some((start, end)) = hit else {
            out.push_str(raw);
            continue;
        };
        let original = &raw[start..end];
        let canonical = title.trim();
        out.push_str(&raw[..start]);
        if original == canonical {
            out.push_str(&format!("[[{canonical}]]"));
        } else {
            out.push_str(&format!("[[{canonical}|{original}]]"));
        }
        out.push_str(&raw[end..]);
        replaced = true;
    }
    replaced.then_some(out)
}

/// Maps link targets (case-insensitive) to the note that owns them, for
/// wikilink resolution and autocomplete (§3.2.2). A note is reachable by
/// its title, its file stem (Obsidian's native identity) and each of its
/// frontmatter `aliases`, in that priority order on collisions.
pub struct WikilinkIndex {
    by_title: HashMap<String, (String, PathBuf)>,
}

impl WikilinkIndex {
    /// Build the index from all (non-trashed) notes currently in the
    /// vault. Titles are assumed unique, matching Obsidian's own
    /// convention; the last note wins on a title collision, while stems
    /// and aliases never override a title.
    pub fn build(notes: &[Note]) -> WikilinkIndex {
        let mut by_title = HashMap::new();
        for note in notes {
            if note.frontmatter.trashed || note.frontmatter.title.is_empty() {
                continue;
            }
            by_title.insert(
                title_key(&note.frontmatter.title),
                (note.frontmatter.title.clone(), note.path.clone()),
            );
        }
        for note in notes {
            if note.frontmatter.trashed {
                continue;
            }
            for key in link_keys_for(note).into_iter().skip(1) {
                by_title
                    .entry(key)
                    .or_insert((note.frontmatter.title.clone(), note.path.clone()));
            }
        }
        WikilinkIndex { by_title }
    }

    /// Adds non-note link targets (PDF file names) so they resolve and
    /// show up in autocomplete too.
    pub fn with_files(mut self, files: &[PathBuf]) -> WikilinkIndex {
        for path in files {
            if let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) {
                self.by_title
                    .entry(title_key(&name))
                    .or_insert((name, path.clone()));
            }
        }
        self
    }

    /// The path of the note titled `title`, if one exists.
    pub fn resolve(&self, title: &str) -> Option<&Path> {
        self.by_title.get(&title_key(title)).map(|(_, p)| p.as_path())
    }

    pub fn contains(&self, title: &str) -> bool {
        self.by_title.contains_key(&title_key(title))
    }

    /// Titles starting with `prefix` (case-insensitive), sorted
    /// alphabetically, then titles merely containing it, capped at
    /// `limit` — backing the `[[` autocomplete popup.
    pub fn suggestions(&self, prefix: &str, limit: usize) -> Vec<String> {
        let needle = title_key(prefix);
        let mut starts: Vec<&str> = Vec::new();
        let mut contains: Vec<&str> = Vec::new();
        for (key, (title, _)) in &self.by_title {
            if key.starts_with(&needle) {
                starts.push(title);
            } else if key.contains(&needle) {
                contains.push(title);
            }
        }
        starts.sort_unstable();
        contains.sort_unstable();
        starts
            .into_iter()
            .chain(contains)
            .take(limit)
            .map(String::from)
            .collect()
    }
}

/// Every key a `[[link]]` may use to reach `note`: its title first, then
/// its file stem (when it isn't a legacy `<uuid>` name) and its aliases.
/// Deduplicated, in that order.
pub fn link_keys_for(note: &Note) -> Vec<String> {
    let mut keys = Vec::new();
    let mut push = |k: String| {
        if !k.is_empty() && !keys.contains(&k) {
            keys.push(k);
        }
    };
    push(title_key(&note.frontmatter.title));
    if let Some(stem) = note.path.file_stem().and_then(|s| s.to_str())
        && Uuid::parse_str(stem).is_err()
    {
        push(title_key(stem));
    }
    for alias in &note.frontmatter.aliases {
        push(title_key(alias));
    }
    keys
}

/// Notes (other than the one identified by `current_id`) whose body
/// contains a wikilink to `target_title`. The app reads backlinks from
/// `core::storage`'s link table; this in-memory version serves callers
/// without an index.
pub fn backlinks_for<'a>(target_title: &str, current_id: Uuid, notes: &'a [Note]) -> Vec<&'a Note> {
    let target = title_key(target_title);
    notes
        .iter()
        .filter(|n| n.frontmatter.id != current_id && !n.frontmatter.trashed)
        .filter(|n| {
            extract_wikilinks(&n.body)
                .iter()
                .any(|t| title_key(t) == target)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn extract_wikilinks_finds_plain_links() {
        let body = "Lihat [[Belanja Mingguan]] dan [[Resep Nasi Goreng]].";
        assert_eq!(
            extract_wikilinks(body),
            vec!["Belanja Mingguan", "Resep Nasi Goreng"]
        );
    }

    #[test]
    fn extract_wikilinks_strips_alias() {
        let body = "Baca [[Belanja Mingguan|daftar belanja]] dulu.";
        assert_eq!(extract_wikilinks(body), vec!["Belanja Mingguan"]);
    }

    #[test]
    fn extract_wikilinks_strips_heading() {
        let body = "Lihat [[Resep#Bahan|bahan-bahan]].";
        assert_eq!(extract_wikilinks(body), vec!["Resep"]);
    }

    #[test]
    fn extract_wikilinks_ignores_image_embeds() {
        let body = "![[foto.png]] tapi [[Catatan Lain]] tetap dihitung.";
        assert_eq!(extract_wikilinks(body), vec!["Catatan Lain"]);
    }

    #[test]
    fn extract_wikilinks_ignores_fenced_code_blocks() {
        let body = "```\n[[Bukan Link]]\n```\n[[Link Asli]]";
        assert_eq!(extract_wikilinks(body), vec!["Link Asli"]);
    }

    #[test]
    fn parse_splits_target_heading_and_alias() {
        let link = WikiLink::parse(" Resep # Bahan | daftar ");
        assert_eq!(link.target, "Resep");
        assert_eq!(link.heading.as_deref(), Some("Bahan"));
        assert_eq!(link.alias.as_deref(), Some("daftar"));
        assert_eq!(link.reference(), "Resep#Bahan");
    }

    #[test]
    fn parse_reads_pdf_page_suffix() {
        let link = WikiLink::parse("laporan.pdf#page=3");
        assert!(link.is_pdf());
        assert_eq!(link.page(), Some(3));
        assert_eq!(WikiLink::parse("Resep#Bahan").page(), None);
    }

    #[test]
    fn parse_wikilinks_records_line_and_context() {
        let body = "judul\n\n  Lihat [[A]] dan [[B#h]]  \n```\n[[C]]\n```";
        let occ = parse_wikilinks(body);
        assert_eq!(occ.len(), 2);
        assert_eq!(occ[0].line, 2);
        assert_eq!(occ[0].context, "Lihat [[A]] dan [[B#h]]");
        assert_eq!(occ[1].link.heading.as_deref(), Some("h"));
    }

    #[test]
    fn rewrite_link_target_keeps_heading_alias_and_newlines() {
        let body = "a [[Lama]] b [[lama#Bagian|teks]]\r\n![[Lama]]\n```\n[[Lama]]\n```\n[[Lain]]\n";
        let out = rewrite_link_target(body, "Lama", "Baru").unwrap();
        // Embeds follow the rename too (an `![[Old]]` transclusion would
        // otherwise break); fenced code is untouched.
        assert_eq!(
            out,
            "a [[Baru]] b [[Baru#Bagian|teks]]\r\n![[Baru]]\n```\n[[Lama]]\n```\n[[Lain]]\n"
        );
    }

    #[test]
    fn display_text_shows_what_a_reader_sees() {
        assert_eq!(
            display_text("Lihat [[Resep#Bahan|bahan]] dan [[Catatan]] ![[foto.png]]"),
            "Lihat bahan dan Catatan foto.png"
        );
        assert_eq!(display_text("tanpa tautan [["), "tanpa tautan [[");
    }

    #[test]
    fn rewrite_link_target_returns_none_when_nothing_links() {
        assert!(rewrite_link_target("tidak ada [[X]]", "Lama", "Baru").is_none());
        assert!(rewrite_link_target("[[Sama]]", "Sama", "Sama").is_none());
    }

    #[test]
    fn unlinked_mentions_finds_plain_whole_word_mentions_only() {
        let body = "Nasi Goreng enak\n[[Nasi Goreng]] sudah\nnasi gorengan bukan\n```\nnasi goreng\n```";
        let found = unlinked_mentions(body, "Nasi Goreng");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 0);
    }

    #[test]
    fn unlinked_mentions_ignores_very_short_titles() {
        assert!(unlinked_mentions("ab ab ab", "ab").is_empty());
    }

    #[test]
    fn link_mention_on_line_wraps_the_mention_with_alias_when_casing_differs() {
        let body = "satu\nsaya suka nasi goreng pedas\n";
        let out = link_mention_on_line(body, 1, "Nasi Goreng").unwrap();
        assert_eq!(out, "satu\nsaya suka [[Nasi Goreng|nasi goreng]] pedas\n");
        let exact = link_mention_on_line("Nasi Goreng!", 0, "Nasi Goreng").unwrap();
        assert_eq!(exact, "[[Nasi Goreng]]!");
        assert!(link_mention_on_line(body, 0, "Nasi Goreng").is_none());
    }

    #[test]
    fn index_resolves_case_insensitively() {
        let dir = tempdir().unwrap();
        let note = Note::create(dir.path(), "Belanja Mingguan", "").unwrap();
        let index = WikilinkIndex::build(&[note.clone()]);

        assert!(index.contains("belanja mingguan"));
        assert_eq!(index.resolve("BELANJA MINGGUAN"), Some(note.path.as_path()));
    }

    #[test]
    fn index_resolves_file_stem_and_aliases_without_overriding_titles() {
        let dir = tempdir().unwrap();
        let mut a = Note::create(dir.path(), "Judul Asli", "").unwrap();
        a.frontmatter.aliases = vec!["alias satu".to_string(), "Judul Lain".to_string()];
        a.save().unwrap();
        // A legacy uuid-named file whose title is "Judul Lain": the title
        // wins over a's alias of the same name.
        let mut b = Note::create(dir.path(), "Judul Lain", "").unwrap();
        let uuid_path = dir.path().join(format!("{}.md", b.frontmatter.id));
        std::fs::rename(&b.path, &uuid_path).unwrap();
        b.path = uuid_path;

        let index = WikilinkIndex::build(&[a.clone(), b.clone()]);
        assert_eq!(index.resolve("Judul Asli"), Some(a.path.as_path()));
        assert_eq!(index.resolve("alias satu"), Some(a.path.as_path()));
        assert_eq!(index.resolve("judul lain"), Some(b.path.as_path()));
        // UUID stems are not offered as link targets.
        assert!(!index.contains(&b.frontmatter.id.to_string()));
        assert_eq!(
            link_keys_for(&a),
            vec!["judul asli".to_string(), "alias satu".to_string(), "judul lain".to_string()]
        );
    }

    #[test]
    fn index_with_files_resolves_pdf_names() {
        let index = WikilinkIndex::build(&[]).with_files(&[PathBuf::from("/v/Laporan.pdf")]);
        assert_eq!(
            index.resolve("laporan.pdf"),
            Some(Path::new("/v/Laporan.pdf"))
        );
    }

    #[test]
    fn index_suggestions_filter_sort_and_limit() {
        let dir = tempdir().unwrap();
        let notes = vec![
            Note::create(dir.path(), "Belanja Mingguan", "").unwrap(),
            Note::create(dir.path(), "Belanja Bulanan", "").unwrap(),
            Note::create(dir.path(), "Resep", "").unwrap(),
            Note::create(dir.path(), "Daftar Belanja", "").unwrap(),
        ];
        let index = WikilinkIndex::build(&notes);

        assert_eq!(
            index.suggestions("bel", 1),
            vec!["Belanja Bulanan".to_string()]
        );
        assert_eq!(
            index.suggestions("bel", 10),
            vec![
                "Belanja Bulanan".to_string(),
                "Belanja Mingguan".to_string(),
                "Daftar Belanja".to_string()
            ]
        );
    }

    #[test]
    fn backlinks_for_finds_referencing_notes_and_excludes_self() {
        let dir = tempdir().unwrap();
        let target = Note::create(dir.path(), "Target", "").unwrap();
        let referrer = Note::create(dir.path(), "Referrer", "Lihat [[Target]].").unwrap();
        let unrelated = Note::create(dir.path(), "Unrelated", "Tidak menaut apa pun.").unwrap();
        let self_referencing =
            Note::create(dir.path(), "Target Duplikat", "[[Target#Bagian]] (harus tetap muncul)")
                .unwrap();

        let notes = vec![target.clone(), referrer.clone(), unrelated, self_referencing.clone()];
        let backlinks = backlinks_for("Target", target.frontmatter.id, &notes);

        let titles: Vec<&str> = backlinks.iter().map(|n| n.frontmatter.title.as_str()).collect();
        assert!(titles.contains(&"Referrer"));
        assert!(titles.contains(&"Target Duplikat"));
        assert_eq!(titles.len(), 2);
    }
}
