//! Text rewrites applied to a block's Markdown before it is rendered
//! (§3.2.2): `[[wikilinks]]` and `#tags` become CommonMark links the viewer
//! can hook, `^anchors` are hidden, `![[Note]]` embeds become quoted
//! transclusions (and sheets preview tables, §3.8.3), and legacy
//! ```` ```canvas ```` fences a readable callout. Pure string logic, shared
//! by the Live view and HTML export. Callers: `markdown::renderer`, `export`.

use super::{EmbedContent, EmbedResolver, slugify};
use crate::markdown::{blocks, wikilink};

/// Nesting depth allowed for note transclusion.
const MAX_EMBED_DEPTH: usize = 3;

/// `![[photo.png]]` (optionally with `|width`) alone on a line.
pub(crate) fn image_embed_target(line: &str) -> Option<&str> {
    let inner = line.strip_prefix("![[")?.strip_suffix("]]")?;
    let name = inner.split('|').next()?.trim();
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp").then_some(name)
}

/// Replaces `![[Note]]` / `![[Note#Heading]]` lines with the target's
/// text as a quote-style callout (Obsidian transclusion), recursively up
/// to `MAX_EMBED_DEPTH`. Unresolved embeds and non-note targets are left
/// for `transform_wikilinks` / image drawing.
pub(crate) fn transform_note_embeds(text: &str, resolve_embed: &EmbedResolver<'_>, depth: usize) -> String {
    if !text.contains("![[") || depth >= MAX_EMBED_DEPTH {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut in_fence = false;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        let t = content.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
        }
        let embedded = if in_fence {
            None
        } else {
            t.strip_prefix("![[")
                .and_then(|r| r.strip_suffix("]]"))
                .and_then(|inner| {
                    let link = wikilink::WikiLink::parse(inner);
                    if link.is_pdf() || image_embed_target(t).is_some() {
                        return None;
                    }
                    match resolve_embed(&link.target)? {
                        EmbedContent::Note { title, body } => {
                            let section = match &link.heading {
                                Some(h) => heading_section(&body, h).unwrap_or(body),
                                None => body,
                            };
                            let nested = transform_note_embeds(
                                &blocks::strip_anchors(&section),
                                resolve_embed,
                                depth + 1,
                            );
                            let mut quoted = format!("> [!quote] [[{title}]]\n");
                            for l in nested.lines() {
                                quoted.push_str("> ");
                                quoted.push_str(l);
                                quoted.push('\n');
                            }
                            Some(quoted)
                        }
                        EmbedContent::Sheet(path) => {
                            Some(crate::markdown::sheet_embed::preview_markdown(&path, &link.target))
                        }
                        EmbedContent::Image(_) => None,
                    }
                })
        };
        match embedded {
            Some(q) => out.push_str(&q),
            None => out.push_str(line),
        }
    }
    out
}

/// The lines under heading `title` (until the next heading of the same
/// or higher level), for `![[Note#Heading]]`.
pub(crate) fn heading_section(body: &str, title: &str) -> Option<String> {
    let wanted = slugify(title);
    let mut level = 0;
    let mut out: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in body.lines() {
        if let Some((l, t)) = super::parse_heading(line.trim_start()) {
            if inside && l <= level {
                break;
            }
            if !inside && slugify(&t) == wanted {
                inside = true;
                level = l;
                out.push(line);
                continue;
            }
        }
        if inside {
            out.push(line);
        }
    }
    inside.then(|| out.join("\n"))
}

pub(crate) fn tag_destination(tag: &str) -> String {
    format!("tag:{tag}")
}

pub(crate) fn wikilink_destination(title: &str) -> String {
    format!("wikilink:{title}")
}

/// Everything inline that CommonMark doesn't know: wikilinks, `#tag`s and
/// block anchors (hidden, as in Obsidian).
pub(crate) fn transform_inline(text: &str, is_resolved: &dyn Fn(&str) -> bool) -> String {
    let stripped = blocks::strip_anchors(text);
    let linked = transform_wikilinks(&stripped, is_resolved);
    transform_tags(&linked)
}

/// Rewrites inline `#tag`s into `[#tag](<tag:tag>)` links (fence- and
/// heading-aware, mirroring `notes::tags::inline_tags`).
pub(crate) fn transform_tags(text: &str) -> String {
    if !text.contains('#') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 32);
    let mut in_fence = false;
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            out.push_str(line);
            continue;
        }
        if in_fence || (t.starts_with('#') && t.chars().find(|c| *c != '#') == Some(' ')) {
            out.push_str(line);
            continue;
        }
        let tags = crate::notes::tags::inline_tags(line);
        if tags.is_empty() {
            out.push_str(line);
            continue;
        }
        // Replace longest tags first so `#a/b` isn't clobbered by `#a`.
        let mut sorted = tags.clone();
        sorted.sort_by_key(|t| std::cmp::Reverse(t.len()));
        let mut rewritten = line.to_string();
        for tag in sorted {
            let needle = format!("#{tag}");
            let mut result = String::with_capacity(rewritten.len());
            let mut rest = rewritten.as_str();
            let mut in_code = false;
            let mut in_link = false;
            while let Some(pos) = rest.find(&needle) {
                let (before, after) = rest.split_at(pos);
                in_code ^= before.matches('`').count() % 2 == 1;
                in_link ^= before.matches("](<").count() != before.matches(">)").count();
                result.push_str(before);
                let prev = before.chars().next_back();
                let next = after[needle.len()..].chars().next();
                let boundary_before = prev.is_none_or(|p| p.is_whitespace() || "([{\"'".contains(p));
                let boundary_after = next.is_none_or(|n| !(n.is_alphanumeric() || matches!(n, '_' | '-' | '/')));
                if boundary_before && boundary_after && !in_code && !in_link {
                    result.push_str(&format!("[{needle}](<{}>)", tag_destination(&tag)));
                } else {
                    result.push_str(&needle);
                }
                rest = &after[needle.len()..];
            }
            result.push_str(rest);
            rewritten = result;
        }
        out.push_str(&rewritten);
    }
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Rewrite `[[Title#Heading|Alias]]` into a real CommonMark link
/// (`[Alias](<wikilink:Title#Heading>)`, angle-bracketed since titles may
/// contain spaces) and `![[name]]` embeds left unresolved into a plain
/// placeholder. Links whose target `is_resolved` rejects get italic link
/// text, Obsidian's cue for "this note doesn't exist yet". Fence-aware,
/// so code blocks are left untouched.
pub(crate) fn transform_wikilinks(text: &str, is_resolved: &dyn Fn(&str) -> bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_fence = false;
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
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
        out.push_str(&transform_wikilinks_in_line(line, is_resolved));
    }
    out
}

fn transform_wikilinks_in_line(line: &str, is_resolved: &dyn Fn(&str) -> bool) -> String {
    let mut out = String::new();
    let mut rest = line;
    loop {
        let Some(start) = rest.find("[[") else {
            out.push_str(rest);
            break;
        };
        let is_embed = start > 0 && rest.as_bytes()[start - 1] == b'!';
        // Copy everything up to (but not including) the wikilink marker.
        // For an embed, also drop the '!' we would otherwise have copied.
        out.push_str(&rest[..if is_embed { start - 1 } else { start }]);

        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else {
            // Unterminated `[[`: treat the rest of the line as plain text.
            out.push_str(&rest[start..]);
            break;
        };
        let link = wikilink::WikiLink::parse(&after[..end]);

        if is_embed {
            out.push_str("📎 ");
            out.push_str(&link.target);
        } else {
            let text = link.alias.clone().unwrap_or_else(|| match &link.heading {
                Some(h) if !link.is_pdf() => format!("{} › {h}", link.target),
                _ => link.target.clone(),
            });
            let marker = if is_resolved(&link.target) { "" } else { "_" };
            out.push('[');
            out.push_str(marker);
            out.push_str(&text);
            out.push_str(marker);
            out.push_str("](<");
            out.push_str(&wikilink_destination(&link.reference()));
            out.push_str(">)");
        }
        rest = &after[end + 2..];
    }
    out
}

/// Flips the `[ ]`/`[x]` marker on body line `line_idx`.
pub(crate) fn toggle_checklist_line(body: &str, line_idx: usize) -> String {
    let had_trailing_newline = body.ends_with('\n');
    let eol = if body.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<String> = body
        .lines()
        .enumerate()
        .map(|(idx, line)| if idx == line_idx { flip_checkbox_marker(line) } else { line.to_string() })
        .collect();
    let mut joined = lines.join(eol);
    if had_trailing_newline {
        joined.push_str(eol);
    }
    joined
}

fn flip_checkbox_marker(line: &str) -> String {
    if let Some(pos) = line.find("[ ]") {
        format!("{}[x]{}", &line[..pos], &line[pos + 3..])
    } else if let Some(pos) = line.find("[x]").or_else(|| line.find("[X]")) {
        format!("{}[ ]{}", &line[..pos], &line[pos + 3..])
    } else {
        line.to_string()
    }
}

/// Transform raw ```canvas ... ``` blocks into a readable callout with the
/// diagram's text (legacy embedded canvases, §Fase 3).
pub(crate) fn transform_canvas_code_blocks(text: &str) -> String {
    if !text.contains("```canvas") {
        return text.to_string();
    }

    let mut result = String::new();
    let mut remaining = text;

    while let Some(start) = remaining.find("```canvas") {
        result.push_str(&remaining[..start]);
        let after_start = &remaining[start + 9..];
        if let Some(end) = after_start.find("```") {
            let json_body = after_start[..end].trim();
            let doc = crate::canvas::CanvasDocument::from_markdown_body("", json_body);
            let summary = doc.summary_text();
            let readable = doc.to_readable_markdown();

            result.push_str("> [!note] 🎨 **Papan Tulis Kanvas (Edgeless)**\n");
            result.push_str(&format!("> *{}*\n", summary));
            if !readable.is_empty() {
                result.push_str(">\n");
                for line in readable.lines() {
                    result.push_str(&format!("> {}\n", line));
                }
            } else {
                result.push_str(">\n> *Kanvas masih kosong. Buka mode 🎨 Edgeless di atas untuk mulai menggambar visual.*\n");
            }
            result.push('\n');

            remaining = &after_start[end + 3..];
        } else {
            result.push_str(&remaining[start..]);
            remaining = "";
            break;
        }
    }
    result.push_str(remaining);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_resolver(target: &str) -> Option<EmbedContent> {
        match target {
            "Anak" => Some(EmbedContent::Note {
                title: "Anak".into(),
                body: "# Bagian A\nisi a ^x1\n\n# Bagian B\nisi b\n![[Cucu]]\n".into(),
            }),
            "Cucu" => Some(EmbedContent::Note {
                title: "Cucu".into(),
                body: "cucu ![[Anak]]".into(),
            }),
            "foto.png" => Some(EmbedContent::Image("/v/foto.png".into())),
            _ => None,
        }
    }

    #[test]
    fn sheet_embeds_become_preview_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Kas.csv");
        std::fs::write(&path, "Item,Harga\nKopi,12000\n").unwrap();
        let resolver = |t: &str| (t == "Kas.csv").then(|| EmbedContent::Sheet(path.clone()));
        let out = transform_note_embeds("awal\n![[Kas.csv]]\nakhir\n", &resolver, 0);
        assert!(out.contains("| Item | Harga |\n| --- | ---: |\n| Kopi | 12000 |\n"), "{out}");
        assert!(out.contains("*[[Kas.csv]] · 1/1 rows · 2 columns*"));
        assert!(out.starts_with("awal\n") && out.ends_with("akhir\n"));
    }

    #[test]
    fn note_embeds_become_quote_callouts_with_depth_limit() {
        let out = transform_note_embeds("awal\n![[Anak#Bagian B]]\n![[Hilang]]\n![[foto.png]]\n", &fake_resolver, 0);
        assert!(out.starts_with("awal\n> [!quote] [[Anak]]\n> # Bagian B\n> isi b\n"), "{out}");
        // Nested embed rendered one level down, then stops recursing.
        assert!(out.contains("> > [!quote] [[Cucu]]"), "{out}");
        assert!(out.contains("![[Hilang]]"), "unresolved embeds are left alone");
        assert!(out.contains("![[foto.png]]"), "images are drawn separately");
        assert!(!out.contains("^x1"));
    }

    #[test]
    fn heading_section_and_image_targets() {
        let body = "intro\n# A\na1\n## A2\na2\n# B\nb1";
        assert_eq!(heading_section(body, "A").as_deref(), Some("# A\na1\n## A2\na2"));
        assert_eq!(heading_section(body, "a2").as_deref(), Some("## A2\na2"));
        assert!(heading_section(body, "Z").is_none());
        assert_eq!(image_embed_target("![[Foto Liburan.JPG|300]]"), Some("Foto Liburan.JPG"));
        assert_eq!(image_embed_target("![[Catatan]]"), None);
    }

    #[test]
    fn inline_tags_render_as_tag_links_outside_code_and_headings() {
        let out = transform_tags("# #bukan heading\nteks #projek/web dan `#kode`\n");
        assert!(out.contains("[#projek/web](<tag:projek/web>)"), "{out}");
        assert!(out.contains("`#kode`"));
        assert!(out.starts_with("# #bukan heading"));
    }

    #[test]
    fn toggle_checklist_line_flips_and_keeps_line_endings() {
        assert_eq!(toggle_checklist_line("- [ ] Beli beras\n- [x] Bayar listrik", 0), "- [x] Beli beras\n- [x] Bayar listrik");
        assert_eq!(toggle_checklist_line("- [ ] Beli beras\n", 0), "- [x] Beli beras\n");
        assert_eq!(toggle_checklist_line("a\r\n- [x] b\r\n", 1), "a\r\n- [ ] b\r\n");
    }

    fn all_resolved(_: &str) -> bool {
        true
    }

    #[test]
    fn transform_wikilinks_rewrites_links_aliases_and_headings() {
        assert_eq!(
            transform_wikilinks("Lihat [[Belanja Mingguan]] ya.", &all_resolved),
            "Lihat [Belanja Mingguan](<wikilink:Belanja Mingguan>) ya."
        );
        assert_eq!(
            transform_wikilinks("[[Belanja Mingguan|daftar belanja]]", &all_resolved),
            "[daftar belanja](<wikilink:Belanja Mingguan>)"
        );
        assert_eq!(
            transform_wikilinks("[[Resep#Bahan]] [[a.pdf#page=2]]", &all_resolved),
            "[Resep › Bahan](<wikilink:Resep#Bahan>) [a.pdf](<wikilink:a.pdf#page=2>)"
        );
    }

    #[test]
    fn transform_wikilinks_italicizes_unresolved_and_skips_fences() {
        assert_eq!(
            transform_wikilinks("[[Ada]] [[Belum Ada]]", &|t| t == "Ada"),
            "[Ada](<wikilink:Ada>) [_Belum Ada_](<wikilink:Belum Ada>)"
        );
        assert_eq!(transform_wikilinks("![[foto.png]]", &all_resolved), "📎 foto.png");
        let body = "```\n[[Bukan Link]]\n```";
        assert_eq!(transform_wikilinks(body, &all_resolved), body);
    }

    #[test]
    fn transform_canvas_code_blocks_renders_alert_and_prose() {
        let raw_canvas_body = "```canvas\n{\n  \"id\": \"00000000-0000-0000-0000-000000000000\",\n  \"title\": \"Diagram\",\n  \"elements\": [],\n  \"viewport\": {\"pan\": [0.0, 0.0], \"zoom\": 1.0}\n}\n```";
        let out = transform_canvas_code_blocks(raw_canvas_body);
        assert!(out.contains("> [!note] 🎨 **Papan Tulis Kanvas (Edgeless)**"));
        assert!(out.contains("Kanvas masih kosong"));
        assert!(!out.contains("```canvas"));
    }
}
