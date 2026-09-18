//! Text measurement and label normalisation for Mermaid layout (§3.7.3).
//!
//! Mermaid.js measures every label through the browser DOM (`getBBox`),
//! which forces a reflow per label and dominates its render time. Here
//! layout only needs a width per string, so it goes through `TextMeasure`:
//! `ApproxMeasure` (a per-class character-width table, deterministic, used
//! by the CLI/SVG path and tests) or `GlyphTable` (real egui glyph widths,
//! collected once per diagram on the UI thread, then usable from any
//! thread). Labels are normalised here too: `<br>` → newline, `#35;`/
//! `&amp;` entities, stripped inline HTML and markdown-string markers.
//! Callers: every diagram layout, `mermaid::paint`.

use std::collections::HashMap;

/// Width of text at a given font size, in the same units as layout.
pub trait TextMeasure {
    /// Width of one line (no newlines) at `size` px.
    fn line_width(&self, line: &str, size: f32) -> f32;

    /// Size of multi-line text (lines separated by `\n`).
    fn size(&self, text: &str, size: f32) -> (f32, f32) {
        let mut w: f32 = 0.0;
        let mut lines = 0;
        for line in text.split('\n') {
            w = w.max(self.line_width(line, size));
            lines += 1;
        }
        (w, lines as f32 * line_height(size))
    }
}

pub fn line_height(size: f32) -> f32 {
    size * 1.25
}

/// Font-free approximation: good to a few percent for proportional sans
/// fonts, and fully deterministic (snapshot tests, CLI SVG export).
#[derive(Debug, Clone, Copy, Default)]
pub struct ApproxMeasure;

impl TextMeasure for ApproxMeasure {
    fn line_width(&self, line: &str, size: f32) -> f32 {
        line.chars().map(approx_char_em).sum::<f32>() * size
    }
}

fn approx_char_em(c: char) -> f32 {
    match c {
        'i' | 'l' | 'j' | '|' | '!' | '\'' | '.' | ',' | ':' | ';' => 0.28,
        'f' | 't' | 'r' | 'I' | '(' | ')' | '[' | ']' | ' ' | '"' | '-' => 0.36,
        'm' | 'w' | 'M' | 'W' | '@' | '%' => 0.86,
        'A'..='Z' => 0.66,
        '0'..='9' => 0.56,
        c if c.is_ascii() => 0.53,
        // CJK and most symbols/emoji are full-width.
        c if (c as u32) >= 0x2E80 => 1.0,
        _ => 0.6,
    }
}

/// Real glyph widths at `BASE_SIZE`, scaled linearly. Built by
/// `mermaid::paint::glyph_table` from the egui font atlas for exactly the
/// characters a diagram uses; unknown characters fall back to
/// `ApproxMeasure`.
#[derive(Debug, Clone, Default)]
pub struct GlyphTable {
    widths: HashMap<char, f32>,
}

impl GlyphTable {
    pub const BASE_SIZE: f32 = 16.0;

    pub fn new(widths: HashMap<char, f32>) -> GlyphTable {
        GlyphTable { widths }
    }
}

impl TextMeasure for GlyphTable {
    fn line_width(&self, line: &str, size: f32) -> f32 {
        let scale = size / Self::BASE_SIZE;
        line.chars()
            .map(|c| {
                self.widths
                    .get(&c)
                    .map_or(approx_char_em(c) * size, |w| w * scale)
            })
            .sum()
    }
}

/// Greedy word wrap of `text` to `max_width`; existing newlines are kept.
pub fn wrap(text: &str, measure: &dyn TextMeasure, size: f32, max_width: f32) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for (i, para) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if !line.is_empty() && measure.line_width(&candidate, size) > max_width {
                out.push_str(&line);
                out.push('\n');
                line = word.to_string();
            } else {
                line = candidate;
            }
        }
        out.push_str(&line);
    }
    out
}

/// Normalise a raw Mermaid label: strip surrounding quotes and markdown
/// string backticks, `<br>` → newline, entity codes → chars, other inline
/// HTML tags and `**`/`*`/`_` emphasis markers dropped.
pub fn clean_label(raw: &str) -> String {
    let mut s = raw.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s = &s[1..s.len() - 1];
    }
    let markdown = s.len() >= 2 && s.starts_with('`') && s.ends_with('`');
    if markdown {
        s = &s[1..s.len() - 1];
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        if c == '<'
            && let Some(end) = rest.find('>')
        {
            let tag = rest[1..end].trim().trim_end_matches('/').trim().to_ascii_lowercase();
            if tag == "br" {
                out.push('\n');
            }
            // Other tags (<b>, <i>, <span …>) are presentation only.
            if tag.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '/') {
                rest = &rest[end + 1..];
                continue;
            }
        }
        if (c == '#' || c == '&')
            && let Some((decoded, used)) = decode_entity(rest)
        {
            out.push(decoded);
            rest = &rest[used..];
            continue;
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    if markdown {
        out = out.replace("**", "").replace("__", "");
        out = strip_single_emphasis(&out);
    }
    // Mermaid also accepts a literal `\n` in markdown strings / labels.
    out.replace("\\n", "\n")
}

fn strip_single_emphasis(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == '*' || c == '_' {
            let prev_word = i > 0 && chars[i - 1].is_alphanumeric();
            let next_word = chars.get(i + 1).is_some_and(|n| n.is_alphanumeric());
            // Keep snake_case underscores; drop emphasis delimiters.
            if !(prev_word && next_word) {
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// `#35;` / `#quot;` (Mermaid) and `&amp;` / `&#35;` (HTML) → (char, bytes used).
fn decode_entity(s: &str) -> Option<(char, usize)> {
    let semi = s.find(';')?;
    if semi > 12 {
        return None;
    }
    let body = &s[1..semi];
    let body = body.strip_prefix('#').unwrap_or(body);
    let c = if let Some(hex) = body.strip_prefix('x').or_else(|| body.strip_prefix('X')) {
        char::from_u32(u32::from_str_radix(hex, 16).ok()?)?
    } else if body.chars().all(|c| c.is_ascii_digit()) && !body.is_empty() {
        char::from_u32(body.parse().ok()?)?
    } else {
        match body {
            "quot" => '"',
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "apos" => '\'',
            "nbsp" => '\u{a0}',
            "hearts" => '♥',
            "copy" => '©',
            "rarr" => '→',
            "larr" => '←',
            "deg" => '°',
            _ => return None,
        }
    };
    Some((c, semi + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_label_handles_br_entities_html_and_markdown() {
        assert_eq!(clean_label("\"a<br/>b\""), "a\nb");
        assert_eq!(clean_label("A #35; B #quot;x#quot;"), "A # B \"x\"");
        assert_eq!(clean_label("<b>bold</b> &amp; plain"), "bold & plain");
        assert_eq!(clean_label("\"`**Big** and *small* snake_case`\""), "Big and small snake_case");
        assert_eq!(clean_label("a < b"), "a < b");
    }

    #[test]
    fn approx_measure_scales_with_size_and_lines() {
        let m = ApproxMeasure;
        let w16 = m.line_width("Hello", 16.0);
        assert!((m.line_width("Hello", 32.0) - 2.0 * w16).abs() < 1e-3);
        let (w, h) = m.size("Hello\nHello world", 16.0);
        assert!(w > w16);
        assert!((h - 2.0 * line_height(16.0)).abs() < 1e-3);
    }

    #[test]
    fn wrap_breaks_long_lines_on_spaces() {
        let m = ApproxMeasure;
        let wrapped = wrap("one two three four five six", &m, 16.0, 60.0);
        assert!(wrapped.lines().count() > 1);
        assert_eq!(wrapped.replace('\n', " "), "one two three four five six");
    }

    #[test]
    fn glyph_table_uses_known_widths() {
        let t = GlyphTable::new(HashMap::from([('a', 10.0)]));
        assert!((t.line_width("aa", 16.0) - 20.0).abs() < 1e-3);
        assert!((t.line_width("aa", 8.0) - 10.0).abs() < 1e-3);
    }
}
