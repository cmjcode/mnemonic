//! Mermaid sequence diagrams (`sequenceDiagram`) — AST and parser
//! (§3.7.6). Covers participants/actors (with `as` aliases and v11 types),
//! `box` groups, `create`/`destroy`, every message arrow (`->`, `-->`,
//! `->>`, `-->>`, `<<->>`, `<<-->>`, `-x`, `--x`, `-)`, `--)`) with the
//! `+`/`-` activation shorthand, `activate`/`deactivate`, notes, the
//! `loop`/`alt`/`else`/`opt`/`par`/`and`/`critical`/`option`/`break`/
//! `rect` blocks, `autonumber` and `title`. Layout lives in `build`.
//! Callers: `mermaid::render`/`mermaid::validate`.

mod build;

pub use build::build;

use super::diag::Diagnostic;
use super::scene::Marker;
use super::source::Source;
use super::text::clean_label;
use super::theme::{Color, parse_color};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActorKind {
    #[default]
    Participant,
    Actor,
    Boundary,
    Control,
    Entity,
    Database,
    Collections,
    Queue,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Participant {
    pub id: String,
    pub label: String,
    pub kind: ActorKind,
    pub line: usize,
    /// `create participant`: first appears at this event.
    pub created_at: Option<usize>,
    /// `destroy`: lifeline ends at this event.
    pub destroyed_at: Option<usize>,
    pub group: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub title: String,
    pub color: Option<Color>,
    pub members: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotePlace {
    Left,
    Right,
    Over,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Loop,
    Alt,
    Opt,
    Par,
    Critical,
    Break,
    Rect,
}

impl BlockKind {
    pub fn keyword(self) -> &'static str {
        match self {
            BlockKind::Loop => "loop",
            BlockKind::Alt => "alt",
            BlockKind::Opt => "opt",
            BlockKind::Par => "par",
            BlockKind::Critical => "critical",
            BlockKind::Break => "break",
            BlockKind::Rect => "rect",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Message {
        from: usize,
        to: usize,
        text: String,
        dotted: bool,
        head: Option<Marker>,
        /// `<<->>`: arrow heads on both ends.
        both: bool,
        /// `+` on the target / `-` on the source.
        activate_target: bool,
        deactivate_source: bool,
        line: usize,
    },
    Note { place: NotePlace, a: usize, b: Option<usize>, text: String, line: usize },
    Activate { who: usize, on: bool, line: usize },
    BlockStart { kind: BlockKind, label: String, color: Option<Color>, line: usize },
    /// `else` / `and` / `option`.
    BlockSection { label: String, line: usize },
    BlockEnd { line: usize },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sequence {
    pub title: Option<String>,
    pub participants: Vec<Participant>,
    pub groups: Vec<Group>,
    pub events: Vec<Event>,
    /// `autonumber [start [step]]`.
    pub autonumber: Option<(i64, i64)>,
}

impl Sequence {
    pub fn index(&self, id: &str) -> Option<usize> {
        self.participants.iter().position(|p| p.id == id)
    }

    /// Find or implicitly create a participant.
    fn ensure(&mut self, id: &str, line: usize, group: Option<usize>) -> usize {
        if let Some(i) = self.index(id) {
            return i;
        }
        self.participants.push(Participant {
            id: id.to_string(),
            label: id.to_string(),
            line,
            group,
            ..Participant::default()
        });
        let idx = self.participants.len() - 1;
        if let Some(g) = group {
            self.groups[g].members.push(idx);
        }
        idx
    }
}

type ArrowSpec = (&'static str, bool, Option<Marker>, bool);

/// Arrow tokens: (text, dotted, head, both ends).
const ARROWS: &[ArrowSpec] = &[
    ("<<-->>", true, Some(Marker::Arrow), true),
    ("<<->>", false, Some(Marker::Arrow), true),
    ("-->>", true, Some(Marker::Arrow), false),
    ("->>", false, Some(Marker::Arrow), false),
    ("--x", true, Some(Marker::Cross), false),
    ("-x", false, Some(Marker::Cross), false),
    ("--)", true, Some(Marker::OpenArrow), false),
    ("-)", false, Some(Marker::OpenArrow), false),
    ("-->", true, None, false),
    ("->", false, None, false),
];

/// A `box` pseudo-block on the block stack (not an event).
const BOX_MARK: usize = usize::MAX;

pub fn parse(src: &Source<'_>) -> (Sequence, Vec<Diagnostic>) {
    let mut seq = Sequence { title: src.title.clone(), ..Sequence::default() };
    let mut diags = Vec::new();
    let mut depth: Vec<(BlockKind, usize)> = Vec::new();
    let mut open_box: Option<usize> = None;
    let mut pending_create = false;
    let mut pending_destroy: Vec<usize> = Vec::new();

    for line in src.body() {
        for stmt in line.text.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            let no = line.no;
            let col = line.indent + 1;
            let (kw, rest) = stmt.split_once(char::is_whitespace).map_or((stmt, ""), |(k, r)| (k, r.trim()));
            let lower = kw.to_ascii_lowercase();
            match lower.as_str() {
                "title" => seq.title = Some(clean_label(rest.trim_start_matches(':').trim())),
                "autonumber" => {
                    let nums: Vec<i64> = rest.split_whitespace().filter_map(|n| n.parse().ok()).collect();
                    seq.autonumber = if rest == "off" {
                        None
                    } else {
                        Some((nums.first().copied().unwrap_or(1), nums.get(1).copied().unwrap_or(1)))
                    };
                }
                "create" => {
                    pending_create = true;
                    let (k2, r2) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                    declare(&mut seq, k2, r2, (no, col), open_box, true, &mut diags);
                }
                "destroy" => match seq.index(rest) {
                    Some(i) => pending_destroy.push(i),
                    None => diags.push(Diagnostic::error(no, col, format!("`destroy` of unknown participant `{rest}`"))),
                },
                "participant" | "actor" => declare(&mut seq, kw, rest, (no, col), open_box, false, &mut diags),
                "box" => {
                    let (color, title) = split_color(rest);
                    seq.groups.push(Group { title, color, members: Vec::new() });
                    open_box = Some(seq.groups.len() - 1);
                    depth.push((BlockKind::Rect, BOX_MARK));
                }
                "activate" | "deactivate" => {
                    let who = seq.ensure(rest, no, open_box);
                    seq.events.push(Event::Activate { who, on: lower == "activate", line: no });
                }
                "note" => match parse_note(rest) {
                    Some((place, a, b, text)) => {
                        let a = seq.ensure(&a, no, open_box);
                        let b = b.map(|b| seq.ensure(&b, no, open_box));
                        seq.events.push(Event::Note { place, a, b, text, line: no });
                    }
                    None => diags.push(Diagnostic::error(
                        no,
                        col,
                        "note must look like `Note right of A: text` or `Note over A,B: text`",
                    )),
                },
                "loop" | "alt" | "opt" | "par" | "par_over" | "critical" | "break" | "rect" => {
                    let kind = match lower.as_str() {
                        "loop" => BlockKind::Loop,
                        "alt" => BlockKind::Alt,
                        "opt" => BlockKind::Opt,
                        "par" | "par_over" => BlockKind::Par,
                        "critical" => BlockKind::Critical,
                        "break" => BlockKind::Break,
                        _ => BlockKind::Rect,
                    };
                    let (color, label) =
                        if kind == BlockKind::Rect { split_color(rest) } else { (None, clean_label(rest)) };
                    depth.push((kind, seq.events.len()));
                    seq.events.push(Event::BlockStart { kind, label, color, line: no });
                }
                "else" | "and" | "option" => {
                    if depth.last().is_none_or(|(_, e)| *e == BOX_MARK) {
                        diags.push(Diagnostic::error(no, col, format!("`{kw}` outside of a block")));
                    } else {
                        seq.events.push(Event::BlockSection { label: clean_label(rest), line: no });
                    }
                }
                "end" => match depth.pop() {
                    Some((_, BOX_MARK)) => open_box = None,
                    Some(_) => seq.events.push(Event::BlockEnd { line: no }),
                    None => diags.push(Diagnostic::error(no, col, "`end` without an open block")),
                },
                "link" | "links" | "properties" | "details" => {}
                _ => match parse_message(stmt) {
                    Some(m) => {
                        let from = seq.ensure(&m.from, no, open_box);
                        let to = seq.ensure(&m.to, no, open_box);
                        let idx = seq.events.len();
                        if pending_create {
                            seq.participants[to].created_at = Some(idx);
                            pending_create = false;
                        }
                        for d in pending_destroy.drain(..) {
                            seq.participants[d].destroyed_at = Some(idx);
                        }
                        seq.events.push(Event::Message {
                            from,
                            to,
                            text: m.text,
                            dotted: m.dotted,
                            head: m.head,
                            both: m.both,
                            activate_target: m.plus,
                            deactivate_source: m.minus,
                            line: no,
                        });
                    }
                    None => diags.push(Diagnostic::error(
                        no,
                        col,
                        format!("unrecognised statement `{stmt}` (expected e.g. `A->>B: hello`)"),
                    )),
                },
            }
        }
    }
    let last = src.lines.last().map_or(1, |l| l.no);
    for (kind, mark) in depth {
        let what = if mark == BOX_MARK { "box" } else { kind.keyword() };
        diags.push(Diagnostic::error(last, 1, format!("`{what}` block is never closed with `end`")));
    }
    (seq, diags)
}

fn declare(
    seq: &mut Sequence,
    kw: &str,
    rest: &str,
    (no, col): (usize, usize),
    open_box: Option<usize>,
    create: bool,
    diags: &mut Vec<Diagnostic>,
) {
    let mut kind = if kw.eq_ignore_ascii_case("actor") { ActorKind::Actor } else { ActorKind::Participant };
    let mut spec = rest.to_string();
    // v11: `participant A@{ "type": "database" }`.
    if let Some(at) = spec.find("@{") {
        let close = spec[at..].find('}').map_or(spec.len(), |c| at + c + 1);
        let meta = spec[at + 1..close].to_string();
        spec = format!("{}{}", &spec[..at], &spec[close..]);
        if let Ok(serde_json::Value::Object(m)) = serde_yaml::from_str::<serde_json::Value>(&meta)
            && let Some(t) = m.get("type").and_then(|v| v.as_str())
        {
            kind = match t {
                "boundary" => ActorKind::Boundary,
                "control" => ActorKind::Control,
                "entity" => ActorKind::Entity,
                "database" => ActorKind::Database,
                "collections" => ActorKind::Collections,
                "queue" => ActorKind::Queue,
                "actor" => ActorKind::Actor,
                _ => kind,
            };
        }
    }
    let (id, label) = match spec.split_once(" as ") {
        Some((id, label)) => (id.trim().to_string(), clean_label(label.trim())),
        None => (spec.trim().to_string(), clean_label(spec.trim())),
    };
    if id.is_empty() {
        diags.push(Diagnostic::error(no, col, format!("`{kw}` needs a name")));
        return;
    }
    let idx = seq.ensure(&id, no, open_box);
    let p = &mut seq.participants[idx];
    p.label = label;
    p.kind = kind;
    p.line = no;
    if create {
        p.created_at = Some(usize::MAX);
    }
}

/// `rgb(1,2,3) Title` / `Aqua Title` / `Title` → (colour, title).
fn split_color(rest: &str) -> (Option<Color>, String) {
    let rest = rest.trim();
    if (rest.starts_with("rgb") || rest.starts_with("hsl"))
        && let Some(end) = rest.find(')')
    {
        return (parse_color(&rest[..=end]), clean_label(rest[end + 1..].trim()));
    }
    let (first, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    match parse_color(first) {
        Some(c) => (Some(c), clean_label(tail.trim())),
        None => (None, clean_label(rest)),
    }
}

fn parse_note(rest: &str) -> Option<(NotePlace, String, Option<String>, String)> {
    let (head, text) = rest.split_once(':')?;
    let head = head.trim();
    let lower = head.to_ascii_lowercase();
    let (place, who) = if let Some(w) = lower.strip_prefix("right of") {
        (NotePlace::Right, &head[head.len() - w.len()..])
    } else if let Some(w) = lower.strip_prefix("left of") {
        (NotePlace::Left, &head[head.len() - w.len()..])
    } else if let Some(w) = lower.strip_prefix("over") {
        (NotePlace::Over, &head[head.len() - w.len()..])
    } else {
        return None;
    };
    let mut parts = who.split(',').map(str::trim).filter(|s| !s.is_empty());
    let a = parts.next()?.to_string();
    let b = parts.next().map(String::from);
    Some((place, a, b, clean_label(text.trim())))
}

struct Msg {
    from: String,
    to: String,
    text: String,
    dotted: bool,
    head: Option<Marker>,
    both: bool,
    plus: bool,
    minus: bool,
}

fn parse_message(stmt: &str) -> Option<Msg> {
    // Leftmost arrow; longest token wins at the same position.
    let mut best: Option<(usize, &ArrowSpec)> = None;
    for a in ARROWS {
        if let Some(pos) = stmt.find(a.0)
            && best.is_none_or(|(bp, b)| pos < bp || (pos == bp && a.0.len() > b.0.len()))
        {
            best = Some((pos, a));
        }
    }
    let (pos, &(tok, dotted, head, both)) = best?;
    let from = stmt[..pos].trim();
    let after = &stmt[pos + tok.len()..];
    let (target, text) = after.split_once(':').unwrap_or((after, ""));
    let mut target = target.trim();
    let (mut plus, mut minus) = (false, false);
    if let Some(t) = target.strip_prefix('+') {
        plus = true;
        target = t.trim();
    } else if let Some(t) = target.strip_prefix('-') {
        minus = true;
        target = t.trim();
    }
    if from.is_empty() || target.is_empty() {
        return None;
    }
    Some(Msg {
        from: from.to_string(),
        to: target.to_string(),
        text: clean_label(text.trim()),
        dotted,
        head,
        both,
        plus,
        minus,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mermaid::source::preprocess;

    fn p(src: &str) -> (Sequence, Vec<Diagnostic>) {
        parse(&preprocess(src))
    }

    fn msgs(s: &Sequence) -> Vec<&Event> {
        s.events.iter().filter(|e| matches!(e, Event::Message { .. })).collect()
    }

    #[test]
    fn parses_participants_messages_and_arrows() {
        let (s, d) = p("sequenceDiagram\n  participant A as Alice\n  actor B as Bob\n  A->>+B: Hello Bob, how are you?\n  B-->>-A: Great!\n  A-)B: async\n  A-xB: cross\n  A->B: plain\n  A<<->>B: both\n  A->>A: self\n");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(s.participants.len(), 2);
        assert_eq!(s.participants[0].label, "Alice");
        assert_eq!(s.participants[1].kind, ActorKind::Actor);
        let m = msgs(&s);
        assert_eq!(m.len(), 7);
        let Event::Message { head, activate_target, text, .. } = m[0] else { unreachable!() };
        assert_eq!((*head, *activate_target, text.as_str()), (Some(Marker::Arrow), true, "Hello Bob, how are you?"));
        let Event::Message { dotted, deactivate_source, .. } = m[1] else { unreachable!() };
        assert!(*dotted && *deactivate_source);
        let Event::Message { head, .. } = m[2] else { unreachable!() };
        assert_eq!(*head, Some(Marker::OpenArrow));
        let Event::Message { head, .. } = m[3] else { unreachable!() };
        assert_eq!(*head, Some(Marker::Cross));
        let Event::Message { head, .. } = m[4] else { unreachable!() };
        assert_eq!(*head, None);
        let Event::Message { both, .. } = m[5] else { unreachable!() };
        assert!(*both);
    }

    #[test]
    fn parses_notes_blocks_boxes_and_lifecycle() {
        let src = "sequenceDiagram\n  autonumber\n  box Aqua Group\n  participant A\n  participant B\n  end\n  Note right of A: hi\n  Note over A,B: both\n  loop Every minute\n    A->>B: ping\n  end\n  alt ok\n    B->>A: yes\n  else fail\n    B->>A: no\n  end\n  rect rgb(200, 150, 255)\n    A->>B: in rect\n  end\n  create participant C\n  A->>C: hi\n  destroy C\n  C->>A: bye\n";
        let (s, d) = p(src);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(s.autonumber, Some((1, 1)));
        assert_eq!(s.groups.len(), 1);
        assert_eq!(s.groups[0].members, vec![0, 1]);
        assert!(s.groups[0].color.is_some());
        assert!(s.events.iter().any(|e| matches!(e, Event::Note { place: NotePlace::Over, b: Some(_), .. })));
        assert_eq!(s.events.iter().filter(|e| matches!(e, Event::BlockStart { .. })).count(), 3);
        assert_eq!(s.events.iter().filter(|e| matches!(e, Event::BlockSection { .. })).count(), 1);
        let c = &s.participants[s.index("C").unwrap()];
        assert!(c.created_at.is_some() && c.destroyed_at.is_some());
    }

    #[test]
    fn typed_participants_keep_their_alias() {
        let (s, d) = p("sequenceDiagram\n  participant D@{ \"type\": \"database\" } as DB\n  D->>D: x\n");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!((s.participants[0].id.as_str(), s.participants[0].label.as_str()), ("D", "DB"));
        assert_eq!(s.participants[0].kind, ActorKind::Database);
    }

    #[test]
    fn reports_bad_lines_and_unclosed_blocks() {
        let (_, d) = p("sequenceDiagram\n  A->>B: ok\n  what is this\n  loop forever\n    A->>B: x\n");
        assert!(d.iter().any(|x| x.line == 3 && x.is_error()), "{d:?}");
        assert!(d.iter().any(|x| x.message.contains("never closed")), "{d:?}");
    }
}
