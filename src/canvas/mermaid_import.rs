//! Mermaid → canvas objects (§3.9.4): a `flowchart`, `erDiagram`,
//! `classDiagram` or `stateDiagram` source becomes native, editable canvas
//! elements — shapes / subgraph frames, entity tables, class boxes, state
//! dots — joined by attached connectors carrying the same relation
//! (cardinality, UML kind, dashed). Positions come from the Mermaid layout
//! itself (`Scene::hits`), so the result looks like the rendered diagram.
//! Together with `mermaid_export` this round-trips: export → import →
//! export gives the same diagram up to ids and layout. Other diagram types
//! are refused with a message (they stay embedded as a fence box).
//! Callers: `app::editor::canvas_io`, `api::canvas`.

use std::collections::HashMap;

use super::diagram_kinds::{ClassRelKind, ConnectorMeta, EdgeRelation, EntityAttr, ErCardinality};
use super::element::{CanvasElement, CanvasElementId, ConnectorRouting, ShapeKind};
use super::outline::attach_points;
use super::tools;
use crate::mermaid::flowchart::{LinkStroke, Shape as FlowShape};
use crate::mermaid::scene::Marker;
use crate::mermaid::state::StateKind;
use crate::mermaid::text::ApproxMeasure;
use crate::mermaid::{self, DiagramKind, RenderOptions, source};

const EDGE_COLOR: [f32; 3] = [0.45, 0.47, 0.52];
const NODE_STROKE: [f32; 3] = [0.40, 0.55, 0.85];

/// Elements for `src` with the diagram's top-left at world `origin`.
pub fn diagram_to_elements(src: &str, origin: [f32; 2]) -> Result<Vec<CanvasElement>, String> {
    let rendered = mermaid::render(src, &RenderOptions { dark: false, measure: &ApproxMeasure });
    if let Some(d) = rendered.diagnostics.iter().find(|d| d.is_error()) {
        return Err(format!("{}:{} {}", d.line, d.col, d.message));
    }
    let scene = rendered.scene.ok_or_else(|| format!("`{}` diagrams can't be drawn", rendered.kind.name()))?;
    let rects: HashMap<String, [f32; 4]> = scene
        .hits
        .iter()
        .map(|h| {
            let r = h.rect;
            (h.id.clone(), [r[0] + origin[0], r[1] + origin[1], r[2] + origin[0], r[3] + origin[1]])
        })
        .collect();
    let pre = source::preprocess(src);
    let mut b = Builder { out: Vec::new(), ids: HashMap::new() };
    match rendered.kind {
        DiagramKind::Flowchart => b.flowchart(&pre, &rects),
        DiagramKind::Er => b.er(&pre, &rects),
        DiagramKind::Class => b.class(&pre, &rects),
        DiagramKind::State => b.state(&pre, &rects),
        other => return Err(format!("`{}` diagrams have no canvas objects; keep them as a fence box", other.name())),
    }
    Ok(b.out)
}

struct Builder {
    out: Vec<CanvasElement>,
    ids: HashMap<String, CanvasElementId>,
}

fn rect_or(rects: &HashMap<String, [f32; 4]>, id: &str, fallback: [f32; 4]) -> [f32; 4] {
    rects.get(id).copied().unwrap_or(fallback)
}

impl Builder {
    fn node(&mut self, key: &str, elem: CanvasElement) {
        self.ids.insert(key.to_string(), elem.id());
        self.out.push(elem);
    }

    fn connect(&mut self, from: &str, to: &str, label: String, arrow_end: bool, meta: ConnectorMeta) {
        let (Some(&a), Some(&b)) = (self.ids.get(from), self.ids.get(to)) else { return };
        let rect = |id| self.out.iter().find(|e| e.id() == id).map(|e| e.bounding_rect());
        let (Some(ra), Some(rb)) = (rect(a), rect(b)) else { return };
        let (p, q) = attach_points(ra, rb);
        self.out.push(CanvasElement::Connector {
            id: CanvasElementId::new(),
            from_elem: Some(a),
            to_elem: Some(b),
            from_pos: p,
            to_pos: q,
            routing: ConnectorRouting::Orthogonal,
            stroke_color: EDGE_COLOR,
            stroke_width: 1.5,
            label,
            arrow_end,
            waypoints: Vec::new(),
            meta,
        });
    }

    fn shape(kind: ShapeKind, rect: [f32; 4], text: String) -> CanvasElement {
        CanvasElement::Shape {
            id: CanvasElementId::new(),
            kind,
            rect,
            stroke_color: NODE_STROKE,
            stroke_width: 1.5,
            fill_color: Some([0.95, 0.96, 1.0]),
            text,
            text_color: None,
            binding: None,
        }
    }

    fn flowchart(&mut self, pre: &source::Source<'_>, rects: &HashMap<String, [f32; 4]>) {
        let (fc, _) = mermaid::flowchart::parse(pre);
        for (i, n) in fc.nodes.iter().enumerate() {
            let kind = match n.shape {
                FlowShape::Rect
                | FlowShape::LinedRect
                | FlowShape::Stacked
                | FlowShape::NotchRect
                | FlowShape::Text
                | FlowShape::Document
                | FlowShape::ForkBar
                | FlowShape::Hourglass
                | FlowShape::Triangle
                | FlowShape::FlippedTriangle => ShapeKind::Rectangle,
                FlowShape::Round | FlowShape::Delay | FlowShape::Cloud => ShapeKind::RoundedRect,
                FlowShape::Stadium => ShapeKind::Stadium,
                FlowShape::Subroutine => ShapeKind::Subroutine,
                FlowShape::Cylinder | FlowShape::HCylinder => ShapeKind::Cylinder,
                FlowShape::Circle | FlowShape::DoubleCircle => ShapeKind::Circle,
                FlowShape::Diamond => ShapeKind::Diamond,
                FlowShape::Hexagon => ShapeKind::Hexagon,
                FlowShape::LeanRight | FlowShape::LeanLeft | FlowShape::Trapezoid | FlowShape::InvTrapezoid => {
                    ShapeKind::Parallelogram
                }
                FlowShape::Asymmetric | FlowShape::Flag | FlowShape::Brace | FlowShape::Bolt => ShapeKind::CalloutBubble,
                FlowShape::SmallCircle | FlowShape::FilledCircle => ShapeKind::StateStart,
                FlowShape::FramedCircle => ShapeKind::StateEnd,
            };
            let fallback = [i as f32 * 160.0, 0.0, i as f32 * 160.0 + 120.0, 60.0];
            let text = n.text().replace("<br/>", "\n").replace("<br>", "\n");
            self.node(&n.id, Self::shape(kind, rect_or(rects, &n.id, fallback), text));
        }
        // Subgraphs → frames around their members, placed under everything.
        let mut frames: Vec<CanvasElement> = Vec::new();
        for (si, sg) in fc.subgraphs.iter().enumerate() {
            let inside = |mut p: Option<usize>| {
                while let Some(x) = p {
                    if x == si {
                        return true;
                    }
                    p = fc.subgraphs[x].parent;
                }
                false
            };
            let bounds = fc
                .nodes
                .iter()
                .filter(|n| inside(n.subgraph))
                .filter_map(|n| rects.get(&n.id))
                .fold(None::<[f32; 4]>, |acc, r| {
                    Some(acc.map_or(*r, |a| [a[0].min(r[0]), a[1].min(r[1]), a[2].max(r[2]), a[3].max(r[3])]))
                });
            let Some(b) = bounds else { continue };
            let depth = std::iter::successors(sg.parent, |p| fc.subgraphs[*p].parent).count() as f32;
            let pad = (24.0 - depth * 4.0).max(8.0);
            let rect = rects.get(&sg.id).copied().unwrap_or([b[0] - pad, b[1] - pad - 18.0, b[2] + pad, b[3] + pad]);
            let frame = CanvasElement::Frame {
                id: CanvasElementId::new(),
                rect,
                title: if sg.title.is_empty() { sg.id.clone() } else { sg.title.clone() },
                color: tools::PALETTE_PRIMARY_ACCENT,
            };
            self.ids.insert(sg.id.clone(), frame.id());
            frames.push(frame);
        }
        self.out.splice(0..0, frames);
        for e in &fc.edges {
            if e.stroke == LinkStroke::Invisible {
                continue;
            }
            let meta = ConnectorMeta { dashed: e.stroke == LinkStroke::Dotted, ..Default::default() };
            self.connect(&e.from, &e.to, e.label.clone().unwrap_or_default(), e.end.is_some(), meta);
        }
    }

    fn er(&mut self, pre: &source::Source<'_>, rects: &HashMap<String, [f32; 4]>) {
        let (er, _) = mermaid::er::parse(pre);
        for (i, e) in er.entities.iter().enumerate() {
            let rows = e.attributes.len() as f32;
            let fallback = [i as f32 * 260.0, 0.0, i as f32 * 260.0 + 220.0, 40.0 + rows * 24.0];
            let mut rect = rect_or(rects, &e.id, fallback);
            rect[2] = rect[2].max(rect[0] + 200.0);
            rect[3] = rect[3].max(rect[1] + 32.0 + rows * 22.0);
            let attributes = e
                .attributes
                .iter()
                .map(|a| EntityAttr { ty: a.ty.clone(), name: a.name.clone(), keys: a.keys.clone(), comment: a.comment.clone() })
                .collect();
            let name = if e.label.is_empty() { e.id.clone() } else { e.label.clone() };
            self.node(
                &e.id,
                CanvasElement::Entity { id: CanvasElementId::new(), rect, name, attributes, color: tools::PALETTE_PRIMARY_ACCENT },
            );
        }
        let card = |m: Marker| match m {
            Marker::ZeroOrOne => ErCardinality::ZeroOrOne,
            Marker::OneOrMore => ErCardinality::OneOrMore,
            Marker::ZeroOrMore => ErCardinality::ZeroOrMore,
            _ => ErCardinality::ExactlyOne,
        };
        for r in &er.relationships {
            let relation = EdgeRelation::Er { from: card(r.card_from), to: card(r.card_to), identifying: r.identifying };
            let meta = ConnectorMeta { relation: Some(relation), ..Default::default() };
            let (from, to) = (er.entities[r.from].id.clone(), er.entities[r.to].id.clone());
            self.connect(&from, &to, r.label.clone(), false, meta);
        }
    }

    fn class(&mut self, pre: &source::Source<'_>, rects: &HashMap<String, [f32; 4]>) {
        let (cd, _) = mermaid::class::parse(pre);
        for (i, c) in cd.classes.iter().enumerate() {
            let rows = (c.attributes.len() + c.methods.len() + c.annotations.len()) as f32;
            let fallback = [i as f32 * 260.0, 0.0, i as f32 * 260.0 + 220.0, 60.0 + rows * 22.0];
            let mut rect = rect_or(rects, &c.id, fallback);
            rect[3] = rect[3].max(rect[1] + 52.0 + rows * 22.0);
            self.node(
                &c.id,
                CanvasElement::ClassBox {
                    id: CanvasElementId::new(),
                    rect,
                    name: if c.label.is_empty() { c.id.clone() } else { c.label.clone() },
                    annotation: c.annotations.first().cloned().unwrap_or_default(),
                    attributes: c.attributes.clone(),
                    methods: c.methods.clone(),
                    color: [0.62, 0.52, 0.85],
                },
            );
        }
        for r in &cd.relations {
            let (a, b) = (cd.classes[r.from].id.clone(), cd.classes[r.to].id.clone());
            // Canvas convention: inheritance / composition / aggregation mark
            // the `from` end, association / dependency / realization the `to` end.
            let (kind, swap) = match (r.start, r.end, r.dashed) {
                (Some(Marker::Triangle), _, false) => (ClassRelKind::Inheritance, false),
                (_, Some(Marker::Triangle), false) => (ClassRelKind::Inheritance, true),
                (_, Some(Marker::Triangle), true) => (ClassRelKind::Realization, false),
                (Some(Marker::Triangle), _, true) => (ClassRelKind::Realization, true),
                (Some(Marker::DiamondFilled), ..) => (ClassRelKind::Composition, false),
                (_, Some(Marker::DiamondFilled), _) => (ClassRelKind::Composition, true),
                (Some(Marker::DiamondHollow), ..) => (ClassRelKind::Aggregation, false),
                (_, Some(Marker::DiamondHollow), _) => (ClassRelKind::Aggregation, true),
                (_, Some(_), true) => (ClassRelKind::Dependency, false),
                (Some(_), _, true) => (ClassRelKind::Dependency, true),
                (_, Some(_), false) => (ClassRelKind::Association, false),
                (Some(_), _, false) => (ClassRelKind::Association, true),
                (None, None, _) => (ClassRelKind::Link, false),
            };
            let (cf, ct) = (r.card_from.clone().unwrap_or_default(), r.card_to.clone().unwrap_or_default());
            let (from, to, card_from, card_to) = if swap { (b, a, ct, cf) } else { (a, b, cf, ct) };
            let relation = EdgeRelation::Class { kind, card_from, card_to };
            let meta = ConnectorMeta { relation: Some(relation), ..Default::default() };
            self.connect(&from, &to, r.label.clone().unwrap_or_default(), false, meta);
        }
    }

    fn state(&mut self, pre: &source::Source<'_>, rects: &HashMap<String, [f32; 4]>) {
        let (sd, _) = mermaid::state::parse(pre);
        for (i, s) in sd.states.iter().enumerate() {
            if s.composite || s.kind == StateKind::Region {
                continue;
            }
            let fallback = [i as f32 * 180.0, 0.0, i as f32 * 180.0 + 140.0, 50.0];
            let r = rect_or(rects, &s.id, fallback);
            let (kind, rect, text) = match s.kind {
                StateKind::Start | StateKind::End => {
                    let c = [(r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0];
                    let k = if s.kind == StateKind::Start { ShapeKind::StateStart } else { ShapeKind::StateEnd };
                    (k, [c[0] - 12.0, c[1] - 12.0, c[0] + 12.0, c[1] + 12.0], String::new())
                }
                StateKind::Choice => (ShapeKind::Diamond, r, String::new()),
                _ => {
                    let mut t = if s.label.is_empty() { s.id.clone() } else { s.label.clone() };
                    for d in &s.descriptions {
                        t.push('\n');
                        t.push_str(d);
                    }
                    (ShapeKind::RoundedRect, r, t)
                }
            };
            self.node(&s.id, Self::shape(kind, rect, text));
        }
        for t in &sd.transitions {
            let (a, b) = (sd.states[t.from].id.clone(), sd.states[t.to].id.clone());
            self.connect(&a, &b, t.label.clone().unwrap_or_default(), true, ConnectorMeta::default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::CanvasDocument;
    use crate::canvas::mermaid_export::{ExportOptions, export_canvas};

    fn round_trip(src: &str) -> (Vec<CanvasElement>, String) {
        let elems = diagram_to_elements(src, [100.0, 100.0]).unwrap();
        let mut doc = CanvasDocument::new("T");
        doc.elements = elems.clone();
        let export = export_canvas(&doc, &ExportOptions { title: "T".into(), mindmap: false });
        assert_eq!(export.diagrams.len(), 1, "{:?}", export.diagrams);
        let out = export.diagrams[0].source.clone();
        let (_, diags) = mermaid::validate(&out);
        assert!(diags.iter().all(|d| !d.is_error()), "{out}\n{diags:?}");
        (elems, out)
    }

    fn count<F: Fn(&CanvasElement) -> bool>(e: &[CanvasElement], f: F) -> usize {
        e.iter().filter(|x| f(x)).count()
    }

    #[test]
    fn flowchart_round_trips_with_subgraph_and_dotted_edge() {
        let (elems, out) = round_trip(
            "flowchart LR\n  subgraph API\n    A[Login] --> B{Valid?}\n  end\n  B -.->|ya| C[(DB)]\n  B --> D([Selesai])\n",
        );
        assert_eq!(count(&elems, |e| matches!(e, CanvasElement::Shape { .. })), 4);
        assert_eq!(count(&elems, |e| matches!(e, CanvasElement::Frame { .. })), 1);
        assert!(out.contains("subgraph API"), "{out}");
        assert!(out.contains("Valid{\"Valid?\"}"), "{out}");
        assert!(out.contains("-.->|\"ya\"|"), "{out}");
        assert!(out.contains("DB[(\"DB\")]"), "{out}");
    }

    #[test]
    fn er_round_trips_attributes_and_cardinality() {
        let (elems, out) = round_trip(
            "erDiagram\n  CUSTOMER ||--o{ ORDER : places\n  CUSTOMER {\n    string name PK\n    int age\n  }\n  ORDER }|..|{ ITEM : contains\n",
        );
        assert_eq!(count(&elems, |e| matches!(e, CanvasElement::Entity { .. })), 3);
        assert!(out.contains("CUSTOMER ||--o{ ORDER : \"places\""), "{out}");
        assert!(out.contains("ORDER }|..|{ ITEM : \"contains\""), "{out}");
        assert!(out.contains("string name PK"), "{out}");
    }

    #[test]
    fn class_round_trips_relations_in_both_directions() {
        let (_, out) = round_trip(
            "classDiagram\n  class Animal {\n    <<interface>>\n    +speak() void\n  }\n  Animal <|-- Dog\n  Cat --|> Animal\n  Car *-- Wheel\n  A ..> B\n",
        );
        assert!(out.contains("Animal <|-- Dog"), "{out}");
        assert!(out.contains("Animal <|-- Cat"), "{out}");
        assert!(out.contains("Car *-- Wheel"), "{out}");
        assert!(out.contains("A ..> B"), "{out}");
        assert!(out.contains("<<interface>>"), "{out}");
    }

    #[test]
    fn state_round_trips_start_and_end() {
        let (elems, out) = round_trip("stateDiagram-v2\n  [*] --> Idle\n  Idle --> Run : go\n  Run --> [*]\n");
        assert_eq!(count(&elems, |e| matches!(e, CanvasElement::Shape { kind, .. } if kind.is_state_marker())), 2);
        assert!(out.contains("[*] --> Idle") && out.contains("Idle --> Run : go") && out.contains("Run --> [*]"), "{out}");
    }

    #[test]
    fn unsupported_types_are_refused() {
        assert!(diagram_to_elements("pie\n  \"a\" : 1\n", [0.0, 0.0]).is_err());
        assert!(diagram_to_elements("flowchart LR\n  A -->\n", [0.0, 0.0]).is_err());
    }
}
