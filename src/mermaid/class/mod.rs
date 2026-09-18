//! Mermaid class diagrams (`classDiagram`) — AST and parser (§3.7.6):
//! `class X { … }` bodies and `X : member` lines (methods detected by
//! `(`), generics `~T~`, `<<annotation>>`s, labels `class X["Label"]`,
//! every relation (`<|--`, `*--`, `o--`, `-->`, `--`, `..>`, `..|>`,
//! `..`, `()--`, two-way forms) with cardinalities and labels,
//! `namespace`, notes, `direction`, `classDef` / `cssClass` / `:::`,
//! `click`/`link`. Layout and drawing live in `build`.
//! Callers: `mermaid::render`/`mermaid::validate`.

mod build;

pub use build::build;

use super::diag::Diagnostic;
use super::layout::Dir;
use super::scene::Marker;
use super::source::Source;
use super::text::clean_label;
use super::theme::StyleSpec;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Class {
    pub id: String,
    pub label: String,
    pub annotations: Vec<String>,
    pub attributes: Vec<String>,
    pub methods: Vec<String>,
    pub namespace: Option<usize>,
    pub classes: Vec<String>,
    pub style: StyleSpec,
    pub link: Option<String>,
    pub tooltip: Option<String>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Relation {
    pub from: usize,
    pub to: usize,
    pub start: Option<Marker>,
    pub end: Option<Marker>,
    pub dashed: bool,
    pub label: Option<String>,
    pub card_from: Option<String>,
    pub card_to: Option<String>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassNote {
    pub target: Option<usize>,
    pub text: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClassDiagram {
    pub dir: Dir,
    pub classes: Vec<Class>,
    pub relations: Vec<Relation>,
    pub notes: Vec<ClassNote>,
    /// (name, source line).
    pub namespaces: Vec<(String, usize)>,
    pub class_defs: Vec<(String, StyleSpec)>,
}

impl ClassDiagram {
    pub fn index(&self, id: &str) -> Option<usize> {
        self.classes.iter().position(|c| c.id == id)
    }

    pub fn class_def(&self, name: &str) -> Option<&StyleSpec> {
        self.class_defs.iter().rev().find(|(n, _)| n == name).map(|(_, s)| s)
    }

    /// Find or create a class from `Name`, `Name~T~` or `Name:::css`.
    fn ensure(&mut self, raw: &str, line: usize, ns: Option<usize>) -> usize {
        let (name, css) = match raw.split_once(":::") {
            Some((n, c)) => (n.trim(), Some(c.trim())),
            None => (raw.trim(), None),
        };
        let (id, generic) = match name.split_once('~') {
            Some((id, g)) => (id.trim(), Some(g.trim_end_matches('~'))),
            None => (name, None),
        };
        let idx = match self.index(id) {
            Some(i) => i,
            None => {
                self.classes.push(Class { id: id.to_string(), label: id.to_string(), namespace: ns, line, ..Class::default() });
                self.classes.len() - 1
            }
        };
        if let Some(g) = generic {
            self.classes[idx].label = format!("{id}<{}>", generics(g));
        }
        if let Some(c) = css.filter(|c| !c.is_empty()) {
            self.classes[idx].classes.push(c.to_string());
        }
        idx
    }
}

/// `List~int~` → `List<int>` (nested `~` pairs too).
pub fn generics(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut open = true;
    for c in s.chars() {
        if c == '~' {
            out.push(if open { '<' } else { '>' });
            open = !open;
        } else {
            out.push(c);
        }
    }
    out
}

fn add_member(c: &mut Class, member: &str) {
    let m = member.trim();
    if m.is_empty() {
        return;
    }
    if let Some(a) = m.strip_prefix("<<").and_then(|a| a.strip_suffix(">>")) {
        c.annotations.push(a.trim().to_string());
    } else if m.contains('(') {
        c.methods.push(generics(m));
    } else {
        c.attributes.push(generics(m));
    }
}

fn marker(tok: &str) -> Option<Marker> {
    match tok {
        "<|" | "|>" => Some(Marker::Triangle),
        "*" => Some(Marker::DiamondFilled),
        "o" => Some(Marker::DiamondHollow),
        "<" | ">" => Some(Marker::OpenArrow),
        "()" | "(" | ")" => Some(Marker::Circle),
        _ => None,
    }
}

type RelationOp = (usize, usize, Option<Marker>, Option<Marker>, bool);

/// Finds a relation operator: (start byte, end byte, start marker, end marker, dashed).
fn find_relation(s: &str) -> Option<RelationOp> {
    let bytes = s.as_bytes();
    let mut quoted = false;
    for i in 0..bytes.len().saturating_sub(1) {
        if bytes[i] == b'"' {
            quoted = !quoted;
        }
        if quoted || !s.is_char_boundary(i) || !s.is_char_boundary(i + 2) {
            continue;
        }
        let pair = &s[i..i + 2];
        if pair != "--" && pair != ".." {
            continue;
        }
        let (before, after) = (&s[..i], &s[i + 2..]);
        // `o` is a marker only when it stands alone (`A o-- B`, `A --o B`).
        let left = ["<|", "()", "*", "o", "<", "("]
            .iter()
            .find(|t| before.ends_with(**t) && (**t != "o" || before.len() == 1 || before[..before.len() - 1].ends_with(' ')))
            .copied();
        let right = ["|>", "()", "*", "o", ">", ")"]
            .iter()
            .find(|t| after.starts_with(**t) && (**t != "o" || after.len() == 1 || after[1..].starts_with(' ')))
            .copied();
        let start = i - left.map_or(0, str::len);
        let end = i + 2 + right.map_or(0, str::len);
        return Some((start, end, left.and_then(marker), right.and_then(marker), pair == ".."));
    }
    None
}

/// Splits a trailing/leading `"cardinality"` off an endpoint.
fn split_card(text: &str, trailing: bool) -> (String, Option<String>) {
    let t = text.trim();
    if trailing {
        if let Some(stripped) = t.strip_suffix('"')
            && let Some(q) = stripped.rfind('"')
        {
            return (stripped[..q].trim().to_string(), Some(stripped[q + 1..].to_string()));
        }
    } else if let Some(rest) = t.strip_prefix('"')
        && let Some(q) = rest.find('"')
    {
        return (rest[q + 1..].trim().to_string(), Some(rest[..q].to_string()));
    }
    (t.to_string(), None)
}

pub fn parse(src: &Source<'_>) -> (ClassDiagram, Vec<Diagnostic>) {
    let mut d = ClassDiagram::default();
    let mut diags = Vec::new();
    let mut body_of: Option<usize> = None;
    let mut ns_stack: Vec<usize> = Vec::new();
    for line in src.body() {
        let (no, col) = (line.no, line.indent + 1);
        let t = line.text;
        if let Some(c) = body_of {
            if t == "}" {
                body_of = None;
            } else {
                add_member(&mut d.classes[c], t);
            }
            continue;
        }
        if t == "}" {
            if ns_stack.pop().is_none() {
                diags.push(Diagnostic::error(no, col, "`}` without an open class or namespace"));
            }
            continue;
        }
        let ns = ns_stack.last().copied();
        let (kw, rest) = t.split_once(char::is_whitespace).map_or((t, ""), |(k, r)| (k, r.trim()));
        match kw {
            "direction" => match Dir::parse(rest) {
                Some(dir) => d.dir = dir,
                None => diags.push(Diagnostic::error(no, col, format!("unknown direction `{rest}`"))),
            },
            "namespace" => {
                d.namespaces.push((rest.trim_end_matches('{').trim().to_string(), no));
                ns_stack.push(d.namespaces.len() - 1);
            }
            "class" => {
                let opens = rest.ends_with('{');
                let mut spec = rest.trim_end_matches('{').trim();
                let mut label = None;
                if let Some(open) = spec.find("[\"")
                    && let Some(close) = spec.rfind("\"]")
                {
                    label = Some(clean_label(&spec[open + 2..close]));
                    spec = spec[..open].trim();
                }
                let c = d.ensure(spec, no, ns);
                d.classes[c].line = no;
                if let Some(l) = label {
                    d.classes[c].label = l;
                }
                if opens {
                    body_of = Some(c);
                }
            }
            "classDef" => {
                let (names, style) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                let spec = StyleSpec::parse(style);
                for n in names.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                    d.class_defs.push((n.to_string(), spec.clone()));
                }
            }
            "cssClass" => {
                let (ids, class) = rest.rsplit_once(char::is_whitespace).unwrap_or((rest, ""));
                for id in ids.trim_matches('"').split(',').map(str::trim).filter(|n| !n.is_empty()) {
                    let c = d.ensure(id, no, ns);
                    d.classes[c].classes.push(class.trim().to_string());
                }
            }
            "style" => {
                let (id, css) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                let c = d.ensure(id, no, ns);
                d.classes[c].style.apply(&StyleSpec::parse(css));
            }
            "note" => {
                let (target, text) = match rest.strip_prefix("for ") {
                    Some(r) => {
                        let (id, text) = r.trim().split_once(char::is_whitespace).unwrap_or((r, ""));
                        (Some(d.ensure(id, no, ns)), text)
                    }
                    None => (None, rest),
                };
                d.notes.push(ClassNote { target, text: clean_label(text.trim()), line: no });
            }
            "click" | "link" | "callback" => {
                let Some(id) = rest.split_whitespace().next() else { continue };
                let c = d.ensure(id, no, ns);
                let quoted: Vec<&str> = rest.split('"').skip(1).step_by(2).collect();
                let is_callback = kw == "callback" || rest.contains(" call ") || rest.contains(" callback ");
                if is_callback {
                    d.classes[c].tooltip = quoted.first().map(|s| s.to_string());
                } else {
                    d.classes[c].link = quoted.first().map(|s| s.to_string());
                    d.classes[c].tooltip = quoted.get(1).map(|s| s.to_string());
                }
            }
            _ if t.starts_with("<<") => {
                // `<<interface>> Animal`
                if let Some((a, id)) = t[2..].split_once(">>") {
                    let c = d.ensure(id.trim(), no, ns);
                    d.classes[c].annotations.push(a.trim().to_string());
                }
            }
            _ => {
                if let Some((s, e, start, end, dashed)) = find_relation(t) {
                    let (rhs, label) = match t[e..].split_once(':') {
                        Some((r, l)) => (r, Some(clean_label(l.trim()))),
                        None => (&t[e..], None),
                    };
                    let (from_txt, card_from) = split_card(&t[..s], true);
                    let (to_txt, card_to) = split_card(rhs, false);
                    if from_txt.is_empty() || to_txt.is_empty() {
                        diags.push(Diagnostic::error(no, col, "relation needs a class on both sides"));
                        continue;
                    }
                    let from = d.ensure(&from_txt, no, ns);
                    let to = d.ensure(&to_txt, no, ns);
                    d.relations.push(Relation { from, to, start, end, dashed, label, card_from, card_to, line: no });
                } else if let Some((id, member)) = t.split_once(':') {
                    let c = d.ensure(id, no, ns);
                    add_member(&mut d.classes[c], member);
                } else if !t.contains(char::is_whitespace) {
                    d.ensure(t, no, ns);
                } else {
                    diags.push(Diagnostic::error(no, col, format!("unrecognised statement `{t}`")));
                }
            }
        }
    }
    if let Some(c) = body_of {
        let cl = &d.classes[c];
        diags.push(Diagnostic::error(cl.line, 1, format!("class `{}` body is never closed with `}}`", cl.id)));
    }
    for &n in &ns_stack {
        let (name, line) = &d.namespaces[n];
        diags.push(Diagnostic::error(*line, 1, format!("namespace `{name}` is never closed")));
    }
    (d, diags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mermaid::source::preprocess;

    fn p(src: &str) -> (ClassDiagram, Vec<Diagnostic>) {
        parse(&preprocess(src))
    }

    #[test]
    fn parses_classes_members_and_annotations() {
        let (d, diags) = p("classDiagram\n  class Animal {\n    <<abstract>>\n    +String name\n    +int age\n    +makeSound() void\n  }\n  class Box~T~\n  Duck : +swim()\n  Duck : +String beak\n  <<interface>> Flyer\n  class Id[\"Display name\"]\n");
        assert!(diags.is_empty(), "{diags:?}");
        let a = &d.classes[d.index("Animal").unwrap()];
        assert_eq!(a.annotations, vec!["abstract"]);
        assert_eq!(a.attributes, vec!["+String name", "+int age"]);
        assert_eq!(a.methods, vec!["+makeSound() void"]);
        assert_eq!(d.classes[d.index("Box").unwrap()].label, "Box<T>");
        let duck = &d.classes[d.index("Duck").unwrap()];
        assert_eq!((duck.methods.len(), duck.attributes.len()), (1, 1));
        assert_eq!(d.classes[d.index("Flyer").unwrap()].annotations, vec!["interface"]);
        assert_eq!(d.classes[d.index("Id").unwrap()].label, "Display name");
    }

    #[test]
    fn parses_every_relation_kind_with_cardinality() {
        let (d, diags) = p("classDiagram\n  Animal <|-- Duck\n  A *-- B\n  C o-- D\n  E --> F\n  G -- H\n  I ..> J\n  K ..|> L\n  M .. N\n  Customer \"1\" --> \"*\" Ticket : buys\n  X <|--|> Y\n");
        assert!(diags.is_empty(), "{diags:?}");
        let r = &d.relations;
        assert_eq!(r.len(), 10);
        assert_eq!((r[0].start, r[0].end), (Some(Marker::Triangle), None));
        assert_eq!(r[1].start, Some(Marker::DiamondFilled));
        assert_eq!(r[2].start, Some(Marker::DiamondHollow));
        assert_eq!(r[3].end, Some(Marker::OpenArrow));
        assert_eq!((r[4].start, r[4].end, r[4].dashed), (None, None, false));
        assert!(r[5].dashed && r[5].end == Some(Marker::OpenArrow));
        assert!(r[6].dashed && r[6].end == Some(Marker::Triangle));
        assert!(r[7].dashed);
        let buys = &r[8];
        assert_eq!((buys.card_from.as_deref(), buys.card_to.as_deref(), buys.label.as_deref()), (Some("1"), Some("*"), Some("buys")));
        assert_eq!((r[9].start, r[9].end), (Some(Marker::Triangle), Some(Marker::Triangle)));
        assert!(d.index("Customer").is_some() && d.index("Ticket").is_some());
    }

    #[test]
    fn parses_namespaces_notes_and_errors() {
        let (d, diags) = p("classDiagram\n  namespace Shapes {\n    class Square\n  }\n  note for Square \"four sides\"\n  note \"general\"\n  class Open {\n");
        assert_eq!(d.namespaces.len(), 1);
        assert_eq!(d.classes[d.index("Square").unwrap()].namespace, Some(0));
        assert_eq!(d.notes.len(), 2);
        assert_eq!(d.notes[0].text, "four sides");
        assert!(diags.iter().any(|x| x.message.contains("never closed")), "{diags:?}");
    }
}
