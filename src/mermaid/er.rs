//! Mermaid entity-relationship diagrams (`erDiagram`) — parser and scene
//! (§3.7.6): `A ||--o{ B : label` relations with crow's-foot
//! cardinalities (`||`, `|o`/`o|`, `}o`/`o{`, `}|`/`|{`), identifying
//! (`--`) and non-identifying (`..`) lines, entity attribute blocks
//! (`type name PK, FK "comment"`) drawn as tables, `E["Label"]` aliases
//! and `direction`. Callers: `mermaid::render`/`mermaid::validate`.

use super::diag::Diagnostic;
use super::layout::Dir;
use super::layout::layered::{self, EdgeIn, End, Graph, NodeIn};
use super::route::{Curve, route};
use super::scene::{Anchor, ClipShape, Hit, Marker, P, Scene, Stroke, polyline_midpoint};
use super::source::{Config, Source};
use super::text::{TextMeasure, clean_label, line_height};
use super::theme::Theme;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Attribute {
    pub ty: String,
    pub name: String,
    pub keys: String,
    pub comment: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Entity {
    pub id: String,
    pub label: String,
    pub attributes: Vec<Attribute>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Relationship {
    pub from: usize,
    pub to: usize,
    pub card_from: Marker,
    pub card_to: Marker,
    pub identifying: bool,
    pub label: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Er {
    pub dir: Dir,
    pub entities: Vec<Entity>,
    pub relationships: Vec<Relationship>,
}

impl Er {
    pub fn index(&self, id: &str) -> Option<usize> {
        self.entities.iter().position(|e| e.id == id)
    }

    /// Find or create `NAME` / `NAME["Label"]` / `NAME[Label]`.
    fn ensure(&mut self, raw: &str, line: usize) -> usize {
        let raw = raw.trim();
        let (id, label) = match raw.split_once('[') {
            Some((id, rest)) => (id.trim(), Some(clean_label(rest.trim_end_matches(']')))),
            None => (raw, None),
        };
        let i = match self.index(id) {
            Some(i) => i,
            None => {
                self.entities.push(Entity { id: id.to_string(), label: id.to_string(), line, ..Entity::default() });
                self.entities.len() - 1
            }
        };
        if let Some(l) = label {
            self.entities[i].label = l;
        }
        i
    }
}

fn left_card(s: &str) -> Option<Marker> {
    match s {
        "||" => Some(Marker::ExactlyOne),
        "|o" => Some(Marker::ZeroOrOne),
        "}o" => Some(Marker::ZeroOrMore),
        "}|" => Some(Marker::OneOrMore),
        _ => None,
    }
}

fn right_card(s: &str) -> Option<Marker> {
    match s {
        "||" => Some(Marker::ExactlyOne),
        "o|" => Some(Marker::ZeroOrOne),
        "o{" => Some(Marker::ZeroOrMore),
        "|{" => Some(Marker::OneOrMore),
        _ => None,
    }
}

struct RelSpec {
    from: String,
    card_from: Marker,
    identifying: bool,
    card_to: Marker,
    to: String,
    label: String,
}

/// `A ||--o{ B : label`; `None` when the line has no relationship line.
fn parse_relationship(t: &str) -> Option<Result<RelSpec, String>> {
    let op = t.find("--").or_else(|| t.find(".."))?;
    let missing = || Err("cardinality missing around the relationship line".to_string());
    let (Some(l), Some(r)) = (op.checked_sub(2).and_then(|s| t.get(s..op)), t.get(op + 2..op + 4)) else {
        return Some(missing());
    };
    let (Some(card_from), Some(card_to)) = (left_card(l), right_card(r)) else {
        return Some(Err(format!("unknown cardinality `{l}` … `{r}` (use ||, |o, }}o, }}| and ||, o|, o{{, |{{)")));
    };
    let from = t[..op - 2].trim().to_string();
    let rest = &t[op + 4..];
    let (to, label) = match rest.split_once(':') {
        Some((to, l)) => (to.trim().to_string(), clean_label(l.trim())),
        None => (rest.trim().to_string(), String::new()),
    };
    if from.is_empty() || to.is_empty() {
        return Some(Err("relationship needs an entity on both sides".into()));
    }
    Some(Ok(RelSpec { from, card_from, identifying: t[op..].starts_with("--"), card_to, to, label }))
}

pub fn parse(src: &Source<'_>) -> (Er, Vec<Diagnostic>) {
    let mut er = Er::default();
    let mut diags = Vec::new();
    let mut body_of: Option<usize> = None;
    for line in src.body() {
        let (no, col) = (line.no, line.indent + 1);
        let t = line.text;
        if let Some(e) = body_of {
            if t == "}" {
                body_of = None;
                continue;
            }
            let (head, comment) = match t.split_once('"') {
                Some((h, c)) => (h, c.trim_end_matches('"').to_string()),
                None => (t, String::new()),
            };
            let mut words = head.split_whitespace();
            let (Some(ty), Some(name)) = (words.next(), words.next()) else {
                diags.push(Diagnostic::error(no, col, "attribute must be `type name [PK|FK|UK] [\"comment\"]`"));
                continue;
            };
            let keys: Vec<&str> = words.collect();
            er.entities[e].attributes.push(Attribute {
                ty: super::class::generics(ty),
                name: name.to_string(),
                keys: keys.join(" "),
                comment,
            });
            continue;
        }
        if let Some(d) = t.strip_prefix("direction") {
            match Dir::parse(d) {
                Some(dir) => er.dir = dir,
                None => diags.push(Diagnostic::error(no, col, format!("unknown direction `{}`", d.trim()))),
            }
            continue;
        }
        if let Some(name) = t.strip_suffix('{') {
            let e = er.ensure(name, no);
            er.entities[e].line = no;
            body_of = Some(e);
            continue;
        }
        match parse_relationship(t) {
            Some(Ok(r)) => {
                let from = er.ensure(&r.from, no);
                let to = er.ensure(&r.to, no);
                er.relationships.push(Relationship {
                    from,
                    to,
                    card_from: r.card_from,
                    card_to: r.card_to,
                    identifying: r.identifying,
                    label: r.label,
                    line: no,
                });
            }
            Some(Err(msg)) => diags.push(Diagnostic::error(no, col, msg)),
            None if !t.contains(char::is_whitespace) || t.contains('[') => {
                er.ensure(t, no);
            }
            None => diags.push(Diagnostic::error(no, col, format!("unrecognised statement `{t}`"))),
        }
    }
    if let Some(e) = body_of {
        let ent = &er.entities[e];
        diags.push(Diagnostic::error(ent.line, 1, format!("entity `{}` block is never closed with `}}`", ent.id)));
    }
    (er, diags)
}

/// Column widths of an entity's attribute table (type, name, keys, comment).
fn columns(e: &Entity, measure: &dyn TextMeasure, font: f32) -> [f32; 4] {
    let mut w = [0.0f32; 4];
    for a in &e.attributes {
        for (k, s) in [&a.ty, &a.name, &a.keys, &a.comment].into_iter().enumerate() {
            if !s.is_empty() {
                w[k] = w[k].max(measure.line_width(s, font) + 16.0);
            }
        }
    }
    w
}

pub fn build(er: &Er, config: &Config, title: Option<&str>, theme: &Theme, measure: &dyn TextMeasure) -> Scene {
    let font = theme.font_size;
    let lh = line_height(font) + 6.0;
    let cols: Vec<[f32; 4]> = er.entities.iter().map(|e| columns(e, measure, font)).collect();
    let sizes: Vec<[f32; 2]> = er
        .entities
        .iter()
        .zip(&cols)
        .map(|(e, c)| {
            let name_w = measure.line_width(&e.label, font) * 1.06 + 30.0;
            let w = name_w.max(c.iter().sum::<f32>()).max(100.0);
            [w, lh + 8.0 + e.attributes.len() as f32 * lh]
        })
        .collect();

    let mut g = Graph::new(er.dir);
    g.node_sep = config.f32(&["er", "nodeSpacing"]).unwrap_or(60.0);
    g.rank_sep = config.f32(&["er", "rankSpacing"]).unwrap_or(70.0);
    for s in &sizes {
        g.nodes.push(NodeIn { size: *s, cluster: None });
    }
    for r in &er.relationships {
        let label = (!r.label.is_empty()).then(|| {
            let (w, h) = measure.size(&r.label, font);
            [w + 8.0, h + 4.0]
        });
        g.edges.push(EdgeIn { from: End::Node(r.from), to: End::Node(r.to), minlen: 1, weight: 1.0, label });
    }
    let lay = layered::layout(&g);

    let mut scene = Scene::new(theme.background);
    let bg = theme.background;
    let geom = |n: usize| (lay.nodes[n], [sizes[n][0] / 2.0, sizes[n][1] / 2.0], ClipShape::Rect);
    let mut labels: Vec<(P, [f32; 2], String)> = Vec::new();
    for (k, r) in er.relationships.iter().enumerate() {
        let Some(pts) = route(&lay.edges[k], &lay, geom(r.from), geom(r.to), Curve::Basis) else { continue };
        let stroke =
            if r.identifying { Stroke::new(theme.line, 1.3) } else { Stroke::dashed(theme.line, 1.3, [6.0, 4.0]) };
        scene.edge(pts.clone(), stroke, Some(r.card_from), Some(r.card_to), bg);
        if let Some(size) = g.edges[k].label {
            labels.push((lay.edges[k].label.unwrap_or_else(|| polyline_midpoint(&pts)), size, r.label.clone()));
        }
    }

    for (i, e) in er.entities.iter().enumerate() {
        let (c, size) = (lay.nodes[i], sizes[i]);
        let (x0, y0) = (c[0] - size[0] / 2.0, c[1] - size[1] / 2.0);
        scene.rect([x0, y0], size, 0.0, theme.primary, Some(Stroke::new(theme.primary_border, 1.2)));
        let head_h = lh + 8.0;
        scene.text(measure, [c[0], y0 + head_h / 2.0], &e.label, font, theme.primary_text, Anchor::Middle, true);
        let extra = (size[0] - cols[i].iter().sum::<f32>()).max(0.0);
        for (k, a) in e.attributes.iter().enumerate() {
            let ry = y0 + head_h + k as f32 * lh;
            let fill = if k % 2 == 0 { theme.secondary } else { theme.tertiary };
            scene.rect([x0, ry], [size[0], lh], 0.0, fill, Some(Stroke::new(theme.primary_border, 0.6)));
            let mut x = x0;
            for (col, text) in [&a.ty, &a.name, &a.keys, &a.comment].into_iter().enumerate() {
                if !text.is_empty() {
                    scene.text(measure, [x + 8.0, ry + lh / 2.0], text, font, theme.text, Anchor::Start, false);
                }
                x += cols[i][col] + if col == 1 { extra } else { 0.0 };
            }
        }
        let rect = [x0, y0, x0 + size[0], y0 + size[1]];
        scene.hits.push(Hit { rect, id: e.id.clone(), line: e.line, link: None, tooltip: None });
    }
    for (pos, size, text) in labels {
        scene.rect([pos[0] - size[0] / 2.0, pos[1] - size[1] / 2.0], size, 2.0, theme.edge_label_bg, None);
        scene.text(measure, pos, &text, font, theme.text, Anchor::Middle, false);
    }
    if let Some(t) = title.filter(|t| !t.is_empty())
        && let Some(b) = scene.bounds()
    {
        scene.text(measure, [(b[0] + b[2]) / 2.0, b[1] - font * 1.6], t, font * 1.15, theme.text, Anchor::Middle, true);
    }
    scene.fit(8.0);
    scene
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mermaid::scene::Prim;
    use crate::mermaid::source::preprocess;
    use crate::mermaid::text::ApproxMeasure;

    #[test]
    fn parses_relationships_and_attributes() {
        let s = preprocess("erDiagram\n  CUSTOMER ||--o{ ORDER : places\n  ORDER ||--|{ LINE-ITEM : contains\n  CUSTOMER }|..|{ DELIVERY-ADDRESS : uses\n  p[Person] |o--o| car : drives\n  CUSTOMER {\n    string name PK \"full name\"\n    int age\n  }\n");
        let (er, d) = parse(&s);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(er.relationships.len(), 4);
        let r = &er.relationships[0];
        assert_eq!((r.card_from, r.card_to, r.identifying), (Marker::ExactlyOne, Marker::ZeroOrMore, true));
        assert!(!er.relationships[2].identifying);
        assert_eq!(er.entities[er.index("p").unwrap()].label, "Person");
        let c = &er.entities[er.index("CUSTOMER").unwrap()];
        let expected = Attribute { ty: "string".into(), name: "name".into(), keys: "PK".into(), comment: "full name".into() };
        assert_eq!(c.attributes[0], expected);
        let sc = build(&er, &s.config, None, &Theme::default_theme(), &ApproxMeasure);
        assert!(sc.prims.iter().any(|p| matches!(p, Prim::Marker { kind: Marker::ZeroOrMore, .. })));
        assert!(sc.prims.iter().any(|p| matches!(p, Prim::Text { text, .. } if text == "full name")));
        assert_eq!(sc.hits.len(), er.entities.len());
    }

    #[test]
    fn bad_cardinality_is_diagnosed() {
        let (_, d) = parse(&preprocess("erDiagram\n  A ><--<> B : x\n  C {\n"));
        assert!(d.iter().any(|x| x.line == 2 && x.message.contains("cardinality")), "{d:?}");
        assert!(d.iter().any(|x| x.message.contains("never closed")), "{d:?}");
    }
}
