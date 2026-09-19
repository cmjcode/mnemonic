//! Note → standalone HTML in a reading theme's print colours (§3.2.5).
//! The page carries its own CSS (theme colours as `--mn-*` variables, with
//! `print-color-adjust: exact` so browsers keep backgrounds and accents
//! when printing), inline SVG for Mermaid fences, `data:` URIs for images,
//! and Obsidian syntax the CommonMark parser doesn't know rewritten first:
//! wikilinks, `#tags`, callouts, `==highlights==`, transclusions.
//! Pure apart from reading embedded image files. Callers: `export`.

use base64::Engine;
use pulldown_cmark::{CodeBlockKind, CowStr, Event, Options, Parser, Tag, TagEnd};

use crate::markdown::renderer::transform::{
    image_embed_target, transform_canvas_code_blocks, transform_inline, transform_note_embeds,
};
use crate::markdown::renderer::{EmbedContent, EmbedResolver};
use crate::reading_theme::{ReadingTheme, ThemeColors};

/// Options for one exported page.
#[derive(Debug, Clone, Default)]
pub struct HtmlOptions {
    /// Open the browser's print dialog as soon as the page loads.
    pub auto_print: bool,
    /// `lang` attribute, e.g. `id` or `en`.
    pub lang: String,
}

/// The note as a complete HTML document in `theme`'s print colours.
pub fn note_document(title: &str, body: &str, theme: &ReadingTheme, resolve_embed: &EmbedResolver<'_>, opts: &HtmlOptions) -> String {
    let colors = &theme.print;
    let content = body_html(body, colors, resolve_embed);
    let lang = if opts.lang.is_empty() { "en" } else { opts.lang.as_str() };
    let script = if opts.auto_print {
        "<script>window.addEventListener('load',()=>setTimeout(()=>window.print(),300));</script>"
    } else {
        ""
    };
    format!(
        "<!doctype html>\n<html lang=\"{lang}\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<meta name=\"generator\" content=\"MNEMONIC\">\n<meta name=\"mnemonic-theme\" content=\"{theme_id}\">\n<title>{title}</title>\n<style>\n{css}</style>\n</head>\n<body>\n<article class=\"mn-note\">\n<h1 class=\"mn-title\">{title}</h1>\n{content}</article>\n{script}\n</body>\n</html>\n",
        title = escape(title),
        theme_id = escape(&theme.id),
        css = stylesheet(colors),
    )
}

/// CSS for a colour set: every colour as a `--mn-<field>` variable, then
/// the rules that use them.
pub fn stylesheet(c: &ThemeColors) -> String {
    let mut css = String::from(":root {\n");
    for (name, color) in c.entries() {
        css.push_str(&format!("  --mn-{}: {};\n", name.replace('_', "-"), color.hex()));
    }
    css.push_str("}\n");
    css.push_str(BASE_CSS);
    css
}

const BASE_CSS: &str = r#"@page { margin: 0; }
html { -webkit-print-color-adjust: exact; print-color-adjust: exact; background: var(--mn-background); }
body { margin: 0; background: var(--mn-background); color: var(--mn-text);
  font: 15px/1.65 Inter, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif; }
.mn-note { max-width: 760px; margin: 0 auto; padding: 40px 32px 64px; }
@media print {
  /* The page margin is padding on every page fragment, so the theme's
     paper colour reaches the edges. */
  .mn-note { max-width: none; padding: 16mm 16mm; box-decoration-break: clone; -webkit-box-decoration-break: clone; }
}
.mn-title { font-size: 2.1em; margin: 0 0 .6em; color: var(--mn-h1); }
h1, h2, h3, h4, h5, h6 { line-height: 1.25; margin: 1.3em 0 .45em; font-weight: 650; break-after: avoid; }
h1 { font-size: 1.85em; color: var(--mn-h1); }
h2 { font-size: 1.5em; color: var(--mn-h2); }
h3 { font-size: 1.25em; color: var(--mn-h3); }
h4 { font-size: 1.1em; color: var(--mn-h4); }
h5 { font-size: 1em; color: var(--mn-h5); }
h6 { font-size: .95em; color: var(--mn-h6); }
p { margin: .5em 0; }
strong { color: var(--mn-strong); }
a { color: var(--mn-link); }
.wikilink { color: var(--mn-link); text-decoration: underline dotted; text-underline-offset: 2px; }
.tag { color: var(--mn-tag); background: color-mix(in srgb, var(--mn-tag) 13%, transparent);
  border-radius: 999px; padding: 0 .5em; font-size: .92em; }
code { font-family: "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: .88em;
  background: var(--mn-code-bg); color: var(--mn-code-text); padding: .1em .35em; border-radius: 4px; }
pre { background: var(--mn-code-bg); padding: 12px 14px; border-radius: 8px; overflow-x: auto; break-inside: avoid; }
pre code { padding: 0; background: none; }
blockquote { border-left: 3px solid var(--mn-quote-bar); color: var(--mn-muted); margin: .8em 0; padding: .1em 1em; }
hr { border: none; border-top: 1.5px solid var(--mn-rule); margin: 1.4em 0; }
table { border-collapse: collapse; width: 100%; margin: .8em 0; break-inside: avoid; }
th, td { border: 1px solid var(--mn-table-border); padding: 6px 10px; text-align: left; }
th { background: var(--mn-table-header-bg); }
mark { background: var(--mn-highlight-bg); color: inherit; padding: 0 .15em; border-radius: 3px; }
ul, ol { padding-left: 1.6em; }
li::marker { color: var(--mn-strong); }
li:has(.task) { list-style: none; margin-left: -1.4em; }
.task { display: inline-block; box-sizing: border-box; width: 1.05em; height: 1.05em; margin-right: .5em; vertical-align: -.15em;
  border: 1.5px solid var(--mn-muted); border-radius: 4px; color: #fff; font-size: .85em; line-height: .95em; text-align: center; }
.task.done { background: var(--mn-checkbox); border-color: var(--mn-checkbox); }
li:has(.task.done) { color: var(--mn-muted); text-decoration: line-through; }
.callout { --accent: var(--mn-callout-note); border-left: 4px solid var(--accent); border-radius: 6px;
  background: color-mix(in srgb, var(--accent) 9%, var(--mn-background)); padding: .55em 1em; margin: .9em 0; break-inside: avoid; }
.callout-title { color: var(--accent); font-weight: 650; }
.callout-title p { display: inline; margin: 0; }
.math { font-style: italic; }
.math.display { display: block; text-align: center; margin: .8em 0; font-size: 1.15em; }
figure.diagram { margin: 1em 0; text-align: center; break-inside: avoid; }
figure.diagram svg { max-width: 100%; height: auto; }
img { max-width: 100%; border-radius: 4px; }
"#;

/// The note body as HTML (no page wrapper).
pub fn body_html(body: &str, colors: &ThemeColors, resolve_embed: &EmbedResolver<'_>) -> String {
    let text = transform_canvas_code_blocks(body);
    let text = transform_note_embeds(&text, resolve_embed, 0);
    let text = image_embeds_to_markdown(&text);
    let text = transform_inline(&text, &|_| true);
    let text = highlights_to_mark(&text);
    let text = callouts_to_html(&text, colors, resolve_embed);
    markdown_to_html(&text, resolve_embed)
}

fn parser_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_MATH
}

/// CommonMark → HTML, rewriting what only MNEMONIC understands: link
/// destinations `wikilink:`/`tag:` (from `transform_inline`), `embed:`
/// images, Mermaid fences and math.
fn markdown_to_html(text: &str, resolve_embed: &EmbedResolver<'_>) -> String {
    let mut events: Vec<Event<'_>> = Vec::new();
    let mut link_closers: Vec<Option<&'static str>> = Vec::new();
    let mut mermaid: Option<String> = None;
    for event in Parser::new_ext(text, parser_options()) {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) if lang.trim() == "mermaid" => {
                mermaid = Some(String::new());
            }
            Event::Text(t) if mermaid.is_some() => {
                if let Some(src) = mermaid.as_mut() {
                    src.push_str(&t);
                }
            }
            Event::End(TagEnd::CodeBlock) if mermaid.is_some() => {
                let source = mermaid.take().unwrap_or_default();
                events.push(Event::Html(CowStr::from(mermaid_figure(&source))));
            }
            Event::Start(Tag::Link { dest_url, .. }) if dest_url.starts_with("wikilink:") => {
                link_closers.push(Some("</span>"));
                events.push(Event::InlineHtml(CowStr::from("<span class=\"wikilink\">")));
            }
            Event::Start(Tag::Link { dest_url, .. }) if dest_url.starts_with("tag:") => {
                link_closers.push(Some("</span>"));
                events.push(Event::InlineHtml(CowStr::from("<span class=\"tag\">")));
            }
            Event::Start(tag @ Tag::Link { .. }) => {
                link_closers.push(None);
                events.push(Event::Start(tag));
            }
            Event::End(TagEnd::Link) => match link_closers.pop().flatten() {
                Some(close) => events.push(Event::InlineHtml(CowStr::from(close))),
                None => events.push(Event::End(TagEnd::Link)),
            },
            Event::Start(Tag::Image { link_type, dest_url, title, id }) => {
                let dest = image_data_uri(&dest_url, resolve_embed).map(CowStr::from).unwrap_or(dest_url);
                events.push(Event::Start(Tag::Image { link_type, dest_url: dest, title, id }));
            }
            Event::TaskListMarker(done) => events.push(Event::InlineHtml(CowStr::from(if done {
                "<span class=\"task done\">✓</span>"
            } else {
                "<span class=\"task\"></span>"
            }))),
            Event::InlineMath(tex) => events.push(Event::InlineHtml(CowStr::from(format!(
                "<span class=\"math\">{}</span>",
                escape(&crate::markdown::math::to_unicode(&tex))
            )))),
            Event::DisplayMath(tex) => events.push(Event::Html(CowStr::from(format!(
                "<span class=\"math display\">{}</span>",
                escape(&crate::markdown::math::to_unicode(&tex))
            )))),
            other => events.push(other),
        }
    }
    let mut html = String::with_capacity(text.len() * 2);
    pulldown_cmark::html::push_html(&mut html, events.into_iter());
    html
}

/// A Mermaid source as an inline SVG figure (light theme, like paper), or
/// the source as a code block when it can't be drawn.
fn mermaid_figure(source: &str) -> String {
    let measure = crate::mermaid::text::ApproxMeasure;
    let rendered = crate::mermaid::render(source, &crate::mermaid::RenderOptions { dark: false, measure: &measure });
    match rendered.scene {
        Some(scene) => format!("<figure class=\"diagram\">{}</figure>\n", crate::mermaid::svg::to_svg(&scene)),
        None => format!("<pre><code class=\"language-mermaid\">{}</code></pre>\n", escape(source)),
    }
}

/// `data:` URI for an image destination that names a vault file
/// (`embed:photo.png` from `![[photo.png]]`, or a relative path).
fn image_data_uri(dest: &str, resolve_embed: &EmbedResolver<'_>) -> Option<String> {
    if dest.contains("://") || dest.starts_with("data:") {
        return None;
    }
    let name = dest.strip_prefix("embed:").unwrap_or(dest);
    let file = percent_encoding::percent_decode_str(name).decode_utf8_lossy().to_string();
    let file_name = std::path::Path::new(&file).file_name()?.to_string_lossy().to_string();
    let Some(EmbedContent::Image(path)) = resolve_embed(&file_name) else {
        return None;
    };
    let mime = match path.extension()?.to_string_lossy().to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => return None,
    };
    let bytes = std::fs::read(&path)
        .map_err(|e| log::warn!("export: cannot read image {}: {e}", path.display()))
        .ok()?;
    Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// `![[photo.png]]` lines → `![photo.png](<embed:photo.png>)`.
fn image_embeds_to_markdown(text: &str) -> String {
    map_outside_fences(text, |line| {
        image_embed_target(line.trim()).map(|name| format!("![{name}](<embed:{name}>)"))
    })
}

/// `==text==` → `<mark>text</mark>`, outside code spans and fences.
fn highlights_to_mark(text: &str) -> String {
    map_outside_fences(text, |line| {
        if !line.contains("==") {
            return None;
        }
        let mut out = String::with_capacity(line.len() + 16);
        let mut open = false;
        for (i, part) in line.split('`').enumerate() {
            if i > 0 {
                out.push('`');
            }
            if i % 2 == 1 {
                out.push_str(part);
                continue;
            }
            let mut rest = part;
            while let Some(pos) = rest.find("==") {
                out.push_str(&rest[..pos]);
                out.push_str(if open { "</mark>" } else { "<mark>" });
                open = !open;
                rest = &rest[pos + 2..];
            }
            out.push_str(rest);
        }
        // An unpaired `==` leaves the line as written.
        (!open).then_some(out)
    })
}

/// Rewrites `> [!type] Title` callouts (with their `>` lines) into HTML
/// blocks the parser keeps, rendering title and content as Markdown.
/// Recursive, so callouts nested in callouts (transclusions) work.
fn callouts_to_html(text: &str, colors: &ThemeColors, resolve_embed: &EmbedResolver<'_>) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = String::with_capacity(text.len() + 64);
    let mut in_fence = false;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
        }
        let head = t
            .strip_prefix('>')
            .filter(|_| !in_fence)
            .map(str::trim_start)
            .and_then(|rest| rest.strip_prefix("[!"))
            .and_then(|rest| rest.split_once(']'));
        let Some((kind, title)) = head else {
            out.push_str(line);
            out.push('\n');
            i += 1;
            continue;
        };
        let kind = kind.trim().to_ascii_lowercase();
        let title = title.trim_start_matches(['+', '-']).trim();
        let mut inner = Vec::new();
        i += 1;
        while i < lines.len() && lines[i].trim_start().starts_with('>') {
            let rest = &lines[i].trim_start()[1..];
            inner.push(rest.strip_prefix(' ').unwrap_or(rest));
            i += 1;
        }
        let title_md = if title.is_empty() { capitalize(&kind) } else { title.to_string() };
        let title_html = markdown_to_html(&title_md, resolve_embed);
        let inner_md = callouts_to_html(&inner.join("\n"), colors, resolve_embed);
        out.push_str(&format!(
            "<div class=\"callout callout-{kind_class}\" style=\"--accent: {accent}\">\n<div class=\"callout-title\">{title}</div>\n\n{inner}\n\n</div>\n\n",
            kind_class = escape(&kind),
            accent = colors.callout(&kind).hex(),
            title = title_html.trim(),
            inner = inner_md.trim_end(),
        ));
    }
    out
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// Applies `f` to every line outside fenced code (`None` keeps the line).
fn map_outside_fences(text: &str, f: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
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
        match (!in_fence).then(|| f(line)).flatten() {
            Some(replaced) => out.push_str(&replaced),
            None => out.push_str(line),
        }
    }
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Minimal HTML escaping for text and attribute values.
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reading_theme::{MNEMONIC_LIGHT, ThemeRegistry};

    fn no_embeds(_: &str) -> Option<EmbedContent> {
        None
    }

    #[test]
    fn document_carries_the_theme_print_colours() {
        let reg = ThemeRegistry::default();
        let theme = reg.get("pelangi");
        let html = note_document("Resep <Kue>", "# Bahan\nteks", theme, &no_embeds, &HtmlOptions::default());
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<title>Resep &lt;Kue&gt;</title>"));
        assert!(html.contains(&format!("--mn-h1: {};", theme.print.h1.hex())));
        assert!(html.contains("print-color-adjust: exact"));
        assert!(html.contains("<h1>Bahan</h1>"));
        assert!(!html.contains("window.print"));
        let printing = note_document("x", "", theme, &no_embeds, &HtmlOptions { auto_print: true, lang: "id".into() });
        assert!(printing.contains("window.print()") && printing.contains("lang=\"id\""));
    }

    #[test]
    fn obsidian_syntax_becomes_styled_html() {
        let body = "Lihat [[Resep Kue|resep]] #dapur dan ==penting== `==kode==`\n- [x] selesai\n- [ ] belum";
        let html = body_html(body, &MNEMONIC_LIGHT, &no_embeds);
        assert!(html.contains("<span class=\"wikilink\">resep</span>"), "{html}");
        assert!(html.contains("<span class=\"tag\">#dapur</span>"), "{html}");
        assert!(html.contains("<mark>penting</mark>"), "{html}");
        assert!(html.contains("<code>==kode==</code>"), "{html}");
        assert!(html.contains("<span class=\"task done\">✓</span>selesai"), "{html}");
        assert!(html.contains("<span class=\"task\"></span>belum"), "{html}");
        assert!(!html.contains("wikilink:"), "internal link scheme leaked: {html}");
    }

    #[test]
    fn callouts_get_their_theme_accent_and_markdown_content() {
        let c = MNEMONIC_LIGHT;
        let html = body_html("> [!warning] Hati-**hati**\n> isi *miring*\n\nsesudah", &c, &no_embeds);
        assert!(html.contains(&format!("style=\"--accent: {}\"", c.callout_warning.hex())), "{html}");
        assert!(html.contains("Hati-<strong>hati</strong>"), "{html}");
        assert!(html.contains("<em>miring</em>"), "{html}");
        assert!(html.contains("<p>sesudah</p>"), "{html}");
        let untitled = body_html("> [!tip]\n> x", &c, &no_embeds);
        assert!(untitled.contains("Tip"), "{untitled}");
    }

    #[test]
    fn mermaid_becomes_svg_and_math_unicode() {
        let html = body_html("```mermaid\nflowchart LR\n  A-->B\n```\n\n$x^2$", &MNEMONIC_LIGHT, &no_embeds);
        assert!(html.contains("<figure class=\"diagram\"><svg"), "{html}");
        assert!(html.contains("<span class=\"math\">x²</span>"), "{html}");
        let broken = body_html("```mermaid\nnonsense\n```", &MNEMONIC_LIGHT, &no_embeds);
        assert!(broken.contains("language-mermaid"), "{broken}");
    }

    #[test]
    fn images_are_inlined_as_data_uris() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("foto.png");
        std::fs::write(&png, [0x89, b'P', b'N', b'G']).unwrap();
        let resolve = |name: &str| (name == "foto.png").then(|| EmbedContent::Image(png.clone()));
        let html = body_html(
            "![[foto.png]]\n\n![alt](attachments/foto.png)\n\n![web](https://x.test/a.png)",
            &MNEMONIC_LIGHT,
            &resolve,
        );
        assert_eq!(html.matches("src=\"data:image/png;base64,iVBORw==\"").count(), 2, "{html}");
        assert!(html.contains("https://x.test/a.png"));
    }

    #[test]
    fn transclusions_render_as_quote_callouts() {
        let resolve =
            |name: &str| (name == "Anak").then(|| EmbedContent::Note { title: "Anak".into(), body: "isi **anak**".into() });
        let html = body_html("![[Anak]]", &MNEMONIC_LIGHT, &resolve);
        assert!(html.contains("callout-quote"), "{html}");
        assert!(html.contains("isi <strong>anak</strong>"), "{html}");
    }
}
