//! State diagram → `Scene` (§3.7.6): composite states and concurrent
//! regions become clusters of the layered layout (so isolated composites
//! get their own `direction`), simple states rounded boxes (with a
//! divider and description lines when described), `[*]` dots/bullseyes,
//! fork/join bars, choice diamonds and notes (kept beside their state by
//! an invisible layout edge). Callers: `mermaid::render`.

use super::{State, StateDiagram, StateKind};
use crate::mermaid::layout::layered::{self, ClusterIn, EdgeIn, End, Graph, NodeIn};
use crate::mermaid::route::{Curve, EndGeom, route};
use crate::mermaid::scene::{Anchor, ClipShape, Hit, Marker, P, Prim, Scene, Stroke, polyline_midpoint};
use crate::mermaid::source::Config;
use crate::mermaid::text::{TextMeasure, line_height};
use crate::mermaid::theme::{StyleSpec, Theme};

#[derive(Clone, Copy)]
enum Slot {
    Node(usize),
    Cluster(usize),
}

pub fn build(d: &StateDiagram, config: &Config, title: Option<&str>, theme: &Theme, measure: &dyn TextMeasure) -> Scene {
    let font = theme.font_size;
    let horizontal = d.dir.is_horizontal();
    let is_cluster = |i: usize| d.states[i].composite || d.states[i].kind == StateKind::Region;

    // Slots: clusters for composites/regions, nodes for the rest.
    let mut slots: Vec<Slot> = Vec::with_capacity(d.states.len());
    let (mut n_nodes, mut n_clusters) = (0, 0);
    for i in 0..d.states.len() {
        if is_cluster(i) {
            slots.push(Slot::Cluster(n_clusters));
            n_clusters += 1;
        } else {
            slots.push(Slot::Node(n_nodes));
            n_nodes += 1;
        }
    }
    let cluster_of =
        |parent: Option<usize>| parent.and_then(|p| if let Slot::Cluster(c) = slots[p] { Some(c) } else { None });

    let mut g = Graph::new(d.dir);
    g.node_sep = config.f32(&["state", "nodeSpacing"]).unwrap_or(50.0);
    g.rank_sep = config.f32(&["state", "rankSpacing"]).unwrap_or(50.0);
    let mut node_state: Vec<usize> = Vec::new();
    let mut cluster_state: Vec<usize> = Vec::new();
    for (i, s) in d.states.iter().enumerate() {
        match slots[i] {
            Slot::Cluster(_) => {
                let title = if s.kind == StateKind::Region || s.label.is_empty() {
                    [0.0, 0.0]
                } else {
                    let (w, h) = measure.size(&s.label, font);
                    [w, h + 4.0]
                };
                g.clusters.push(ClusterIn { parent: cluster_of(s.parent), dir: s.dir, title });
                cluster_state.push(i);
            }
            Slot::Node(_) => {
                g.nodes.push(NodeIn { size: state_size(s, measure, font, horizontal), cluster: cluster_of(s.parent) });
                node_state.push(i);
            }
        }
    }
    let end = |i: usize| match slots[i] {
        Slot::Node(n) => End::Node(n),
        Slot::Cluster(c) => End::Cluster(c),
    };
    for t in &d.transitions {
        let label = t.label.as_ref().filter(|l| !l.is_empty()).map(|l| {
            let (w, h) = measure.size(l, font);
            [w + 8.0, h + 4.0]
        });
        g.edges.push(EdgeIn { from: end(t.from), to: end(t.to), minlen: 1, weight: 1.0, label });
    }
    // Notes: extra nodes tied to their state by an invisible edge.
    let first_note_node = g.nodes.len();
    for note in &d.notes {
        let (w, h) = measure.size(&note.text, font);
        g.nodes.push(NodeIn { size: [w + 20.0, h + 16.0], cluster: cluster_of(d.states[note.target].parent) });
        let note_node = End::Node(g.nodes.len() - 1);
        let (from, to) = if note.left { (note_node, end(note.target)) } else { (end(note.target), note_node) };
        g.edges.push(EdgeIn { from, to, minlen: 1, weight: 0.3, label: None });
    }
    let lay = layered::layout(&g);

    let mut scene = Scene::new(theme.background);
    let bg = theme.background;
    let style_of = |i: usize| -> StyleSpec {
        let mut s = StyleSpec::default();
        for c in &d.states[i].classes {
            if let Some(def) = d.class_def(c) {
                s.apply(def);
            }
        }
        s
    };

    // Composites (outermost first) and regions.
    let depth = |mut s: usize| {
        let mut k = 0;
        while let Some(p) = d.states[s].parent {
            s = p;
            k += 1;
            if k > d.states.len() {
                break;
            }
        }
        k
    };
    let mut order: Vec<usize> = (0..cluster_state.len()).collect();
    order.sort_by_key(|&c| depth(cluster_state[c]));
    for c in order {
        let Some(r) = lay.clusters[c] else { continue };
        let si = cluster_state[c];
        let s = &d.states[si];
        if s.kind == StateKind::Region {
            let border = Stroke::dashed(theme.primary_border, 1.0, [5.0, 4.0]);
            let size = [r[2] - r[0] - 8.0, r[3] - r[1] - 8.0];
            scene.rect([r[0] + 4.0, r[1] + 4.0], size, 0.0, [0, 0, 0, 0], Some(border));
            continue;
        }
        let st = style_of(si);
        let border = Stroke::new(st.stroke.unwrap_or(theme.primary_border), st.stroke_width.unwrap_or(1.0));
        scene.rect([r[0], r[1]], [r[2] - r[0], r[3] - r[1]], 6.0, st.fill.unwrap_or(theme.tertiary), Some(border));
        let th = g.clusters[c].title[1];
        if th > 0.0 {
            let pos = [(r[0] + r[2]) / 2.0, r[1] + 6.0 + th / 2.0];
            scene.text(measure, pos, &s.label, font, st.color.unwrap_or(theme.text), Anchor::Middle, true);
            scene.line(vec![[r[0], r[1] + th + 10.0], [r[2], r[1] + th + 10.0]], border);
        }
        scene.hit(r, s.id.clone(), s.line);
    }

    // Transitions and note connectors.
    let geom = |e: End| -> EndGeom {
        match e {
            End::Node(n) => {
                let size = g.nodes[n].size;
                let shape = match node_state.get(n).map(|&si| d.states[si].kind) {
                    Some(StateKind::Start | StateKind::End) => ClipShape::Ellipse,
                    Some(StateKind::Choice) => ClipShape::Diamond,
                    _ => ClipShape::Rect,
                };
                (lay.nodes[n], [size[0] / 2.0, size[1] / 2.0], shape)
            }
            End::Cluster(_) => ([0.0, 0.0], [0.0, 0.0], ClipShape::Rect),
        }
    };
    let mut labels: Vec<(P, [f32; 2], String)> = Vec::new();
    let curve = Curve::from_name(config.str(&["state", "curve"]));
    for (k, e) in g.edges.iter().enumerate() {
        let note_edge = k >= d.transitions.len();
        let c = if note_edge { Curve::Linear } else { curve };
        let Some(pts) = route(&lay.edges[k], &lay, geom(e.from), geom(e.to), c) else { continue };
        if note_edge {
            scene.line(pts, Stroke::dashed(theme.note_border, 1.0, [3.0, 3.0]));
            continue;
        }
        scene.edge(pts.clone(), Stroke::new(theme.line, 1.4), None, Some(Marker::Arrow), bg);
        if let (Some(text), Some(size)) = (d.transitions[k].label.as_ref(), e.label) {
            labels.push((lay.edges[k].label.unwrap_or_else(|| polyline_midpoint(&pts)), size, text.clone()));
        }
    }

    // States.
    for (n, &si) in node_state.iter().enumerate() {
        let s = &d.states[si];
        let (c, size) = (lay.nodes[n], g.nodes[n].size);
        draw_state(&mut scene, s, c, size, &style_of(si), theme, font, measure);
        let (x0, y0) = (c[0] - size[0] / 2.0, c[1] - size[1] / 2.0);
        scene.hits.push(Hit { rect: [x0, y0, x0 + size[0], y0 + size[1]], id: s.id.clone(), line: s.line, link: None, tooltip: None });
    }

    for (k, note) in d.notes.iter().enumerate() {
        let n = first_note_node + k;
        let (c, size) = (lay.nodes[n], g.nodes[n].size);
        let r = [c[0] - size[0] / 2.0, c[1] - size[1] / 2.0, c[0] + size[0] / 2.0, c[1] + size[1] / 2.0];
        scene.rect([r[0], r[1]], size, 0.0, theme.note_bkg, Some(Stroke::new(theme.note_border, 1.0)));
        scene.text(measure, c, &note.text, font, theme.note_text, Anchor::Middle, false);
        scene.hit(r, "note", note.line);
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

#[allow(clippy::too_many_arguments)]
fn draw_state(
    scene: &mut Scene,
    s: &State,
    c: P,
    size: [f32; 2],
    st: &StyleSpec,
    theme: &Theme,
    font: f32,
    measure: &dyn TextMeasure,
) {
    let bg = theme.background;
    let fill = st.fill.unwrap_or(theme.primary);
    let stroke = Stroke::new(st.stroke.unwrap_or(theme.primary_border), st.stroke_width.unwrap_or(1.0));
    let text_color = st.color.unwrap_or(theme.primary_text);
    let (x0, y0) = (c[0] - size[0] / 2.0, c[1] - size[1] / 2.0);
    match s.kind {
        StateKind::Start => scene.push(Prim::Ellipse { center: c, radius: [7.0, 7.0], fill: theme.line, stroke: None }),
        StateKind::End => {
            scene.push(Prim::Ellipse { center: c, radius: [7.0, 7.0], fill: bg, stroke: Some(Stroke::new(theme.line, 1.5)) });
            scene.push(Prim::Ellipse { center: c, radius: [4.0, 4.0], fill: theme.line, stroke: None });
        }
        StateKind::Fork | StateKind::Join => scene.rect([x0, y0], size, 2.0, theme.line, None),
        StateKind::Choice => {
            let (hw, hh) = (size[0] / 2.0, size[1] / 2.0);
            let pts = vec![[c[0], c[1] - hh], [c[0] + hw, c[1]], [c[0], c[1] + hh], [c[0] - hw, c[1]]];
            scene.polygon(pts, fill, Some(stroke));
        }
        StateKind::Normal | StateKind::Region => {
            scene.rect([x0, y0], size, 6.0, fill, Some(stroke));
            if s.descriptions.is_empty() {
                scene.text(measure, c, &s.label, font, text_color, Anchor::Middle, false);
            } else {
                let th = line_height(font);
                scene.text(measure, [c[0], y0 + 8.0 + th / 2.0], &s.label, font, text_color, Anchor::Middle, true);
                scene.line(vec![[x0, y0 + th + 14.0], [x0 + size[0], y0 + th + 14.0]], stroke);
                let desc = s.descriptions.join("\n");
                let dh = measure.size(&desc, font).1;
                scene.text(measure, [x0 + 12.0, y0 + th + 20.0 + dh / 2.0], &desc, font, text_color, Anchor::Start, false);
            }
        }
    }
}

fn state_size(s: &State, measure: &dyn TextMeasure, font: f32, horizontal: bool) -> [f32; 2] {
    match s.kind {
        StateKind::Start | StateKind::End => [14.0, 14.0],
        StateKind::Fork | StateKind::Join => {
            if horizontal {
                [8.0, 70.0]
            } else {
                [70.0, 8.0]
            }
        }
        StateKind::Choice => [28.0, 28.0],
        StateKind::Normal | StateKind::Region => {
            let (tw, th) = measure.size(&s.label, font);
            if s.descriptions.is_empty() {
                [(tw + 30.0).max(50.0), th + 16.0]
            } else {
                let (dw, dh) = measure.size(&s.descriptions.join("\n"), font);
                [tw.max(dw) + 30.0, th + dh + 30.0]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mermaid::source::preprocess;
    use crate::mermaid::state::parse;
    use crate::mermaid::text::ApproxMeasure;

    fn scene(src: &str) -> Scene {
        let s = preprocess(src);
        let (d, diags) = parse(&s);
        assert!(diags.iter().all(|x| !x.is_error()), "{diags:?}");
        build(&d, &s.config, s.title.as_deref(), &Theme::default_theme(), &ApproxMeasure)
    }

    #[test]
    fn draws_states_pseudo_states_and_transitions() {
        let sc = scene("stateDiagram-v2\n  [*] --> Still\n  Still --> [*]\n  Still --> Moving\n  Moving --> Still\n  Moving --> Crash\n  Crash --> [*]\n");
        for id in ["Still", "Moving", "Crash"] {
            assert!(sc.hits.iter().any(|h| h.id == id), "{id}");
        }
        let arrows = sc.prims.iter().filter(|p| matches!(p, Prim::Marker { .. })).count();
        assert_eq!(arrows, 6);
    }

    #[test]
    fn composites_contain_their_children() {
        let sc = scene("stateDiagram-v2\n  [*] --> First\n  state First {\n    [*] --> second\n    second --> [*]\n  }\n  First --> Done\n  note right of Done : finished\n");
        let first = sc.hits.iter().find(|h| h.id == "First").unwrap().rect;
        let second = sc.hits.iter().find(|h| h.id == "second").unwrap().rect;
        assert!(second[0] >= first[0] && second[2] <= first[2] && second[1] >= first[1] && second[3] <= first[3]);
        assert!(sc.hits.iter().any(|h| h.id == "note"));
    }
}
