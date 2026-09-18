//! Class diagram → `Scene` (§3.7.6): three-compartment class boxes
//! («annotations» + name / attributes / methods), relations with UML end
//! markers (hollow triangle, filled/hollow diamond, open arrow, lollipop),
//! dashed dependency/realization lines, cardinalities beside the ends,
//! namespaces as clusters and notes tied by dashed connectors.
//! Callers: `mermaid::render`.

use super::ClassDiagram;
use crate::mermaid::layout::layered::{self, ClusterIn, EdgeIn, End, Graph, NodeIn};
use crate::mermaid::route::{Curve, EndGeom, route};
use crate::mermaid::scene::{Anchor, ClipShape, Hit, P, Scene, Stroke, polyline_midpoint};
use crate::mermaid::source::Config;
use crate::mermaid::text::{TextMeasure, line_height};
use crate::mermaid::theme::{StyleSpec, Theme};

/// Heights of the header and attribute compartments, and the box size.
struct BoxGeom {
    size: [f32; 2],
    header: f32,
    attrs: f32,
}

fn class_geom(c: &super::Class, measure: &dyn TextMeasure, font: f32) -> BoxGeom {
    let lh = line_height(font);
    let small = font * 0.85;
    let mut w: f32 = measure.line_width(&c.label, font) * 1.06;
    for a in &c.annotations {
        w = w.max(measure.line_width(&format!("«{a}»"), small));
    }
    for m in c.attributes.iter().chain(&c.methods) {
        w = w.max(measure.line_width(m, font));
    }
    let header = lh + c.annotations.len() as f32 * line_height(small) + 14.0;
    let attrs = c.attributes.len() as f32 * lh + 10.0;
    let methods = c.methods.len() as f32 * lh + 10.0;
    BoxGeom { size: [(w + 24.0).max(80.0), header + attrs + methods], header, attrs }
}

pub fn build(d: &ClassDiagram, config: &Config, title: Option<&str>, theme: &Theme, measure: &dyn TextMeasure) -> Scene {
    let font = theme.font_size;
    let geoms: Vec<BoxGeom> = d.classes.iter().map(|c| class_geom(c, measure, font)).collect();

    let mut g = Graph::new(d.dir);
    g.node_sep = config.f32(&["class", "nodeSpacing"]).unwrap_or(50.0);
    g.rank_sep = config.f32(&["class", "rankSpacing"]).unwrap_or(60.0);
    for (name, _) in &d.namespaces {
        let (w, h) = measure.size(name, font);
        g.clusters.push(ClusterIn { parent: None, dir: None, title: [w, h] });
    }
    for (i, c) in d.classes.iter().enumerate() {
        g.nodes.push(NodeIn { size: geoms[i].size, cluster: c.namespace });
    }
    for r in &d.relations {
        let label = r.label.as_ref().filter(|l| !l.is_empty()).map(|l| {
            let (w, h) = measure.size(l, font);
            [w + 8.0, h + 4.0]
        });
        g.edges.push(EdgeIn { from: End::Node(r.from), to: End::Node(r.to), minlen: 1, weight: 1.0, label });
    }
    let first_note = g.nodes.len();
    for n in &d.notes {
        let (w, h) = measure.size(&n.text, font);
        let cluster = n.target.and_then(|t| d.classes[t].namespace);
        g.nodes.push(NodeIn { size: [w + 20.0, h + 16.0], cluster });
        if let Some(t) = n.target {
            let note = End::Node(g.nodes.len() - 1);
            g.edges.push(EdgeIn { from: note, to: End::Node(t), minlen: 1, weight: 0.3, label: None });
        }
    }
    let lay = layered::layout(&g);

    let mut scene = Scene::new(theme.background);
    let bg = theme.background;
    for (k, (name, line)) in d.namespaces.iter().enumerate() {
        let Some(r) = lay.clusters[k] else { continue };
        let border = Some(Stroke::new(theme.cluster_border, 1.0));
        scene.rect([r[0], r[1]], [r[2] - r[0], r[3] - r[1]], 0.0, theme.cluster_bkg, border);
        let th = g.clusters[k].title[1];
        scene.text(measure, [(r[0] + r[2]) / 2.0, r[1] + 6.0 + th / 2.0], name, font, theme.text, Anchor::Middle, true);
        scene.hit(r, name.clone(), *line);
    }

    let geom = |n: usize| -> EndGeom {
        let s = g.nodes[n].size;
        (lay.nodes[n], [s[0] / 2.0, s[1] / 2.0], ClipShape::Rect)
    };
    let curve = Curve::from_name(config.str(&["class", "curve"]));
    let mut labels: Vec<(P, [f32; 2], String)> = Vec::new();
    for (k, e) in g.edges.iter().enumerate() {
        let (End::Node(a), End::Node(b)) = (e.from, e.to) else { continue };
        let note_edge = k >= d.relations.len();
        let c = if note_edge { Curve::Linear } else { curve };
        let Some(pts) = route(&lay.edges[k], &lay, geom(a), geom(b), c) else { continue };
        if note_edge {
            scene.line(pts, Stroke::dashed(theme.note_border, 1.0, [3.0, 3.0]));
            continue;
        }
        let r = &d.relations[k];
        let stroke = if r.dashed { Stroke::dashed(theme.line, 1.3, [5.0, 4.0]) } else { Stroke::new(theme.line, 1.3) };
        scene.edge(pts.clone(), stroke, r.start, r.end, bg);
        for (card, at_start) in [(&r.card_from, true), (&r.card_to, false)] {
            let Some(card) = card.as_deref().filter(|c| !c.is_empty()) else { continue };
            scene.text(measure, card_position(&pts, at_start), card, font * 0.85, theme.text, Anchor::Middle, false);
        }
        if let (Some(text), Some(size)) = (r.label.as_ref(), e.label) {
            labels.push((lay.edges[k].label.unwrap_or_else(|| polyline_midpoint(&pts)), size, text.clone()));
        }
    }

    for (i, c) in d.classes.iter().enumerate() {
        let geo = &geoms[i];
        let center = lay.nodes[i];
        let (x0, y0) = (center[0] - geo.size[0] / 2.0, center[1] - geo.size[1] / 2.0);
        let mut st = StyleSpec::default();
        for cls in &c.classes {
            if let Some(def) = d.class_def(cls) {
                st.apply(def);
            }
        }
        st.apply(&c.style);
        let border = Stroke::new(st.stroke.unwrap_or(theme.primary_border), st.stroke_width.unwrap_or(1.2));
        let color = st.color.unwrap_or(theme.primary_text);
        scene.rect([x0, y0], geo.size, 0.0, st.fill.unwrap_or(theme.primary), Some(border));
        let small = font * 0.85;
        let mut y = y0 + 7.0;
        for a in &c.annotations {
            let lh = line_height(small);
            scene.text(measure, [center[0], y + lh / 2.0], &format!("«{a}»"), small, color, Anchor::Middle, false);
            y += lh;
        }
        let lh = line_height(font);
        scene.text(measure, [center[0], y + lh / 2.0], &c.label, font, color, Anchor::Middle, true);
        let (x1, split1, split2) = (x0 + geo.size[0], y0 + geo.header, y0 + geo.header + geo.attrs);
        scene.line(vec![[x0, split1], [x1, split1]], border);
        scene.line(vec![[x0, split2], [x1, split2]], border);
        for (k, m) in c.attributes.iter().enumerate() {
            let pos = [x0 + 10.0, split1 + 5.0 + lh * (k as f32 + 0.5)];
            scene.text(measure, pos, m, font, color, Anchor::Start, false);
        }
        for (k, m) in c.methods.iter().enumerate() {
            let pos = [x0 + 10.0, split2 + 5.0 + lh * (k as f32 + 0.5)];
            scene.text(measure, pos, m, font, color, Anchor::Start, false);
        }
        scene.hits.push(Hit {
            rect: [x0, y0, x1, y0 + geo.size[1]],
            id: c.id.clone(),
            line: c.line,
            link: c.link.clone(),
            tooltip: c.tooltip.clone(),
        });
    }

    for (k, n) in d.notes.iter().enumerate() {
        let i = first_note + k;
        let (c, size) = (lay.nodes[i], g.nodes[i].size);
        let r = [c[0] - size[0] / 2.0, c[1] - size[1] / 2.0, c[0] + size[0] / 2.0, c[1] + size[1] / 2.0];
        scene.rect([r[0], r[1]], size, 0.0, theme.note_bkg, Some(Stroke::new(theme.note_border, 1.0)));
        scene.text(measure, c, &n.text, font, theme.note_text, Anchor::Middle, false);
        scene.hit(r, "note", n.line);
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

/// A spot beside an edge end for its cardinality: 18px along the edge,
/// 12px to the side.
fn card_position(pts: &[P], at_start: bool) -> P {
    let n = pts.len();
    let (a, b) = if at_start { (pts[0], pts[1.min(n - 1)]) } else { (pts[n - 1], pts[n.saturating_sub(2)]) };
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = (dx * dx + dy * dy).sqrt().max(1e-3);
    let (ux, uy) = (dx / len, dy / len);
    [a[0] + ux * 18.0 - uy * 12.0, a[1] + uy * 18.0 + ux * 12.0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mermaid::class::parse;
    use crate::mermaid::scene::{Marker, Prim};
    use crate::mermaid::source::preprocess;
    use crate::mermaid::text::ApproxMeasure;

    #[test]
    fn draws_class_boxes_relations_and_cardinalities() {
        let s = preprocess("classDiagram\n  class Animal {\n    <<abstract>>\n    +String name\n    +eat() void\n  }\n  Animal <|-- Duck\n  Customer \"1\" --> \"*\" Order : places\n  note for Duck \"quacks\"\n");
        let (d, diags) = parse(&s);
        assert!(diags.is_empty(), "{diags:?}");
        let sc = build(&d, &s.config, None, &Theme::default_theme(), &ApproxMeasure);
        let texts: Vec<&str> = sc
            .prims
            .iter()
            .filter_map(|p| if let Prim::Text { text, .. } = p { Some(text.as_str()) } else { None })
            .collect();
        for t in ["«abstract»", "Animal", "+String name", "+eat() void", "1", "*", "places", "quacks"] {
            assert!(texts.contains(&t), "missing {t}: {texts:?}");
        }
        assert!(sc.prims.iter().any(|p| matches!(p, Prim::Marker { kind: Marker::Triangle, .. })));
        assert!(sc.prims.iter().any(|p| matches!(p, Prim::Marker { kind: Marker::OpenArrow, .. })));
        let ids = ["Animal", "Duck", "Customer", "Order"];
        assert_eq!(sc.hits.iter().filter(|h| ids.contains(&h.id.as_str())).count(), 4);
    }
}
