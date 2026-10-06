//! Mermaid state diagrams (`stateDiagram` / `stateDiagram-v2`) — AST and
//! parser (§3.7.6): states with `as` labels and `S : description` lines,
//! `[*]` start/end pseudo-states scoped per composite, composite states
//! `state X { … }` (nested, with their own `direction`), concurrent
//! regions (`--`), `<<fork>>` / `<<join>>` / `<<choice>>`, transitions
//! with labels, notes (single-line and `note … end note`), `classDef` /
//! `class` / `:::`. Layout and drawing live in `build`.
//! Callers: `mermaid::render`/`mermaid::validate`.

mod build;

pub use build::build;

use super::diag::Diagnostic;
use super::layout::Dir;
use super::source::Source;
use super::text::clean_label;
use super::theme::StyleSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StateKind {
    #[default]
    Normal,
    Start,
    End,
    Fork,
    Join,
    Choice,
    /// A concurrent region inside a composite (`--`).
    Region,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct State {
    pub id: String,
    pub label: String,
    pub descriptions: Vec<String>,
    pub kind: StateKind,
    /// Enclosing composite state or region.
    pub parent: Option<usize>,
    /// Has children (drawn as a box around them).
    pub composite: bool,
    pub dir: Option<Dir>,
    pub classes: Vec<String>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    pub from: usize,
    pub to: usize,
    pub label: Option<String>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub target: usize,
    pub left: bool,
    pub text: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StateDiagram {
    pub dir: Dir,
    pub states: Vec<State>,
    pub transitions: Vec<Transition>,
    pub notes: Vec<Note>,
    pub class_defs: Vec<(String, StyleSpec)>,
}

impl StateDiagram {
    pub fn index(&self, id: &str) -> Option<usize> {
        self.states.iter().position(|s| s.id == id)
    }

    pub fn class_def(&self, name: &str) -> Option<&StyleSpec> {
        self.class_defs.iter().rev().find(|(n, _)| n == name).map(|(_, s)| s)
    }
}

struct Parser {
    d: StateDiagram,
    diags: Vec<Diagnostic>,
    /// Open composites / regions (innermost last).
    stack: Vec<usize>,
    regions: usize,
}

impl Parser {
    fn scope(&self) -> Option<usize> {
        self.stack.last().copied()
    }

    fn state(&mut self, id: &str, line: usize) -> usize {
        if let Some(i) = self.d.index(id) {
            return i;
        }
        self.d.states.push(State { id: id.to_string(), label: id.to_string(), parent: self.scope(), line, ..State::default() });
        self.d.states.len() - 1
    }

    /// `[*]` in the current scope, as a start (source) or end (target).
    fn pseudo(&mut self, start: bool, line: usize) -> usize {
        let scope = self.scope().map_or("root".to_string(), |s| s.to_string());
        let id = format!("[*]{}@{scope}", if start { "start" } else { "end" });
        let i = self.state(&id, line);
        self.d.states[i].kind = if start { StateKind::Start } else { StateKind::End };
        self.d.states[i].label = String::new();
        i
    }

    /// A state reference with optional `:::class`.
    fn endpoint(&mut self, text: &str, start: bool, line: usize) -> usize {
        let (name, class) = match text.split_once(":::") {
            Some((n, c)) => (n.trim(), Some(c.trim())),
            None => (text.trim(), None),
        };
        let i = if name == "[*]" { self.pseudo(start, line) } else { self.state(name, line) };
        if let Some(c) = class.filter(|c| !c.is_empty()) {
            self.d.states[i].classes.push(c.to_string());
        }
        i
    }
}

pub fn parse(src: &Source<'_>) -> (StateDiagram, Vec<Diagnostic>) {
    let mut p = Parser { d: StateDiagram::default(), diags: Vec::new(), stack: Vec::new(), regions: 0 };
    let body = src.body();
    let mut i = 0;
    while i < body.len() {
        let line = &body[i];
        let (no, col) = (line.no, line.indent + 1);
        let t = line.text;
        i += 1;
        if t == "}" {
            // Close any open region first, then the composite.
            if let Some(&top) = p.stack.last()
                && p.d.states[top].kind == StateKind::Region
            {
                p.stack.pop();
            }
            if p.stack.pop().is_none() {
                p.diags.push(Diagnostic::error(no, col, "`}` without an open composite state"));
            }
            continue;
        }
        if t == "--" {
            split_region(&mut p, no, col);
            continue;
        }
        let (kw, rest) = t.split_once(char::is_whitespace).map_or((t, ""), |(k, r)| (k, r.trim()));
        match kw {
            "direction" => match Dir::parse(rest) {
                Some(d) => match p.scope() {
                    Some(s) => p.d.states[s].dir = Some(d),
                    None => p.d.dir = d,
                },
                None => p.diags.push(Diagnostic::error(no, col, format!("unknown direction `{rest}`"))),
            },
            "hide" | "scale" => {}
            "classDef" => {
                let (names, style) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                let spec = StyleSpec::parse(style);
                for n in names.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                    p.d.class_defs.push((n.to_string(), spec.clone()));
                }
            }
            "class" => {
                let (ids, class) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                for id in ids.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                    let s = p.state(id, no);
                    p.d.states[s].classes.push(class.trim().to_string());
                }
            }
            "state" => state_statement(&mut p, rest, no, col),
            "note" => {
                let lower = rest.to_ascii_lowercase();
                let (left, after) = if let Some(a) = lower.strip_prefix("left of") {
                    (true, &rest[rest.len() - a.len()..])
                } else if let Some(a) = lower.strip_prefix("right of") {
                    (false, &rest[rest.len() - a.len()..])
                } else {
                    p.diags.push(Diagnostic::error(no, col, "note must be `note left of S` or `note right of S`"));
                    continue;
                };
                let (target, text) = match after.split_once(':') {
                    Some((t, x)) => (t.trim(), clean_label(x.trim())),
                    None => {
                        // Multi-line until `end note`.
                        let mut lines = Vec::new();
                        while i < body.len() && !body[i].text.eq_ignore_ascii_case("end note") {
                            lines.push(body[i].text);
                            i += 1;
                        }
                        if i >= body.len() {
                            p.diags.push(Diagnostic::error(no, col, "note is never closed with `end note`"));
                        }
                        i += 1;
                        (after.trim(), clean_label(&lines.join("\n")))
                    }
                };
                let target = p.state(target, no);
                p.d.notes.push(Note { target, left, text, line: no });
            }
            _ => {
                if let Some((a, b)) = t.split_once("-->") {
                    let (to_text, label) = match b.split_once(':') {
                        Some((to, l)) => (to, Some(clean_label(l.trim()))),
                        None => (b, None),
                    };
                    if a.trim().is_empty() || to_text.trim().is_empty() {
                        p.diags.push(Diagnostic::error(no, col, "transition needs a source and a target"));
                        continue;
                    }
                    let from = p.endpoint(a, true, no);
                    let to = p.endpoint(to_text, false, no);
                    p.d.transitions.push(Transition { from, to, label, line: no });
                } else if let Some((id, desc)) = t.split_once(':') {
                    let s = p.endpoint(id, false, no);
                    p.d.states[s].descriptions.push(clean_label(desc.trim()));
                } else if !t.contains(char::is_whitespace) {
                    p.endpoint(t, false, no);
                } else {
                    p.diags.push(Diagnostic::error(no, col, format!("unrecognised statement `{t}`")));
                }
            }
        }
    }
    for &open in &p.stack {
        if p.d.states[open].kind != StateKind::Region {
            let s = &p.d.states[open];
            p.diags.push(Diagnostic::error(s.line, 1, format!("composite state `{}` is never closed with `}}`", s.id)));
        }
    }
    let parents: Vec<usize> = p.d.states.iter().filter_map(|s| s.parent).collect();
    for par in parents {
        p.d.states[par].composite = true;
    }
    (p.d, p.diags)
}

fn state_statement(p: &mut Parser, rest: &str, no: usize, col: usize) {
    let opens = rest.ends_with('{');
    let rest = rest.trim_end_matches('{').trim();
    // `state "Long label" as S` / `state S <<fork>>` / `state S`.
    let (id, label) = if let Some(q) = rest.strip_prefix('"') {
        match q.split_once('"') {
            Some((label, after)) => {
                let id = after.trim().strip_prefix("as").map(str::trim).unwrap_or(label);
                (id.to_string(), Some(clean_label(label)))
            }
            None => {
                p.diags.push(Diagnostic::error(no, col, "unclosed `\"` in state label"));
                return;
            }
        }
    } else if let Some((label, id)) = rest.split_once(" as ") {
        (id.trim().to_string(), Some(clean_label(label.trim())))
    } else {
        (rest.to_string(), None)
    };
    let (id, kind) = if let Some(i) = id.find("<<") {
        let tag = id[i + 2..].trim_end_matches(">>").trim().to_ascii_lowercase();
        let kind = match tag.as_str() {
            "fork" => StateKind::Fork,
            "join" => StateKind::Join,
            "choice" => StateKind::Choice,
            _ => StateKind::Normal,
        };
        (id[..i].trim().to_string(), kind)
    } else {
        (id, StateKind::Normal)
    };
    if id.is_empty() {
        p.diags.push(Diagnostic::error(no, col, "`state` needs a name"));
        return;
    }
    let (id, desc) = match id.split_once(':') {
        Some((i, d)) => (i.trim().to_string(), Some(clean_label(d.trim()))),
        None => (id, None),
    };
    let s = p.state(&id, no);
    p.d.states[s].line = no;
    if kind != StateKind::Normal {
        p.d.states[s].kind = kind;
        p.d.states[s].label = String::new();
    }
    if let Some(l) = label {
        p.d.states[s].label = l;
    }
    if let Some(d) = desc {
        p.d.states[s].descriptions.push(d);
    }
    if opens {
        p.stack.push(s);
    }
}

/// `--` inside a composite: everything so far becomes region 1, what
/// follows region 2 (and so on).
fn split_region(p: &mut Parser, no: usize, col: usize) {
    let Some(&top) = p.stack.last() else {
        p.diags.push(Diagnostic::error(no, col, "`--` is only allowed inside a composite state"));
        return;
    };
    let composite = if p.d.states[top].kind == StateKind::Region {
        p.stack.pop();
        p.d.states[top].parent.unwrap_or(top)
    } else {
        top
    };
    let has_regions = p.d.states.iter().any(|s| s.kind == StateKind::Region && s.parent == Some(composite));
    let new_region = |p: &mut Parser| -> usize {
        p.regions += 1;
        p.d.states.push(State {
            id: format!("[region]{}", p.regions),
            kind: StateKind::Region,
            parent: Some(composite),
            line: no,
            ..State::default()
        });
        p.d.states.len() - 1
    };
    if !has_regions {
        // Move the existing children into the first region.
        let first = new_region(p);
        for s in 0..p.d.states.len() - 1 {
            if p.d.states[s].parent == Some(composite) {
                p.d.states[s].parent = Some(first);
            }
        }
    }
    let next = new_region(p);
    p.stack.push(next);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mermaid::source::preprocess;

    fn p(src: &str) -> (StateDiagram, Vec<Diagnostic>) {
        parse(&preprocess(src))
    }

    #[test]
    fn parses_states_transitions_and_pseudo_states() {
        let (d, diags) = p("stateDiagram-v2\n  [*] --> Still\n  Still --> [*]\n  Still --> Moving : push\n  Moving --> Crash\n  Crash --> [*]\n  state \"Very long\" as L\n  L : first line\n  L : second line\n");
        assert!(diags.is_empty(), "{diags:?}");
        let still = d.index("Still").unwrap();
        assert_eq!(d.transitions.len(), 5);
        assert_eq!(d.transitions[2].label.as_deref(), Some("push"));
        assert_eq!(d.states.iter().filter(|s| s.kind == StateKind::Start).count(), 1);
        assert_eq!(d.states.iter().filter(|s| s.kind == StateKind::End).count(), 1);
        assert_eq!(d.states[still].label, "Still");
        let l = &d.states[d.index("L").unwrap()];
        assert_eq!(l.label, "Very long");
        assert_eq!(l.descriptions, vec!["first line", "second line"]);
    }

    #[test]
    fn parses_composites_regions_and_special_states() {
        let src = "stateDiagram-v2\n  state fork <<fork>>\n  state join <<join>>\n  state if <<choice>>\n  [*] --> Active\n  state Active {\n    direction LR\n    [*] --> NumLockOff\n    NumLockOff --> NumLockOn\n    --\n    [*] --> CapsOff\n  }\n  note right of Active : a note\n  note left of fork\n    multi\n    line\n  end note\n";
        let (d, diags) = p(src);
        assert!(diags.is_empty(), "{diags:?}");
        let active = d.index("Active").unwrap();
        assert!(d.states[active].composite);
        assert_eq!(d.states[active].dir, Some(Dir::LR));
        let regions: Vec<&State> = d.states.iter().filter(|s| s.kind == StateKind::Region).collect();
        assert_eq!(regions.len(), 2);
        let off = &d.states[d.index("NumLockOff").unwrap()];
        let caps = &d.states[d.index("CapsOff").unwrap()];
        assert_ne!(off.parent, caps.parent, "regions separate the children");
        // `[*]` inside each region is a different start state than at the root.
        assert_eq!(d.states.iter().filter(|s| s.kind == StateKind::Start).count(), 3);
        assert_eq!(d.states[d.index("fork").unwrap()].kind, StateKind::Fork);
        assert_eq!(d.states[d.index("if").unwrap()].kind, StateKind::Choice);
        assert_eq!(d.notes.len(), 2);
        assert_eq!(d.notes[1].text, "multi\nline");
    }

    #[test]
    fn reports_unclosed_composites() {
        let (_, diags) = p("stateDiagram-v2\n  state A {\n    B --> C\n");
        assert!(diags.iter().any(|x| x.is_error() && x.message.contains("never closed")), "{diags:?}");
    }
}
