//! Flowchart AST → `Scene`: resolve styles (theme ← `classDef default` ←
//! classes ← `style`), measure labels, run the layered layout, clip and
//! curve edges, then emit clusters, edges, nodes and labels (in that
//! paint order) plus hit regions carrying source lines and `click` links.
//! Callers: `mermaid::render`.

use super::shapes::{clip_shape, draw, label_offset, node_size, shows_label};
use super::{Flowchart, LinkStroke, Shape};
use crate::mermaid::layout::layered::{self, ClusterIn, EdgeIn, End, Graph, NodeIn};
use crate::mermaid::route::{Curve, route};
use crate::mermaid::scene::{Anchor, ClipShape, Hit, P, Scene, Stroke, polyline_midpoint};
use crate::mermaid::source::Config;
use crate::mermaid::text::{TextMeasure, wrap};
use crate::mermaid::theme::{Color, StyleSpec, Theme};

pub fn build(fc: &Flowchart, config: &Config, title: Option<&str>, theme: &Theme, measure: &dyn TextMeasure) -> Scene {
    let font = theme.font_size;
    let pad = config.f32(&["flowchart", "padding"]).unwrap_or(15.0).clamp(2.0, 60.0);
    let wrap_w = config.f32(&["flowchart", "wrappingWidth"]).unwrap_or(200.0).max(40.0);
    let curve = Curve::from_name(config.str(&["flowchart", "curve"]));
    let horizontal = fc.dir.is_horizontal();

    // Styles.
    let base = StyleSpec {
        fill: Some(theme.primary),
        stroke: Some(theme.primary_border),
        stroke_width: Some(1.0),
        color: Some(theme.primary_text),
        ..StyleSpec::default()
    };
    let resolve_style = |classes: &[String], own: &StyleSpec, mut s: StyleSpec| -> StyleSpec {
        if let Some(d) = fc.class_def("default") {
            s.apply(d);
        }
        for c in classes {
            if let Some(d) = fc.class_def(c) {
                s.apply(d);
            }
        }
        s.apply(own);
        s
    };
    let node_styles: Vec<StyleSpec> =
        fc.nodes.iter().map(|n| resolve_style(&n.classes, &n.style, base.clone())).collect();

    // Labels and sizes.
    let mut texts = Vec::with_capacity(fc.nodes.len());
    let mut sizes = Vec::with_capacity(fc.nodes.len());
    for (i, n) in fc.nodes.iter().enumerate() {
        let size_px = node_styles[i].font_size.unwrap_or(font);
        let mut text = n.text().to_string();
        if n.markdown || config.bool(&["wrap"]) == Some(true) {
            text = wrap(&text, measure, size_px, wrap_w);
        }
        let (tw, th) = if shows_label(n.shape) { measure.size(&text, size_px) } else { (0.0, 0.0) };
        sizes.push(node_size(n.shape, tw, th, pad, horizontal));
        texts.push(text);
    }

    // Graph.
    let mut g = Graph::new(fc.dir);
    g.node_sep = config.f32(&["flowchart", "nodeSpacing"]).unwrap_or(50.0).clamp(5.0, 400.0);
    g.rank_sep = config.f32(&["flowchart", "rankSpacing"]).unwrap_or(50.0).clamp(5.0, 400.0);
    for sg in &fc.subgraphs {
        let (tw, th) = if sg.title.is_empty() { (0.0, 0.0) } else { measure.size(&sg.title, font) };
        g.clusters.push(ClusterIn { parent: sg.parent, dir: sg.dir, title: [tw, th] });
    }
    for (i, n) in fc.nodes.iter().enumerate() {
        g.nodes.push(NodeIn { size: sizes[i], cluster: n.subgraph });
    }
    let end_of =
        |id: &str| -> Option<End> { fc.subgraph_index(id).map(End::Cluster).or_else(|| fc.node_index(id).map(End::Node)) };
    let mut edge_map = Vec::with_capacity(fc.edges.len());
    for e in &fc.edges {
        let (Some(from), Some(to)) = (end_of(&e.from), end_of(&e.to)) else {
            edge_map.push(None);
            continue;
        };
        let label = e.label.as_ref().filter(|l| !l.is_empty()).map(|l| {
            let (w, h) = measure.size(l, font);
            [w + 8.0, h + 4.0]
        });
        edge_map.push(Some(g.edges.len()));
        g.edges.push(EdgeIn { from, to, minlen: e.minlen, weight: 1.0, label });
    }
    let lay = layered::layout(&g);

    let mut scene = Scene::new(theme.background);
    let bg = theme.background;

    // Clusters, outermost first.
    let depth = |mut c: usize| {
        let mut d = 0;
        while let Some(p) = fc.subgraphs[c].parent {
            d += 1;
            c = p;
            if d > fc.subgraphs.len() {
                break;
            }
        }
        d
    };
    let mut order: Vec<usize> = (0..fc.subgraphs.len()).collect();
    order.sort_by_key(|&c| depth(c));
    let cluster_base = StyleSpec {
        fill: Some(theme.cluster_bkg),
        stroke: Some(theme.cluster_border),
        stroke_width: Some(1.0),
        color: Some(theme.text),
        ..StyleSpec::default()
    };
    for c in order {
        let Some(r) = lay.clusters[c] else { continue };
        let sg = &fc.subgraphs[c];
        let style = resolve_style(&sg.classes, &sg.style, cluster_base.clone());
        scene.rect(
            [r[0], r[1]],
            [r[2] - r[0], r[3] - r[1]],
            0.0,
            style.fill.unwrap_or(theme.cluster_bkg),
            Some(stroke_of(&style, theme.cluster_border, 1.0)),
        );
        if !sg.title.is_empty() {
            let th = g.clusters[c].title[1];
            let color = style.color.unwrap_or(theme.text);
            scene.text(measure, [(r[0] + r[2]) / 2.0, r[1] + 6.0 + th / 2.0], &sg.title, font, color, Anchor::Middle, false);
        }
        scene.hit(r, sg.id.clone(), sg.line);
    }

    // Edges.
    let mut labels: Vec<(P, [f32; 2], String, Color)> = Vec::new();
    for (i, e) in fc.edges.iter().enumerate() {
        let Some(gi) = edge_map[i] else { continue };
        let out = &lay.edges[gi];
        let from = node_geom(fc, &lay, g.edges[gi].from, &sizes);
        let to = node_geom(fc, &lay, g.edges[gi].to, &sizes);
        let Some(pts) = route(out, &lay, from, to, curve) else { continue };
        let mut style = StyleSpec::default();
        for (idx, s) in &fc.link_styles {
            if idx.is_none() || *idx == Some(i) {
                style.apply(s);
            }
        }
        let (width, dash) = match e.stroke {
            LinkStroke::Normal | LinkStroke::Invisible => (1.6, None),
            LinkStroke::Thick => (3.5, None),
            LinkStroke::Dotted => (1.6, Some([3.0, 3.0])),
        };
        if e.stroke != LinkStroke::Invisible {
            let stroke = Stroke {
                color: style.stroke.unwrap_or(theme.line),
                width: style.stroke_width.unwrap_or(width),
                dash: style.dash.or(dash),
            };
            scene.edge(pts.clone(), stroke, e.start, e.end, bg);
        }
        if let Some(text) = e.label.as_ref().filter(|l| !l.is_empty()) {
            let pos = out.label.unwrap_or_else(|| polyline_midpoint(&pts));
            let size = g.edges[gi].label.unwrap_or([0.0, 0.0]);
            labels.push((pos, size, text.clone(), style.color.unwrap_or(theme.text)));
        }
    }

    // Nodes.
    for (i, n) in fc.nodes.iter().enumerate() {
        let c = lay.nodes[i];
        let st = &node_styles[i];
        let stroke = stroke_of(st, theme.primary_border, 1.0);
        draw(&mut scene, n.shape, c, sizes[i], st.fill.unwrap_or(theme.primary), stroke, theme.line);
        if shows_label(n.shape) && !texts[i].is_empty() {
            let off = label_offset(n.shape, sizes[i]);
            let color = if n.shape == Shape::Text { st.color.unwrap_or(theme.text) } else { st.color.unwrap_or(theme.primary_text) };
            let size_px = st.font_size.unwrap_or(font);
            let bold = st.bold.unwrap_or(false);
            scene.text(measure, [c[0] + off[0], c[1] + off[1]], &texts[i], size_px, color, Anchor::Middle, bold);
        }
        let s = sizes[i];
        scene.hits.push(Hit {
            rect: [c[0] - s[0] / 2.0, c[1] - s[1] / 2.0, c[0] + s[0] / 2.0, c[1] + s[1] / 2.0],
            id: n.id.clone(),
            line: n.line,
            link: n.link.clone(),
            tooltip: n.tooltip.clone(),
        });
    }

    // Edge labels on top.
    for (pos, size, text, color) in labels {
        scene.rect([pos[0] - size[0] / 2.0, pos[1] - size[1] / 2.0], size, 2.0, theme.edge_label_bg, None);
        scene.text(measure, pos, &text, font, color, Anchor::Middle, false);
    }

    if let Some(t) = title.filter(|t| !t.is_empty())
        && let Some(b) = scene.bounds()
    {
        scene.text(measure, [(b[0] + b[2]) / 2.0, b[1] - font * 1.6], t, font * 1.15, theme.text, Anchor::Middle, true);
    }
    scene.fit(8.0);
    scene
}

fn stroke_of(style: &StyleSpec, default: Color, width: f32) -> Stroke {
    Stroke { color: style.stroke.unwrap_or(default), width: style.stroke_width.unwrap_or(width), dash: style.dash }
}

/// Centre, half-size and outline of a node edge end.
fn node_geom(fc: &Flowchart, lay: &layered::Layout, end: End, sizes: &[[f32; 2]]) -> (P, P, ClipShape) {
    match end {
        End::Node(n) if n < sizes.len() => {
            (lay.nodes[n], [sizes[n][0] / 2.0, sizes[n][1] / 2.0], clip_shape(fc.nodes[n].shape))
        }
        _ => ([0.0, 0.0], [0.0, 0.0], ClipShape::Rect),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mermaid::flowchart::parse;
    use crate::mermaid::scene::{Prim, prim_bounds};
    use crate::mermaid::source::preprocess;
    use crate::mermaid::text::ApproxMeasure;

    fn scene(src: &str) -> Scene {
        let s = preprocess(src);
        let (fc, d) = parse(&s);
        assert!(d.iter().all(|x| !x.is_error()), "{d:?}");
        build(&fc, &s.config, s.title.as_deref(), &Theme::default_theme(), &ApproxMeasure)
    }

    #[test]
    fn builds_nodes_edges_labels_and_hits() {
        let sc = scene(
            "flowchart TD\nA[Christmas] -->|Get money| B(Go shopping)\nB --> C{Let me think}\nC -->|One| D[Laptop]\nC -->|Two| E[iPhone]\nC -->|Three| F[fa:fa-car Car]\n",
        );
        assert!(sc.width > 100.0 && sc.height > 200.0);
        assert_eq!(sc.hits.len(), 6);
        let texts: Vec<&str> = sc
            .prims
            .iter()
            .filter_map(|p| if let Prim::Text { text, .. } = p { Some(text.as_str()) } else { None })
            .collect();
        assert!(texts.contains(&"Christmas") && texts.contains(&"Get money") && texts.contains(&"Let me think"));
        let markers = sc.prims.iter().filter(|p| matches!(p, Prim::Marker { .. })).count();
        assert_eq!(markers, 5);
        assert_eq!(sc.hits.iter().find(|h| h.id == "C").unwrap().line, 3);
        for p in &sc.prims {
            let b = prim_bounds(p);
            assert!(b[0] >= -0.5 && b[1] >= -0.5 && b[2] <= sc.width + 0.5 && b[3] <= sc.height + 0.5, "{p:?}");
        }
    }

    #[test]
    fn edges_touch_node_borders_not_centres() {
        let sc = scene("flowchart LR\nA --> B\n");
        let a = sc.hits.iter().find(|h| h.id == "A").unwrap().rect;
        let line = sc
            .prims
            .iter()
            .find_map(|p| if let Prim::Line { points, .. } = p { Some(points.clone()) } else { None })
            .unwrap();
        assert!((line[0][0] - a[2]).abs() < 1.0, "edge starts at A's right border: {:?} vs {a:?}", line[0]);
    }

    #[test]
    fn subgraphs_render_as_titled_boxes() {
        let sc = scene("flowchart TB\nsubgraph one [Group One]\na1 --> a2\nend\nc --> one\n");
        assert!(sc.hits.iter().any(|h| h.id == "one"));
        assert!(sc.prims.iter().any(|p| matches!(p, Prim::Text { text, .. } if text == "Group One")));
    }

    #[test]
    fn styles_apply_in_order() {
        let sc = scene("flowchart LR\nclassDef default fill:#010101\nclassDef hot fill:#ff0000\nA:::hot --> B\nstyle B fill:#00ff00\n");
        let fills: Vec<Color> = sc
            .prims
            .iter()
            .filter_map(|p| if let Prim::Rect { fill, radius, .. } = p { (*radius == 0.0).then_some(*fill) } else { None })
            .collect();
        assert!(fills.contains(&[255, 0, 0, 255]));
        assert!(fills.contains(&[0, 255, 0, 255]));
    }
}
