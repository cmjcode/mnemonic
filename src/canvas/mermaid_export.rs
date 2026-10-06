//! Canvas → Mermaid (§3.9.4). Mermaid can't mix diagram types, so the
//! canvas is partitioned into families and each becomes its own diagram:
//! - `flowchart`: section boxes, sticky notes, shapes, doc cards; frames
//!   become (nested) `subgraph`s; bound boxes get a
//!   `click … href "[[Note#^id]]"` back to their Markdown; outline edges are
//!   plain links, so the mind map survives.
//! - `erDiagram`: entity boxes and their relations (crow's-foot cardinality).
//! - `classDiagram`: class boxes and UML relations.
//! - `stateDiagram-v2`: shapes connected to a `[*]` start/end dot.
//! - a box holding a ```` ```mermaid ```` fence is copied verbatim.
//! - optional `mindmap` of the section outline.
//!
//! Positions, freehand strokes and edges between families have no Mermaid
//! form; they are reported in `warnings`, never silently lost. Every
//! diagram produced here parses without errors in `crate::mermaid`
//! (tested). Callers: `app::editor::canvas_io`, `api::canvas`.

use std::collections::{HashMap, HashSet};

use emath::{Pos2, Rect};

use super::CanvasDocument;
use super::diagram_kinds::{ClassRelKind, EdgeRelation, EntityAttr, ErCardinality};
use super::drawio::to_hex_color;
use super::element::{CanvasElement, CanvasElementId, ShapeKind};
use crate::markdown::blocks::strip_anchors;
use crate::markdown::sections::{SegmentKind, heading_level, summary_of};

/// What to export besides the diagrams themselves.
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// Note title: root of the mind map and target of `click` links.
    pub title: String,
    /// Also emit a `mindmap` of the section outline.
    pub mindmap: bool,
}

/// One Mermaid diagram (the text inside a ```` ```mermaid ```` fence).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ExportedDiagram {
    /// Header keyword: `flowchart`, `erDiagram`, `classDiagram`,
    /// `stateDiagram-v2`, `mindmap`, or `embedded` for a copied fence.
    pub kind: String,
    pub source: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct MermaidExport {
    pub diagrams: Vec<ExportedDiagram>,
    /// What had no Mermaid form (positions aside).
    pub warnings: Vec<String>,
}

impl MermaidExport {
    /// Markdown with one fence per diagram.
    pub fn to_markdown(&self) -> String {
        let fences: Vec<String> =
            self.diagrams.iter().map(|d| format!("```mermaid\n{}\n```\n", d.source.trim_end())).collect();
        fences.join("\n")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Family {
    Flow,
    Er,
    Class,
    State,
}

/// Mermaid-safe, unique identifiers derived from labels.
#[derive(Default)]
struct Ids {
    used: HashSet<String>,
}

const RESERVED: &[&str] = &[
    "end", "graph", "flowchart", "subgraph", "style", "class", "classdef", "click", "default", "linkstyle",
    "direction", "state", "note", "as", "call", "href",
];

impl Ids {
    fn make(&mut self, label: &str, fallback: &str) -> String {
        let mut base = String::new();
        for c in label.chars() {
            if c.is_ascii_alphanumeric() {
                base.push(c);
            } else if !base.ends_with('_') {
                base.push('_');
            }
            if base.len() >= 24 {
                break;
            }
        }
        let mut base = base.trim_matches('_').to_string();
        if !base.starts_with(|c: char| c.is_ascii_alphabetic()) {
            base = format!("{fallback}{base}");
        }
        if RESERVED.contains(&base.to_ascii_lowercase().as_str()) {
            base.push('_');
        }
        let mut id = base.clone();
        let mut n = 2;
        while !self.used.insert(id.clone()) {
            id = format!("{base}_{n}");
            n += 1;
        }
        id
    }
}

/// Flowchart label text: quotes and newlines encoded.
fn escape_label(s: &str) -> String {
    s.trim().replace('"', "#quot;").replace('\n', "<br/>")
}

/// Short label for a text box: its section summary for Markdown, else the
/// first lines of plain text.
fn text_label(text: &str, bound: bool) -> String {
    let text = strip_anchors(text);
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim_start();
    let kind = if heading_level(first).is_some() {
        SegmentKind::Section { level: 1 }
    } else if first.starts_with('|') {
        SegmentKind::Table
    } else if first.starts_with("```") || first.starts_with("~~~") {
        SegmentKind::Code { lang: first.trim_start_matches(['`', '~']).trim().to_string() }
    } else {
        SegmentKind::Text
    };
    if bound || kind != SegmentKind::Text {
        return summary_of(&kind, &text);
    }
    let joined = text.lines().map(str::trim).filter(|l| !l.is_empty()).take(3).collect::<Vec<_>>().join("\n");
    if joined.chars().count() > 120 { joined.chars().take(119).collect::<String>() + "…" } else { joined }
}

/// `flowchart` node syntax for a shape kind.
fn flow_node(id: &str, kind: ShapeKind, label: &str) -> String {
    let l = escape_label(label);
    match kind {
        ShapeKind::Rectangle => format!("{id}[\"{l}\"]"),
        ShapeKind::RoundedRect => format!("{id}(\"{l}\")"),
        ShapeKind::Stadium => format!("{id}([\"{l}\"])"),
        ShapeKind::Subroutine => format!("{id}[[\"{l}\"]]"),
        ShapeKind::Cylinder => format!("{id}[(\"{l}\")]"),
        ShapeKind::Circle | ShapeKind::Ellipse => format!("{id}((\"{l}\"))"),
        ShapeKind::Diamond => format!("{id}{{\"{l}\"}}"),
        ShapeKind::Hexagon => format!("{id}{{{{\"{l}\"}}}}"),
        ShapeKind::Parallelogram => format!("{id}[/\"{l}\"/]"),
        ShapeKind::CalloutBubble => format!("{id}>\"{l}\"]"),
        ShapeKind::StateStart => format!("{id}@{{ shape: sm-circ }}"),
        ShapeKind::StateEnd => format!("{id}@{{ shape: fr-circ }}"),
    }
}

/// Sanitized ER attribute (Mermaid wants word characters for type and name).
fn er_attr(a: &EntityAttr) -> String {
    let word = |s: &str, fallback: &str| {
        let w: String = s
            .chars()
            .map(|c| if c.is_alphanumeric() || matches!(c, '_' | '-' | '(' | ')' | '[' | ']') { c } else { '_' })
            .collect();
        if w.is_empty() { fallback.to_string() } else { w }
    };
    let mut s = format!("{} {}", word(&a.ty, "string"), word(&a.name, "field"));
    let keys: Vec<&str> = a.keys.split(',').map(str::trim).filter(|k| matches!(*k, "PK" | "FK" | "UK")).collect();
    if !keys.is_empty() {
        s.push(' ');
        s.push_str(&keys.join(", "));
    }
    if !a.comment.is_empty() {
        s.push_str(&format!(" \"{}\"", a.comment.replace('"', "'")));
    }
    s
}

/// Exports `doc` as Mermaid diagrams (see the module docs).
pub fn export_canvas(doc: &CanvasDocument, opts: &ExportOptions) -> MermaidExport {
    let mut out = MermaidExport::default();
    let nodes: Vec<&CanvasElement> =
        doc.elements.iter().filter(|e| e.is_node() && !matches!(e, CanvasElement::Frame { .. })).collect();
    let frames: Vec<&CanvasElement> = doc.elements.iter().filter(|e| matches!(e, CanvasElement::Frame { .. })).collect();
    let connectors: Vec<&CanvasElement> =
        doc.elements.iter().filter(|e| matches!(e, CanvasElement::Connector { .. })).collect();

    // --- families -------------------------------------------------------
    let mut family: HashMap<CanvasElementId, Family> = HashMap::new();
    let mut embedded: Vec<String> = Vec::new();
    let mut embedded_ids: HashSet<CanvasElementId> = HashSet::new();
    for n in &nodes {
        let f = match n {
            CanvasElement::Entity { .. } => Family::Er,
            CanvasElement::ClassBox { .. } => Family::Class,
            other => match other.text().and_then(mermaid_source) {
                Some(src) => {
                    embedded.push(src);
                    embedded_ids.insert(other.id());
                    continue;
                }
                None => Family::Flow,
            },
        };
        family.insert(n.id(), f);
    }
    // Shapes connected (transitively) to a start/end dot form a state diagram.
    let adjacency: Vec<(CanvasElementId, CanvasElementId)> = connectors
        .iter()
        .filter_map(|c| match c {
            CanvasElement::Connector { from_elem: Some(a), to_elem: Some(b), meta, .. } if !meta.outline => {
                Some((*a, *b))
            }
            _ => None,
        })
        .collect();
    let mut stack: Vec<CanvasElementId> = nodes
        .iter()
        .filter(|n| matches!(n, CanvasElement::Shape { kind, .. } if kind.is_state_marker()))
        .map(|n| n.id())
        .collect();
    while let Some(id) = stack.pop() {
        if family.get(&id) != Some(&Family::Flow) {
            continue;
        }
        family.insert(id, Family::State);
        for (a, b) in &adjacency {
            let other = if *a == id {
                *b
            } else if *b == id {
                *a
            } else {
                continue;
            };
            if family.get(&other) == Some(&Family::Flow) {
                stack.push(other);
            }
        }
    }

    // --- edges per family -------------------------------------------------
    let mut dangling = 0;
    let mut crossing = 0;
    let mut edges: HashMap<Family, Vec<&CanvasElement>> = HashMap::new();
    for c in &connectors {
        let CanvasElement::Connector { from_elem, to_elem, meta, .. } = c else { continue };
        let (Some(a), Some(b)) = (from_elem, to_elem) else {
            dangling += 1;
            continue;
        };
        // An embedded fence is its own diagram: outline edges to it live on
        // in the mind map, drawn ones can't be expressed.
        if embedded_ids.contains(a) || embedded_ids.contains(b) {
            if !meta.outline {
                crossing += 1;
            }
            continue;
        }
        match (family.get(a), family.get(b)) {
            (Some(fa), Some(fb)) if fa == fb => edges.entry(*fa).or_default().push(c),
            (Some(_), Some(_)) => crossing += 1,
            _ => dangling += 1,
        }
    }

    let members =
        |f: Family| -> Vec<&CanvasElement> { nodes.iter().copied().filter(|n| family.get(&n.id()) == Some(&f)).collect() };
    let edges_of = |f: Family| edges.get(&f).map(Vec::as_slice).unwrap_or(&[]);
    let flow_nodes = members(Family::Flow);
    if !flow_nodes.is_empty() {
        out.diagrams.push(ExportedDiagram {
            kind: "flowchart".into(),
            source: flowchart(&flow_nodes, &frames, edges_of(Family::Flow), opts),
        });
    }
    let er_nodes = members(Family::Er);
    if !er_nodes.is_empty() {
        out.diagrams.push(ExportedDiagram { kind: "erDiagram".into(), source: er_diagram(&er_nodes, edges_of(Family::Er)) });
    }
    let class_nodes = members(Family::Class);
    if !class_nodes.is_empty() {
        out.diagrams.push(ExportedDiagram {
            kind: "classDiagram".into(),
            source: class_diagram(&class_nodes, edges_of(Family::Class)),
        });
    }
    let state_nodes = members(Family::State);
    if !state_nodes.is_empty() {
        out.diagrams.push(ExportedDiagram {
            kind: "stateDiagram-v2".into(),
            source: state_diagram(&state_nodes, edges_of(Family::State)),
        });
    }
    for src in embedded {
        out.diagrams.push(ExportedDiagram { kind: "embedded".into(), source: src });
    }
    if opts.mindmap
        && let Some(src) = mindmap(doc, opts)
    {
        out.diagrams.push(ExportedDiagram { kind: "mindmap".into(), source: src });
    }

    let strokes = doc.elements.iter().filter(|e| matches!(e, CanvasElement::FreehandStroke { .. })).count();
    if strokes > 0 {
        out.warnings.push(format!("{strokes} freehand stroke(s) have no Mermaid form and were skipped"));
    }
    if dangling > 0 {
        out.warnings.push(format!("{dangling} connector(s) not attached at both ends were skipped"));
    }
    if crossing > 0 {
        out.warnings.push(format!("{crossing} connector(s) between different diagram types were skipped"));
    }
    out
}

/// Frames as nested subgraphs: `(id, rect, title)` and which frame owns what.
struct FrameTree {
    frames: Vec<(CanvasElementId, Rect, String)>,
}

impl FrameTree {
    /// Smallest frame (other than `me`) around the centre of `r`.
    fn owner(&self, r: Rect, me: Option<CanvasElementId>) -> Option<CanvasElementId> {
        self.frames
            .iter()
            .filter(|(fid, fr, _)| Some(*fid) != me && fr.contains(r.center()) && fr.area() > r.area())
            .min_by(|a, b| a.1.area().total_cmp(&b.1.area()))
            .map(|(fid, _, _)| *fid)
    }
}

fn flowchart(nodes: &[&CanvasElement], frames: &[&CanvasElement], edges: &[&CanvasElement], opts: &ExportOptions) -> String {
    let mut ids = Ids::default();
    let bounds = nodes.iter().map(|n| n.bounding_rect()).fold(Rect::NOTHING, |a, r| a.union(r));
    let dir = if bounds.width() > bounds.height() * 1.2 { "LR" } else { "TD" };
    let mut lines = vec![format!("flowchart {dir}")];
    let mut id_of: HashMap<CanvasElementId, String> = HashMap::new();
    let mut decl: Vec<(CanvasElementId, Rect, String)> = Vec::new();
    let mut styles: Vec<String> = Vec::new();
    let mut clicks: Vec<String> = Vec::new();
    for n in nodes {
        let (label, kind, fill, stroke) = match n {
            CanvasElement::Shape { kind, text, fill_color, stroke_color, binding, .. } => {
                (text_label(text, binding.is_some()), *kind, *fill_color, Some(*stroke_color))
            }
            CanvasElement::StickyNote { text, color, binding, .. } => {
                (text_label(text, binding.is_some()), ShapeKind::Rectangle, Some(*color), None)
            }
            CanvasElement::DocCard { title, .. } => (title.clone(), ShapeKind::Subroutine, None, None),
            _ => continue,
        };
        let id = ids.make(&label, "n");
        if let Some(f) = fill {
            let stroke = stroke.map(|s| format!(",stroke:{}", to_hex_color(s))).unwrap_or_default();
            styles.push(format!("style {id} fill:{}{stroke}", to_hex_color(f)));
        }
        let target = match n {
            CanvasElement::DocCard { title, .. } => Some(format!("[[{title}]]")),
            _ => n.binding().map(|b| {
                let note = b.file.as_deref().map(|f| f.trim_end_matches(".md").to_string()).unwrap_or_else(|| opts.title.clone());
                format!("[[{note}#^{}]]", b.block_id)
            }),
        };
        if let Some(t) = target {
            clicks.push(format!("click {id} href \"{}\"", t.replace('"', "'")));
        }
        decl.push((n.id(), n.bounding_rect(), flow_node(&id, kind, if label.is_empty() { " " } else { &label })));
        id_of.insert(n.id(), id);
    }

    let tree = FrameTree {
        frames: frames
            .iter()
            .filter_map(|f| match f {
                CanvasElement::Frame { rect, title, .. } => Some((
                    f.id(),
                    Rect::from_min_max(Pos2::new(rect[0], rect[1]), Pos2::new(rect[2], rect[3])),
                    title.clone(),
                )),
                _ => None,
            })
            .collect(),
    };
    let mut frame_ids: HashMap<CanvasElementId, (String, String)> = HashMap::new();
    for (fid, _, title) in &tree.frames {
        let id = ids.make(if title.is_empty() { "group" } else { title }, "g");
        frame_ids.insert(*fid, (id, if title.is_empty() { " ".into() } else { title.clone() }));
    }
    let mut children: HashMap<Option<CanvasElementId>, Vec<String>> = HashMap::new();
    for (_, r, d) in &decl {
        children.entry(tree.owner(*r, None)).or_default().push(d.clone());
    }
    let mut sub_frames: HashMap<Option<CanvasElementId>, Vec<CanvasElementId>> = HashMap::new();
    for (fid, fr, _) in &tree.frames {
        sub_frames.entry(tree.owner(*fr, Some(*fid))).or_default().push(*fid);
    }
    // Depth-first emission; `(frame, depth, closing)`.
    let mut stack: Vec<(Option<CanvasElementId>, usize, bool)> = vec![(None, 1, false)];
    let mut seen: HashSet<CanvasElementId> = HashSet::new();
    while let Some((frame, depth, closing)) = stack.pop() {
        let pad = "  ".repeat(depth.saturating_sub(1).max(1));
        if closing {
            lines.push(format!("{pad}end"));
            continue;
        }
        let inner = "  ".repeat(depth);
        if let Some(f) = frame {
            let (id, title) = &frame_ids[&f];
            lines.push(format!("{pad}subgraph {id}[\"{}\"]", escape_label(title)));
            stack.push((frame, depth, true));
        }
        for c in children.get(&frame).into_iter().flatten() {
            lines.push(format!("{inner}{c}"));
        }
        for f in sub_frames.get(&frame).into_iter().flatten().rev() {
            if seen.insert(*f) {
                stack.push((Some(*f), depth + 1, false));
            }
        }
    }

    for e in edges {
        let CanvasElement::Connector { from_elem: Some(a), to_elem: Some(b), label, arrow_end, meta, .. } = e else {
            continue;
        };
        let (Some(a), Some(b)) = (id_of.get(a), id_of.get(b)) else { continue };
        let arrow = match (meta.dashed, *arrow_end) {
            (true, true) => "-.->",
            (true, false) => "-.-",
            (false, true) => "-->",
            (false, false) => "---",
        };
        let label = label.trim().replace(['|', '\n'], " ");
        if label.is_empty() {
            lines.push(format!("  {a} {arrow} {b}"));
        } else {
            lines.push(format!("  {a} {arrow}|\"{}\"| {b}", label.replace('"', "#quot;")));
        }
    }
    lines.extend(styles.into_iter().map(|s| format!("  {s}")));
    lines.extend(clicks.into_iter().map(|s| format!("  {s}")));
    lines.join("\n")
}

fn er_diagram(nodes: &[&CanvasElement], edges: &[&CanvasElement]) -> String {
    let mut ids = Ids::default();
    let mut lines = vec!["erDiagram".to_string()];
    let mut id_of: HashMap<CanvasElementId, String> = HashMap::new();
    for n in nodes {
        let CanvasElement::Entity { name, attributes, .. } = n else { continue };
        let id = ids.make(name, "E");
        let head = if id == *name { id.clone() } else { format!("{id}[\"{}\"]", name.replace('"', "'")) };
        if attributes.is_empty() {
            lines.push(format!("  {head}"));
        } else {
            lines.push(format!("  {head} {{"));
            lines.extend(attributes.iter().map(|a| format!("    {}", er_attr(a))));
            lines.push("  }".into());
        }
        id_of.insert(n.id(), id);
    }
    for e in edges {
        let CanvasElement::Connector { from_elem: Some(a), to_elem: Some(b), label, meta, .. } = e else { continue };
        let (Some(a), Some(b)) = (id_of.get(a), id_of.get(b)) else { continue };
        let (from, to, identifying) = match &meta.relation {
            Some(EdgeRelation::Er { from, to, identifying }) => (*from, *to, *identifying),
            _ => (ErCardinality::ExactlyOne, ErCardinality::ZeroOrMore, !meta.dashed),
        };
        let line = if identifying { "--" } else { ".." };
        let label = label.trim().replace('"', "'").replace('\n', " ");
        lines.push(format!("  {a} {}{line}{} {b} : \"{label}\"", from.left_token(), to.right_token()));
    }
    lines.join("\n")
}

fn class_diagram(nodes: &[&CanvasElement], edges: &[&CanvasElement]) -> String {
    let mut ids = Ids::default();
    let mut lines = vec!["classDiagram".to_string()];
    let mut id_of: HashMap<CanvasElementId, String> = HashMap::new();
    for n in nodes {
        let CanvasElement::ClassBox { name, annotation, attributes, methods, .. } = n else { continue };
        let id = ids.make(name, "C");
        let head = if id == *name { format!("class {id}") } else { format!("class {id}[\"{}\"]", name.replace('"', "'")) };
        let members: Vec<&String> = attributes.iter().chain(methods.iter()).collect();
        if members.is_empty() && annotation.is_empty() {
            lines.push(format!("  {head}"));
        } else {
            lines.push(format!("  {head} {{"));
            if !annotation.is_empty() {
                lines.push(format!("    <<{annotation}>>"));
            }
            lines.extend(members.iter().map(|m| format!("    {}", m.replace(['{', '}'], ""))));
            lines.push("  }".into());
        }
        id_of.insert(n.id(), id);
    }
    for e in edges {
        let CanvasElement::Connector { from_elem: Some(a), to_elem: Some(b), label, arrow_end, meta, .. } = e else {
            continue;
        };
        let (Some(a), Some(b)) = (id_of.get(a), id_of.get(b)) else { continue };
        let (kind, card_from, card_to) = match &meta.relation {
            Some(EdgeRelation::Class { kind, card_from, card_to }) => (*kind, card_from.as_str(), card_to.as_str()),
            _ if *arrow_end => (ClassRelKind::Association, "", ""),
            _ => (ClassRelKind::Link, "", ""),
        };
        let card = |s: &str| if s.is_empty() { String::new() } else { format!("\"{}\" ", s.replace('"', "'")) };
        let mut l = format!("  {a} {}{} {}{b}", card(card_from), kind.arrow(), card(card_to));
        let label = label.trim().replace('\n', " ");
        if !label.is_empty() {
            l.push_str(&format!(" : {label}"));
        }
        lines.push(l);
    }
    lines.join("\n")
}

fn state_diagram(nodes: &[&CanvasElement], edges: &[&CanvasElement]) -> String {
    let mut ids = Ids::default();
    let mut lines = vec!["stateDiagram-v2".to_string()];
    let mut id_of: HashMap<CanvasElementId, String> = HashMap::new();
    for n in nodes {
        match n {
            CanvasElement::Shape { kind, .. } if kind.is_state_marker() => {
                id_of.insert(n.id(), "[*]".into());
            }
            _ => {
                let label = n.text().map(|t| text_label(t, n.is_bound())).unwrap_or_default();
                let id = ids.make(&label, "s");
                if label.is_empty() || label == id {
                    lines.push(format!("  {id}"));
                } else {
                    lines.push(format!("  state \"{}\" as {id}", label.replace(['"', '\n'], " ")));
                }
                id_of.insert(n.id(), id);
            }
        }
    }
    for e in edges {
        let CanvasElement::Connector { from_elem: Some(a), to_elem: Some(b), label, .. } = e else { continue };
        let (Some(a), Some(b)) = (id_of.get(a), id_of.get(b)) else { continue };
        let label = label.trim().replace(['\n', ':'], " ");
        if label.is_empty() {
            lines.push(format!("  {a} --> {b}"));
        } else {
            lines.push(format!("  {a} --> {b} : {label}"));
        }
    }
    lines.join("\n")
}

/// `mindmap` of the section outline (outline edges), rooted at the title.
fn mindmap(doc: &CanvasDocument, opts: &ExportOptions) -> Option<String> {
    let by_id: HashMap<CanvasElementId, &CanvasElement> = doc
        .elements
        .iter()
        .filter(|e| e.binding().is_some_and(|b| b.is_segment()))
        .map(|e| (e.id(), e))
        .collect();
    if by_id.is_empty() {
        return None;
    }
    let mut children: HashMap<CanvasElementId, Vec<CanvasElementId>> = HashMap::new();
    let mut has_parent: HashSet<CanvasElementId> = HashSet::new();
    for e in &doc.elements {
        if let CanvasElement::Connector { from_elem: Some(a), to_elem: Some(b), meta, .. } = e
            && meta.outline
        {
            children.entry(*a).or_default().push(*b);
            has_parent.insert(*b);
        }
    }
    let clean = |s: &str| -> String {
        let t: String = s.chars().filter(|c| !matches!(c, '(' | ')' | '[' | ']' | '{' | '}')).collect();
        let t = t.trim().to_string();
        if t.is_empty() { "…".into() } else { t }
    };
    // Top to bottom order within a level reads like the note.
    let top = |id: &CanvasElementId| by_id.get(id).map(|e| e.bounding_rect().min.y).unwrap_or(0.0);
    let mut lines = vec!["mindmap".to_string(), format!("  root(({}))", clean(&opts.title))];
    let mut roots: Vec<CanvasElementId> = by_id.keys().copied().filter(|id| !has_parent.contains(id)).collect();
    roots.sort_by(|a, b| top(a).total_cmp(&top(b)));
    let mut stack: Vec<(CanvasElementId, usize)> = roots.into_iter().rev().map(|r| (r, 2)).collect();
    let mut seen = HashSet::new();
    while let Some((id, depth)) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(e) = by_id.get(&id) else { continue };
        let label = e.text().map(|t| text_label(t, true)).unwrap_or_default();
        lines.push(format!("{}{}", "  ".repeat(depth), clean(&label)));
        let mut kids = children.get(&id).cloned().unwrap_or_default();
        kids.sort_by(|a, b| top(a).total_cmp(&top(b)));
        stack.extend(kids.into_iter().rev().map(|k| (k, depth + 1)));
    }
    Some(lines.join("\n"))
}

/// Inner source of a ```` ```mermaid ```` fence, if `text` is one.
pub fn mermaid_source(text: &str) -> Option<String> {
    let mut lines = text.lines();
    let first = lines.next()?.trim();
    let marker = if first.starts_with("```") { "```" } else if first.starts_with("~~~") { "~~~" } else { return None };
    if first.trim_start_matches(['`', '~']).split_whitespace().next() != Some("mermaid") {
        return None;
    }
    let body: Vec<&str> = lines.take_while(|l| !l.trim().starts_with(marker)).collect();
    Some(body.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::diagram_kinds::ConnectorMeta;
    use crate::canvas::element::ConnectorRouting;
    use crate::canvas::outline::build_outline_canvas;
    use crate::markdown::sections::segments;
    use crate::mermaid;

    fn connector(a: CanvasElementId, b: CanvasElementId, meta: ConnectorMeta, label: &str) -> CanvasElement {
        CanvasElement::Connector {
            id: CanvasElementId::new(),
            from_elem: Some(a),
            to_elem: Some(b),
            from_pos: [0.0, 0.0],
            to_pos: [1.0, 1.0],
            routing: ConnectorRouting::Straight,
            stroke_color: [0.0; 3],
            stroke_width: 1.0,
            label: label.into(),
            arrow_end: true,
            waypoints: Vec::new(),
            meta,
        }
    }

    fn shape(kind: ShapeKind, x: f32, text: &str) -> CanvasElement {
        CanvasElement::Shape {
            id: CanvasElementId::new(),
            kind,
            rect: [x, 0.0, x + 100.0, 50.0],
            stroke_color: [0.2; 3],
            stroke_width: 1.0,
            fill_color: None,
            text: text.into(),
            text_color: None,
            binding: None,
        }
    }

    fn entity(name: &str, x: f32, attributes: Vec<EntityAttr>) -> CanvasElement {
        CanvasElement::Entity { id: CanvasElementId::new(), rect: [x, 600.0, x + 200.0, 700.0], name: name.into(), attributes, color: [0.4; 3] }
    }

    fn class(name: &str, x: f32, annotation: &str, attributes: Vec<String>, methods: Vec<String>) -> CanvasElement {
        CanvasElement::ClassBox {
            id: CanvasElementId::new(),
            rect: [x, 900.0, x + 200.0, 1000.0],
            name: name.into(),
            annotation: annotation.into(),
            attributes,
            methods,
            color: [0.5; 3],
        }
    }

    pub(crate) fn assert_valid(export: &MermaidExport) {
        for d in &export.diagrams {
            let (_, diags) = mermaid::validate(&d.source);
            assert!(diags.iter().all(|x| !x.is_error()), "{}:\n{}\n{diags:?}", d.kind, d.source);
        }
    }

    #[test]
    fn mixed_canvas_exports_one_valid_diagram_per_family() {
        let mut doc = CanvasDocument::new("Proyek");
        let body = "# Produk ^p\nVisi \"hebat\".\n\n## Data ^d\nisi\n\n```mermaid\nflowchart LR\n  X-->Y\n```\n^m\n";
        doc.elements.extend(build_outline_canvas("Proyek", &segments(body)).elements);
        let a = shape(ShapeKind::Hexagon, 900.0, "Mulai (awal)");
        let b = shape(ShapeKind::Cylinder, 1100.0, "DB");
        let (ai, bi) = (a.id(), b.id());
        doc.add_element(a);
        doc.add_element(b);
        doc.add_element(connector(ai, bi, ConnectorMeta { dashed: true, ..Default::default() }, "baca|tulis"));
        doc.add_element(CanvasElement::Frame {
            id: CanvasElementId::new(),
            rect: [880.0, -20.0, 1220.0, 80.0],
            title: "Backend".into(),
            color: [0.5; 3],
        });
        let key = EntityAttr { ty: "int".into(), name: "id".into(), keys: "PK".into(), comment: "kunci".into() };
        let cust = entity("Customer Account", 0.0, vec![key]);
        let order = entity("ORDER", 300.0, vec![]);
        let (ci, oi) = (cust.id(), order.id());
        doc.add_element(cust);
        doc.add_element(order);
        let er = EdgeRelation::Er { from: ErCardinality::ExactlyOne, to: ErCardinality::ZeroOrMore, identifying: false };
        doc.add_element(connector(ci, oi, ConnectorMeta { relation: Some(er), ..Default::default() }, "places"));
        let animal = class("Animal", 0.0, "interface", vec!["+String name".into()], vec!["+speak() void".into()]);
        let dog = class("Dog", 300.0, "", vec![], vec![]);
        let (an, dg) = (animal.id(), dog.id());
        doc.add_element(animal);
        doc.add_element(dog);
        let inh = EdgeRelation::Class { kind: ClassRelKind::Inheritance, card_from: "1".into(), card_to: "*".into() };
        doc.add_element(connector(an, dg, ConnectorMeta { relation: Some(inh), ..Default::default() }, ""));
        let start = shape(ShapeKind::StateStart, 0.0, "");
        let idle = shape(ShapeKind::RoundedRect, 200.0, "Idle state");
        let end = shape(ShapeKind::StateEnd, 400.0, "");
        let (s, i, e) = (start.id(), idle.id(), end.id());
        for el in [start, idle, end] {
            doc.add_element(el);
        }
        doc.add_element(connector(s, i, ConnectorMeta::default(), ""));
        doc.add_element(connector(i, e, ConnectorMeta::default(), "selesai"));
        doc.add_element(CanvasElement::FreehandStroke {
            id: CanvasElementId::new(),
            points: vec![[0.0, 0.0], [1.0, 1.0]],
            color: [0.0; 3],
            width: 1.0,
        });
        doc.add_element(connector(ai, ci, ConnectorMeta::default(), ""));

        let export = export_canvas(&doc, &ExportOptions { title: "Proyek".into(), mindmap: true });
        let kinds: Vec<&str> = export.diagrams.iter().map(|d| d.kind.as_str()).collect();
        assert_eq!(kinds, vec!["flowchart", "erDiagram", "classDiagram", "stateDiagram-v2", "embedded", "mindmap"]);
        assert_valid(&export);
        let flow = &export.diagrams[0].source;
        assert!(flow.contains("subgraph Backend[\"Backend\"]"), "{flow}");
        assert!(flow.contains("-.->|\"baca tulis\"|"), "{flow}");
        assert!(flow.contains("click Produk href \"[[Proyek#^p]]\""), "{flow}");
        assert!(flow.contains("Produk --- Data"), "{flow}");
        let er = &export.diagrams[1].source;
        assert!(er.contains("Customer_Account[\"Customer Account\"] {"), "{er}");
        assert!(er.contains("||..o{ ORDER : \"places\""), "{er}");
        let class = &export.diagrams[2].source;
        assert!(class.contains("Animal \"1\" <|-- \"*\" Dog"), "{class}");
        let state = &export.diagrams[3].source;
        assert!(state.contains("[*] --> Idle_state") && state.contains("Idle_state --> [*] : selesai"), "{state}");
        assert_eq!(export.diagrams[4].source, "flowchart LR\n  X-->Y");
        let mm = &export.diagrams[5].source;
        assert!(mm.starts_with("mindmap\n  root((Proyek))\n    Produk\n      Data"), "{mm}");
        assert_eq!(export.warnings.len(), 2, "{:?}", export.warnings);
        assert_eq!(export.to_markdown().matches("```mermaid").count(), 6);
    }

    #[test]
    fn ids_are_unique_and_never_reserved() {
        let mut ids = Ids::default();
        assert_eq!(ids.make("end", "n"), "end_");
        assert_eq!(ids.make("A b", "n"), "A_b");
        assert_eq!(ids.make("A b", "n"), "A_b_2");
        assert_eq!(ids.make("123", "n"), "n123");
        assert_eq!(ids.make("", "n"), "n");
    }
}

#[cfg(test)]
mod mermaid_source_tests {
    use super::mermaid_source;

    #[test]
    fn mermaid_source_extracts_fence_body() {
        assert_eq!(
            mermaid_source("```mermaid\nflowchart LR\n  A-->B\n```").as_deref(),
            Some("flowchart LR\n  A-->B")
        );
        assert_eq!(mermaid_source("```rust\nx\n```"), None);
        assert_eq!(mermaid_source("# Judul"), None);
    }
}
