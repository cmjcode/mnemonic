//! Diagram-independent preprocessing of a Mermaid source (§3.7.2): YAML
//! frontmatter (`---\ntitle: …\nconfig: …\n---`), `%%{init: …}%%`
//! directives, `%%` comments, blank lines and the accessibility statements
//! (`accTitle:` / `accDescr:` / `accDescr { … }`) every diagram type
//! accepts. What remains is a list of numbered lines for the per-type
//! parser, so every diagnostic still points at the user's real line.
//! Callers: `mermaid::render`/`mermaid::validate`, every diagram parser.

use serde_json::Value;

use super::diag::Diagnostic;

/// One meaningful source line. `no` is 1-based; `text` is trimmed on both
/// sides; `indent` is the leading-whitespace width (tab = 4), which the
/// indentation-sensitive diagrams (mindmap, treemap, kanban) need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line<'a> {
    pub no: usize,
    pub indent: usize,
    pub text: &'a str,
}

/// Merged configuration: frontmatter `config:` first, then every
/// `%%{init}%%` directive in order (later wins), as Mermaid does.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    pub root: Value,
}

impl Config {
    pub fn get(&self, path: &[&str]) -> Option<&Value> {
        let mut v = &self.root;
        for key in path {
            v = v.get(*key)?;
        }
        Some(v)
    }

    pub fn str(&self, path: &[&str]) -> Option<&str> {
        self.get(path)?.as_str()
    }

    /// Numbers, or strings such as `"16px"`.
    pub fn f32(&self, path: &[&str]) -> Option<f32> {
        match self.get(path)? {
            Value::Number(n) => n.as_f64().map(|v| v as f32),
            Value::String(s) => leading_number(s),
            _ => None,
        }
    }

    pub fn bool(&self, path: &[&str]) -> Option<bool> {
        match self.get(path)? {
            Value::Bool(b) => Some(*b),
            Value::String(s) => match s.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    fn merge(&mut self, other: Value) {
        deep_merge(&mut self.root, other);
    }
}

/// `"16px"` → 16.0, `"1.5"` → 1.5.
pub fn leading_number(s: &str) -> Option<f32> {
    let s = s.trim();
    let end = s
        .char_indices()
        .find(|(i, c)| !(c.is_ascii_digit() || *c == '.' || (*i == 0 && (*c == '-' || *c == '+'))))
        .map_or(s.len(), |(i, _)| i);
    s[..end].parse().ok()
}

fn deep_merge(base: &mut Value, other: Value) {
    match (base, other) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                deep_merge(a.entry(k).or_insert(Value::Null), v);
            }
        }
        (slot, v) => *slot = v,
    }
}

/// A preprocessed diagram source.
#[derive(Debug, Clone, Default)]
pub struct Source<'a> {
    pub lines: Vec<Line<'a>>,
    /// Frontmatter `title:` (per-type `title …` statements are parsed by
    /// the diagram parsers and override this).
    pub title: Option<String>,
    pub acc_title: Option<String>,
    pub acc_descr: Option<String>,
    pub config: Config,
    pub diagnostics: Vec<Diagnostic>,
}

impl<'a> Source<'a> {
    /// First line (the diagram header such as `flowchart LR`), if any.
    pub fn header(&self) -> Option<&Line<'a>> {
        self.lines.first()
    }

    /// Lines after the header.
    pub fn body(&self) -> &[Line<'a>] {
        self.lines.get(1..).unwrap_or(&[])
    }
}

pub fn preprocess(src: &str) -> Source<'_> {
    let mut out = Source::default();
    let raw: Vec<&str> = src.lines().collect();
    let mut i = 0;

    // Frontmatter: must be the first non-blank line.
    while i < raw.len() && raw[i].trim().is_empty() {
        i += 1;
    }
    if i < raw.len() && raw[i].trim() == "---" {
        let start = i;
        let close = (start + 1..raw.len()).find(|&j| raw[j].trim() == "---");
        match close {
            Some(end) => {
                let yaml = raw[start + 1..end].join("\n");
                match serde_yaml::from_str::<Value>(&yaml) {
                    Ok(Value::Object(map)) => {
                        if let Some(Value::String(t)) = map.get("title") {
                            out.title = Some(t.clone());
                        }
                        if let Some(cfg) = map.get("config") {
                            out.config.merge(cfg.clone());
                        }
                    }
                    Ok(Value::Null) => {}
                    Ok(_) => out
                        .diagnostics
                        .push(Diagnostic::warning(start + 2, 1, "frontmatter is not a YAML mapping; ignored")),
                    Err(e) => out
                        .diagnostics
                        .push(Diagnostic::error(start + 2, 1, format!("invalid frontmatter YAML: {e}"))),
                }
                i = end + 1;
            }
            None => {
                out.diagnostics
                    .push(Diagnostic::error(start + 1, 1, "frontmatter `---` is never closed"));
                i = raw.len();
            }
        }
    }

    while i < raw.len() {
        let line = raw[i];
        let no = i + 1;
        let trimmed = line.trim();
        i += 1;
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("%%{") {
            // Directives may span lines until `}%%`.
            let mut text = trimmed.to_string();
            while !text.contains("}%%") && i < raw.len() {
                text.push('\n');
                text.push_str(raw[i].trim());
                i += 1;
            }
            parse_directive(&text, no, &mut out);
            continue;
        }
        if trimmed.starts_with("%%") {
            continue;
        }
        if let Some(rest) = keyword_value(trimmed, "accTitle") {
            out.acc_title = Some(rest.to_string());
            continue;
        }
        if let Some(rest) = keyword_value(trimmed, "accDescr") {
            out.acc_descr = Some(rest.to_string());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("accDescr")
            && rest.trim_start().starts_with('{')
        {
            let mut text = rest.trim_start()[1..].to_string();
            while !text.contains('}') && i < raw.len() {
                text.push('\n');
                text.push_str(raw[i].trim());
                i += 1;
            }
            let body = text.split('}').next().unwrap_or("").trim().to_string();
            out.acc_descr = Some(body);
            continue;
        }
        out.lines.push(Line { no, indent: indent_width(line), text: trimmed });
    }
    out
}

/// `accTitle: text` → `Some("text")`.
fn keyword_value<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(keyword)?.trim_start();
    Some(rest.strip_prefix(':')?.trim())
}

fn indent_width(line: &str) -> usize {
    line.chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
}

fn parse_directive(text: &str, no: usize, out: &mut Source<'_>) {
    let inner = text
        .trim()
        .strip_prefix("%%{")
        .and_then(|t| t.rsplit_once("}%%").map(|(a, _)| a))
        .unwrap_or("");
    // Directive bodies are JSON-ish (`init: {'theme': 'forest'}`) — a YAML
    // flow mapping accepts unquoted keys and single quotes alike.
    let yaml = format!("{{{inner}}}");
    match serde_yaml::from_str::<Value>(&yaml) {
        Ok(Value::Object(map)) => {
            for (key, value) in map {
                match key.as_str() {
                    "init" | "initialize" | "config" => out.config.merge(value),
                    "wrap" => out.config.merge(serde_json::json!({ "wrap": true })),
                    _ => out
                        .diagnostics
                        .push(Diagnostic::warning(no, 1, format!("unknown directive `{key}` ignored"))),
                }
            }
        }
        Ok(_) | Err(_) => {
            if inner.trim() == "wrap" {
                out.config.merge(serde_json::json!({ "wrap": true }));
            } else {
                out.diagnostics
                    .push(Diagnostic::error(no, 1, "unparseable `%%{…}%%` directive"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_frontmatter_directives_comments_and_keeps_line_numbers() {
        let src = "---\ntitle: Hello\nconfig:\n  theme: forest\n---\n%%{init: {'flowchart': {'curve': 'linear'}}}%%\nflowchart LR\n  %% a comment\n\n  A --> B\n";
        let s = preprocess(src);
        assert_eq!(s.title.as_deref(), Some("Hello"));
        assert_eq!(s.config.str(&["theme"]), Some("forest"));
        assert_eq!(s.config.str(&["flowchart", "curve"]), Some("linear"));
        let texts: Vec<(usize, &str)> = s.lines.iter().map(|l| (l.no, l.text)).collect();
        assert_eq!(texts, vec![(7, "flowchart LR"), (10, "A --> B")]);
        assert_eq!(s.lines[1].indent, 2);
        assert!(s.diagnostics.is_empty());
    }

    #[test]
    fn later_directive_overrides_frontmatter() {
        let src = "---\nconfig:\n  theme: dark\n---\n%%{init: {\"theme\": \"neutral\"}}%%\npie\n";
        assert_eq!(preprocess(src).config.str(&["theme"]), Some("neutral"));
    }

    #[test]
    fn accessibility_statements_are_consumed() {
        let src = "pie\naccTitle: Pets\naccDescr {\n multi\n line }\n\"Dogs\" : 3\n";
        let s = preprocess(src);
        assert_eq!(s.acc_title.as_deref(), Some("Pets"));
        assert_eq!(s.acc_descr.as_deref(), Some("multi\nline"));
        assert_eq!(s.lines.len(), 2);
    }

    #[test]
    fn unclosed_frontmatter_is_an_error_not_a_panic() {
        let s = preprocess("---\ntitle: x\nflowchart LR\n");
        assert!(s.diagnostics.iter().any(|d| d.is_error()));
        assert!(s.lines.is_empty());
    }

    #[test]
    fn leading_number_parses_units() {
        assert_eq!(leading_number("16px"), Some(16.0));
        assert_eq!(leading_number("-2.5em"), Some(-2.5));
        assert_eq!(leading_number("px"), None);
    }
}
