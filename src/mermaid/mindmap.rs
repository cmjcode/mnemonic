//! Mermaid mind maps (`mindmap`) — parser and scene (§3.7.6, §3.9.4).
//! Hierarchy comes from indentation; node shapes `id[square]`,
//! `id(rounded)`, `id((circle))`, `id))bang((`, `id)cloud(`,
//! `id{{hexagon}}` or plain text; `::icon(…)` and `:::class` lines are
//! accepted and ignored. Layout: the root in the middle, first-level
//! branches alternating right / left, each side a tidy tree growing
//! outward; every branch keeps one palette colour and joins its parent
//! with a curve. The canvas' section outline exports to this type.
//! Callers: `mermaid::render`/`mermaid::validate`.

use super::diag::Diagnostic;
use super::scene::{Anchor, P, Prim, Scene, Stroke};
use super::source::{Config, Source};
use super::text::{TextMeasure, clean_label};
use super::theme::{Theme, text_on};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NodeShape {
    /// Plain text with an underline.
    #[default]
    Default,
    Square,
    Rounded,
    Circle,
    Bang,
    Cloud,
    Hexagon,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MindNode {
    pub id: String,
    pub text: String,
    pub shape: NodeShape,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// 1-based source line.
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mindmap {
    /// Node 0 is the root (when there is one).
    pub nodes: Vec<MindNode>,
}

/// `(id, text, shape)` of one node line.
fn parse_node(t: &str) -> (String, String, NodeShape) {
    const SHAPES: [(&str, &str, NodeShape); 6] = [
        ("((", "))", NodeShape::Circle),
        ("))", "((", NodeShape::Bang),
        ("{{", "}}", NodeShape::Hexagon),
        ("[", "]", NodeShape::Square),
        ("(", ")", NodeShape::Rounded),
        (")", "(", NodeShape::Cloud),
    ];
    // The earliest opening delimiter wins; at the same position the longer
    // one (`((` before `(`).
    let first = SHAPES
        .iter()
        .filter_map(|(open, close, shape)| t.find(open).map(|i| (i, *open, *close, *shape)))
        .filter(|(i, open, close, _)| t.ends_with(close) && t.len() >= i + open.len() + close.len())
        .min_by_key(|(i, open, ..)| (*i, std::cmp::Reverse(open.len())));
    if let Some((i, open, close, shape)) = first {
        let inner = &t[i + open.len()..t.len() - close.len()];
        let id = t[..i].trim();
        let text = clean_label(inner);
        return (if id.is_empty() { text.clone() } else { id.to_string() }, text, shape);
    }
    let text = clean_label(t);
    (text.clone(), text, NodeShape::Default)
}

pub fn parse(src: &Source<'_>) -> (Mindmap, Vec<Diagnostic>) {
    let mut mm = Mindmap::default();
    let mut diags = Vec::new();
    // (indent, node index) of the open ancestors.
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for line in src.body() {
        let t = line.text.trim();
        if t.is_empty() || t.starts_with("::icon(") || t.starts_with(":::") {
            continue;
        }
        let t = t.split(":::").next().unwrap_or(t).trim();
        while stack.last().is_some_and(|(ind, _)| *ind >= line.indent) {
            stack.pop();
        }
        let parent = stack.last().map(|(_, i)| *i);
        if parent.is_none() && !mm.nodes.is_empty() {
            diags.push(Diagnostic::error(line.no, line.indent + 1, "a mindmap has one root: indent this node under it"));
            continue;
        }
        let (id, text, shape) = parse_node(t);
        if text.is_empty() {
            diags.push(Diagnostic::error(line.no, line.indent + 1, "empty mindmap node"));
            continue;
        }
        let idx = mm.nodes.len();
        mm.nodes.push(MindNode { id, text, shape, parent, children: Vec::new(), line: line.no });
        if let Some(p) = parent {
            mm.nodes[p].children.push(idx);
        }
        stack.push((line.indent, idx));
    }
    if mm.nodes.is_empty() {
        let (no, col) = src.header().map(|h| (h.no, h.indent + 1)).unwrap_or((1, 1));
        diags.push(Diagnostic::warning(no, col, "empty mindmap: add an indented root node"));
    }
    (mm, diags)
}

const COL_GAP: f32 = 56.0;
const ROW_GAP: f32 = 14.0;

/// `[w, h]` of a node's box.
fn node_size(n: &MindNode, depth: usize, theme: &Theme, measure: &dyn TextMeasure) -> [f32; 2] {
    let font = if depth == 0 { theme.font_size * 1.2 } else { theme.font_size };
    let (w, h) = measure.size(&n.text, font);
    let pad = if depth == 0 { 28.0 } else { 16.0 };
    let (w, h) = (w + pad * 2.0, h + pad);
    match n.shape {
        NodeShape::Circle => {
            let d = w.max(h);
            [d, d]
        }
        _ => [w.max(40.0), h.max(28.0)],
    }
}

/// Height of the band the children of a node need.
fn band(kids: &[usize], span: &[f32]) -> f32 {
    kids.iter().map(|c| span[*c]).sum::<f32>() + ROW_GAP * kids.len().saturating_sub(1) as f32
}

pub fn build(mm: &Mindmap, _config: &Config, theme: &Theme, measure: &dyn TextMeasure) -> Scene {
    let mut scene = Scene::new(theme.background);
    if mm.nodes.is_empty() {
        return scene;
    }
    let n = mm.nodes.len();
    let mut depth = vec![0usize; n];
    let mut branch = vec![0usize; n];
    for i in 1..n {
        if let Some(p) = mm.nodes[i].parent {
            depth[i] = depth[p] + 1;
            branch[i] = if depth[i] == 1 {
                mm.nodes[p].children.iter().position(|c| *c == i).unwrap_or(0)
            } else {
                branch[p]
            };
        }
    }
    let sizes: Vec<[f32; 2]> = (0..n).map(|i| node_size(&mm.nodes[i], depth[i], theme, measure)).collect();
    // Subtree band heights (children come after parents in source order).
    let mut span = vec![0f32; n];
    for i in (0..n).rev() {
        span[i] = sizes[i][1].max(band(&mm.nodes[i].children, &span));
    }
    let max_depth = depth.iter().copied().max().unwrap_or(0);
    let mut col_w = vec![0f32; max_depth + 1];
    for i in 0..n {
        col_w[depth[i]] = col_w[depth[i]].max(sizes[i][0]);
    }
    // Centre x of each depth column, measured outward from the root centre.
    let mut col_x = vec![0f32; max_depth + 1];
    for d in 1..=max_depth {
        col_x[d] = col_x[d - 1] + col_w[d - 1] / 2.0 + COL_GAP + col_w[d] / 2.0;
    }
    let mut center = vec![[0f32; 2]; n];
    // 1st, 3rd, … branch to the right; 2nd, 4th, … to the left.
    let root_kids = &mm.nodes[0].children;
    let right: Vec<usize> = root_kids.iter().step_by(2).copied().collect();
    let left: Vec<usize> = root_kids.iter().skip(1).step_by(2).copied().collect();
    for (side, kids) in [(1.0f32, right), (-1.0f32, left)] {
        let mut stack: Vec<(usize, f32)> = Vec::new();
        let mut top = -band(&kids, &span) / 2.0;
        for c in &kids {
            stack.push((*c, top));
            top += span[*c] + ROW_GAP;
        }
        while let Some((i, top)) = stack.pop() {
            center[i] = [side * col_x[depth[i]], top + span[i] / 2.0];
            let kids = &mm.nodes[i].children;
            let mut t = top + (span[i] - band(kids, &span)) / 2.0;
            for c in kids {
                stack.push((*c, t));
                t += span[*c] + ROW_GAP;
            }
        }
    }

    // Edges first (under the nodes): a smooth S-curve per link.
    for i in 1..n {
        let Some(p) = mm.nodes[i].parent else { continue };
        let (a, b) = (center[p], center[i]);
        let side = if b[0] >= a[0] { 1.0 } else { -1.0 };
        let from = [a[0] + side * sizes[p][0] / 2.0, a[1]];
        let to = [b[0] - side * sizes[i][0] / 2.0, b[1]];
        let mid = (from[0] + to[0]) / 2.0;
        let pts: Vec<P> = (0..=16)
            .map(|k| {
                let t = k as f32 / 16.0;
                let u = 1.0 - t;
                // Cubic Bézier with horizontal tangents at both ends.
                let x = u * u * u * from[0] + 3.0 * u * u * t * mid + 3.0 * u * t * t * mid + t * t * t * to[0];
                let y = u * u * u * from[1] + 3.0 * u * u * t * from[1] + 3.0 * u * t * t * to[1] + t * t * t * to[1];
                [x, y]
            })
            .collect();
        let width = (4.0 - depth[i] as f32).max(1.5);
        scene.line(pts, Stroke::new(theme.palette_color(branch[i]), width));
    }

    for i in 0..n {
        let node = &mm.nodes[i];
        let [w, h] = sizes[i];
        let c = center[i];
        let min = [c[0] - w / 2.0, c[1] - h / 2.0];
        let fill = if i == 0 { theme.primary } else { theme.palette_color(branch[i]) };
        let font = if i == 0 { theme.font_size * 1.2 } else { theme.font_size };
        let stroke = Some(Stroke::new(theme.primary_border, 1.0));
        let hit = [min[0], min[1], min[0] + w, min[1] + h];
        match (i, node.shape) {
            (0, NodeShape::Default) | (_, NodeShape::Circle) => {
                scene.push(Prim::Ellipse { center: c, radius: [w / 2.0, h / 2.0], fill, stroke })
            }
            (_, NodeShape::Default) => {
                // Mermaid's default: text over a coloured underline.
                scene.line(vec![[min[0], min[1] + h], [min[0] + w, min[1] + h]], Stroke::new(fill, 3.0));
                scene.text(measure, c, &node.text, font, theme.text, Anchor::Middle, false);
                scene.hit(hit, node.id.clone(), node.line);
                continue;
            }
            (_, NodeShape::Square) => scene.rect(min, [w, h], 0.0, fill, stroke),
            (_, NodeShape::Rounded) => scene.rect(min, [w, h], 8.0, fill, stroke),
            (_, NodeShape::Hexagon) => {
                let k = h / 3.0;
                let pts = vec![
                    [min[0] + k, min[1]],
                    [min[0] + w - k, min[1]],
                    [min[0] + w, c[1]],
                    [min[0] + w - k, min[1] + h],
                    [min[0] + k, min[1] + h],
                    [min[0], c[1]],
                ];
                scene.polygon(pts, fill, stroke);
            }
            (_, NodeShape::Bang | NodeShape::Cloud) => {
                // Scalloped outline: spiky for bang, puffy for cloud.
                let inner = if node.shape == NodeShape::Bang { 0.78 } else { 0.9 };
                let spikes = 28;
                let pts: Vec<P> = (0..spikes)
                    .map(|k| {
                        let a = k as f32 / spikes as f32 * std::f32::consts::TAU;
                        let r = if k % 2 == 0 { 1.0 } else { inner };
                        [c[0] + a.cos() * w / 2.0 * r, c[1] + a.sin() * h / 2.0 * r]
                    })
                    .collect();
                scene.polygon(pts, fill, stroke);
            }
        }
        scene.text(measure, c, &node.text, font, text_on(fill), Anchor::Middle, i == 0);
        scene.hit(hit, node.id.clone(), node.line);
    }
    scene.fit(12.0);
    scene
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mermaid::source::preprocess;
    use crate::mermaid::text::ApproxMeasure;

    const SRC: &str = "mindmap\n  root((Proyek))\n    Produk\n      Visi\n    id1[Data]\n      ::icon(fa fa-db)\n      Tabel\n    Tim:::urgent\n    b))Risiko((\n";

    #[test]
    fn parses_hierarchy_and_shapes() {
        let (mm, d) = parse(&preprocess(SRC));
        assert!(d.is_empty(), "{d:?}");
        let texts: Vec<&str> = mm.nodes.iter().map(|n| n.text.as_str()).collect();
        assert_eq!(texts, vec!["Proyek", "Produk", "Visi", "Data", "Tabel", "Tim", "Risiko"]);
        assert_eq!(mm.nodes[0].shape, NodeShape::Circle);
        assert_eq!(mm.nodes[3].shape, NodeShape::Square);
        assert_eq!(mm.nodes[3].id, "id1");
        assert_eq!(mm.nodes[6].shape, NodeShape::Bang);
        assert_eq!(mm.nodes[0].children, vec![1, 3, 5, 6]);
        assert_eq!(mm.nodes[4].parent, Some(3));
    }

    #[test]
    fn builds_branches_on_both_sides() {
        let s = preprocess(SRC);
        let (mm, _) = parse(&s);
        let sc = build(&mm, &s.config, &Theme::default_theme(), &ApproxMeasure);
        assert_eq!(sc.hits.len(), 7);
        let x = |id: &str| {
            let h = sc.hits.iter().find(|h| h.id == id).unwrap();
            (h.rect[0] + h.rect[2]) / 2.0
        };
        let root = x("root");
        assert!(x("Produk") > root && x("Tim") > root, "1st and 3rd branch go right");
        assert!(x("id1") < root && x("b") < root, "2nd and 4th branch go left");
        assert!(x("Visi") > x("Produk") && x("Tabel") < x("id1"), "grandchildren grow outward");
    }

    #[test]
    fn second_root_is_an_error() {
        let (_, d) = parse(&preprocess("mindmap\n  A\n  B\n"));
        assert_eq!(d.len(), 1);
        assert!(d[0].is_error());
    }
}
