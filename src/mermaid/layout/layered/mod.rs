//! Layered (Sugiyama) graph layout — the dagre-equivalent engine behind
//! flowchart, class, state, ER and requirement diagrams (§3.7.3).
//!
//! Pipeline (`flat`): cycle removal (DFS back edges reversed) → ranking
//! (longest path + balancing) → long edges split into dummy nodes, with
//! edge labels as sized dummies in the middle rank (ranks are doubled, as
//! dagre does, so every edge has a label slot) → crossing minimisation
//! (`order`: barycenter sweeps that keep subgraph members contiguous) →
//! coordinates (`position`: iterative weighted-mean alignment solved
//! exactly per layer with pool-adjacent-violators under separation
//! constraints) → edge polylines through the dummies.
//!
//! Subgraphs: a cluster with no edge crossing its border is laid out
//! recursively (honouring its own `direction`) and placed as one box — the
//! same trick Mermaid's dagre-wrapper uses — so isolated clusters never
//! overlap. Clusters with crossing edges stay in the flat layout, kept
//! contiguous per rank and padded. No text measurement happens here
//! (sizes come in). Callers: `mermaid::flowchart::layout` (and the
//! class/state/ER builders).

mod flat;
mod order;
mod position;

use super::Dir;
use crate::mermaid::scene::P;

/// An edge endpoint: a node, or a whole subgraph (`A --> sub1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    Node(usize),
    Cluster(usize),
}

#[derive(Debug, Clone)]
pub struct Graph {
    pub dir: Dir,
    pub node_sep: f32,
    pub rank_sep: f32,
    pub cluster_pad: f32,
    pub nodes: Vec<NodeIn>,
    pub edges: Vec<EdgeIn>,
    pub clusters: Vec<ClusterIn>,
}

impl Graph {
    pub fn new(dir: Dir) -> Graph {
        Graph {
            dir,
            node_sep: 50.0,
            rank_sep: 50.0,
            cluster_pad: 16.0,
            nodes: Vec::new(),
            edges: Vec::new(),
            clusters: Vec::new(),
        }
    }
}

/// `size` is the node's final on-screen size (width, height).
#[derive(Debug, Clone)]
pub struct NodeIn {
    pub size: [f32; 2],
    pub cluster: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct EdgeIn {
    pub from: End,
    pub to: End,
    /// Minimum rank distance (Mermaid's extra dashes: `--->` = 2).
    pub minlen: u32,
    pub weight: f32,
    /// Size of the edge label, if any.
    pub label: Option<[f32; 2]>,
}

#[derive(Debug, Clone)]
pub struct ClusterIn {
    pub parent: Option<usize>,
    /// Own `direction` (only honoured for isolated clusters, as in Mermaid).
    pub dir: Option<Dir>,
    /// Size of the title text (zero when untitled).
    pub title: [f32; 2],
}

#[derive(Debug, Clone, Default)]
pub struct EdgeOut {
    /// Polyline from the source centre (or cluster) to the target centre;
    /// callers clip the ends to the shapes. Empty = not drawn.
    pub points: Vec<P>,
    pub label: Option<P>,
    pub self_loop: bool,
    /// The source/target is a subgraph: clip to `Layout::clusters[c]`.
    pub from_cluster: Option<usize>,
    pub to_cluster: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    /// Node centres.
    pub nodes: Vec<P>,
    pub edges: Vec<EdgeOut>,
    /// `[x0, y0, x1, y1]` per cluster (`None` for empty clusters). The
    /// title band is included at the top.
    pub clusters: Vec<Option<[f32; 4]>>,
    pub width: f32,
    pub height: f32,
}

pub fn layout(g: &Graph) -> Layout {
    let mut out = layout_raw(g);
    normalize(&mut out, g);
    out
}

/// Ancestor chain (outermost first, `c` last). Cycle-safe.
fn cluster_chain(g: &Graph, c: usize) -> Vec<usize> {
    let mut chain = vec![c];
    let mut cur = g.clusters[c].parent;
    while let Some(p) = cur {
        if p >= g.clusters.len() || chain.contains(&p) {
            break;
        }
        chain.push(p);
        cur = g.clusters[p].parent;
    }
    chain.reverse();
    chain
}

type SubLayout = (Layout, Vec<usize>, Vec<usize>, Vec<usize>);

fn layout_raw(g: &Graph) -> Layout {
    let nc = g.clusters.len();
    let chains: Vec<Vec<usize>> = (0..nc).map(|c| cluster_chain(g, c)).collect();
    let node_chain: Vec<Vec<usize>> = g
        .nodes
        .iter()
        .map(|n| n.cluster.filter(|&c| c < nc).map_or_else(Vec::new, |c| chains[c].clone()))
        .collect();
    let inside = |end: End, c: usize| -> bool {
        match end {
            End::Node(n) => node_chain.get(n).is_some_and(|ch| ch.contains(&c)),
            End::Cluster(k) => k != c && k < nc && chains[k].contains(&c),
        }
    };

    // Choose the outermost isolated clusters to lay out recursively.
    let mut by_depth: Vec<usize> = (0..nc).collect();
    by_depth.sort_by_key(|&c| chains[c].len());
    let mut collapsed = vec![false; nc];
    for &c in &by_depth {
        let has_members = node_chain.iter().any(|ch| ch.contains(&c));
        let crossing = g.edges.iter().any(|e| inside(e.from, c) != inside(e.to, c));
        let ancestor_collapsed = chains[c].iter().any(|&a| a != c && collapsed[a]);
        if has_members && !crossing && !ancestor_collapsed {
            collapsed[c] = true;
        }
    }
    let collapsed_owner = |chain: &[usize]| -> Option<usize> { chain.iter().copied().find(|&c| collapsed[c]) };

    let mut subs: Vec<Option<SubLayout>> = vec![None; nc];
    for c in (0..nc).filter(|&c| collapsed[c]) {
        subs[c] = Some(sub_layout(g, c, &chains, &node_chain, &inside));
    }

    // Flat graph: free nodes + one proxy per collapsed cluster.
    let mut sizes: Vec<[f32; 2]> = Vec::new();
    let mut paths: Vec<Vec<usize>> = Vec::new();
    let mut node_flat: Vec<Option<usize>> = vec![None; g.nodes.len()];
    for (i, n) in g.nodes.iter().enumerate() {
        if collapsed_owner(&node_chain[i]).is_none() {
            node_flat[i] = Some(sizes.len());
            sizes.push(n.size);
            paths.push(node_chain[i].clone());
        }
    }
    let mut proxy: Vec<Option<usize>> = vec![None; nc];
    let mut proxy_size: Vec<[f32; 2]> = vec![[0.0; 2]; nc];
    for c in 0..nc {
        if let Some((sub, ..)) = &subs[c] {
            let title = g.clusters[c].title;
            let pad = g.cluster_pad;
            let size = [sub.width.max(title[0]) + 2.0 * pad, sub.height + 2.0 * pad + title_band(title)];
            proxy[c] = Some(sizes.len());
            proxy_size[c] = size;
            sizes.push(size);
            paths.push(chains[c][..chains[c].len() - 1].to_vec());
        }
    }

    let resolve = |end: End| -> Option<(usize, Option<usize>)> {
        match end {
            End::Node(n) => node_flat.get(n).copied().flatten().map(|f| (f, None)),
            End::Cluster(k) if k < nc => {
                if let Some(p) = proxy[k] {
                    return Some((p, Some(k)));
                }
                if collapsed_owner(&chains[k]).is_some() {
                    return None;
                }
                // Representative member for ranking; drawn clipped to the cluster.
                paths.iter().position(|p| p.contains(&k)).map(|f| (f, Some(k)))
            }
            End::Cluster(_) => None,
        }
    };

    let mut flat_edges = Vec::new();
    let mut flat_edge_orig = Vec::new();
    let mut edge_ends: Vec<(Option<usize>, Option<usize>)> = vec![(None, None); g.edges.len()];
    for (i, e) in g.edges.iter().enumerate() {
        let (Some((a, ca)), Some((b, cb))) = (resolve(e.from), resolve(e.to)) else { continue };
        edge_ends[i] = (ca, cb);
        flat_edges.push(flat::FlatEdge { from: a, to: b, minlen: e.minlen.max(1), weight: e.weight, label: e.label });
        flat_edge_orig.push(i);
    }

    let titles: Vec<[f32; 2]> = g.clusters.iter().map(|c| c.title).collect();
    let flat_out = flat::run(&flat::FlatInput {
        dir: g.dir,
        node_sep: g.node_sep,
        rank_sep: g.rank_sep,
        cluster_pad: g.cluster_pad,
        sizes: &sizes,
        paths: &paths,
        titles: &titles,
        edges: &flat_edges,
    });

    let mut out = Layout {
        nodes: vec![[0.0, 0.0]; g.nodes.len()],
        edges: vec![EdgeOut::default(); g.edges.len()],
        clusters: vec![None; nc],
        width: 0.0,
        height: 0.0,
    };
    for (i, f) in node_flat.iter().enumerate() {
        if let Some(f) = f {
            out.nodes[i] = flat_out.centers[*f];
        }
    }
    for (k, (points, label, self_loop)) in flat_out.edges.into_iter().enumerate() {
        let i = flat_edge_orig[k];
        out.edges[i] = EdgeOut { points, label, self_loop, from_cluster: edge_ends[i].0, to_cluster: edge_ends[i].1 };
    }
    for (c, rect) in flat_out.clusters {
        out.clusters[c] = Some(rect);
    }

    // Place the recursive layouts inside their proxies.
    for c in 0..nc {
        let (Some((sub, members, sub_clusters, sub_edges)), Some(p)) = (subs[c].take(), proxy[c]) else { continue };
        let center = flat_out.centers[p];
        let size = proxy_size[c];
        let pad = g.cluster_pad;
        let inner_w = size[0] - 2.0 * pad;
        let ox = center[0] - size[0] / 2.0 + pad + (inner_w - sub.width) / 2.0;
        let oy = center[1] - size[1] / 2.0 + pad + title_band(g.clusters[c].title);
        let shift = |q: P| [q[0] + ox, q[1] + oy];
        for (si, &n) in members.iter().enumerate() {
            out.nodes[n] = shift(sub.nodes[si]);
        }
        for (si, &k) in sub_clusters.iter().enumerate() {
            out.clusters[k] = sub.clusters[si].map(|r| [r[0] + ox, r[1] + oy, r[2] + ox, r[3] + oy]);
        }
        for (si, &e) in sub_edges.iter().enumerate() {
            let se = &sub.edges[si];
            out.edges[e] = EdgeOut {
                points: se.points.iter().map(|q| shift(*q)).collect(),
                label: se.label.map(shift),
                self_loop: se.self_loop,
                from_cluster: se.from_cluster.map(|k| sub_clusters[k]),
                to_cluster: se.to_cluster.map(|k| sub_clusters[k]),
            };
        }
        out.clusters[c] = Some([
            center[0] - size[0] / 2.0,
            center[1] - size[1] / 2.0,
            center[0] + size[0] / 2.0,
            center[1] + size[1] / 2.0,
        ]);
    }
    out
}

/// Height reserved above a cluster's content for its title.
pub(crate) fn title_band(title: [f32; 2]) -> f32 {
    if title[1] > 0.0 { title[1] + 4.0 } else { 0.0 }
}

/// Lays out the inside of isolated cluster `c`. Returns the layout plus
/// the sub→original index maps for nodes, clusters and edges.
fn sub_layout(
    g: &Graph,
    c: usize,
    chains: &[Vec<usize>],
    node_chain: &[Vec<usize>],
    inside: &dyn Fn(End, usize) -> bool,
) -> SubLayout {
    let members: Vec<usize> = (0..g.nodes.len()).filter(|&n| node_chain[n].contains(&c)).collect();
    let sub_clusters: Vec<usize> = (0..g.clusters.len()).filter(|&k| k != c && chains[k].contains(&c)).collect();
    let cmap = |k: usize| sub_clusters.iter().position(|&x| x == k);
    let nmap = |n: usize| members.iter().position(|&x| x == n);
    let map_end = |e: End| -> Option<End> {
        match e {
            End::Node(n) => nmap(n).map(End::Node),
            End::Cluster(k) => cmap(k).map(End::Cluster),
        }
    };
    let mut sub_edges = Vec::new();
    let mut edges = Vec::new();
    for (i, e) in g.edges.iter().enumerate() {
        if !(inside(e.from, c) && inside(e.to, c)) {
            continue;
        }
        if let (Some(from), Some(to)) = (map_end(e.from), map_end(e.to)) {
            edges.push(EdgeIn { from, to, ..e.clone() });
            sub_edges.push(i);
        }
    }
    let sub = Graph {
        dir: g.clusters[c].dir.unwrap_or(g.dir),
        node_sep: g.node_sep,
        rank_sep: g.rank_sep,
        cluster_pad: g.cluster_pad,
        nodes: members
            .iter()
            .map(|&n| NodeIn { size: g.nodes[n].size, cluster: g.nodes[n].cluster.filter(|&k| k != c).and_then(cmap) })
            .collect(),
        edges,
        clusters: sub_clusters
            .iter()
            .map(|&k| ClusterIn {
                parent: g.clusters[k].parent.filter(|&p| p != c).and_then(cmap),
                dir: g.clusters[k].dir,
                title: g.clusters[k].title,
            })
            .collect(),
    };
    (layout(&sub), members, sub_clusters, sub_edges)
}

/// Translate so everything starts at (0, 0) and set width/height.
fn normalize(out: &mut Layout, g: &Graph) {
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    let mut grow = |r: [f32; 4]| {
        b[0] = b[0].min(r[0]);
        b[1] = b[1].min(r[1]);
        b[2] = b[2].max(r[2]);
        b[3] = b[3].max(r[3]);
    };
    for (i, c) in out.nodes.iter().enumerate() {
        let s = g.nodes[i].size;
        grow([c[0] - s[0] / 2.0, c[1] - s[1] / 2.0, c[0] + s[0] / 2.0, c[1] + s[1] / 2.0]);
    }
    for (i, e) in out.edges.iter().enumerate() {
        for p in &e.points {
            grow([p[0], p[1], p[0], p[1]]);
        }
        if let (Some(l), Some(size)) = (e.label, g.edges[i].label) {
            grow([l[0] - size[0] / 2.0, l[1] - size[1] / 2.0, l[0] + size[0] / 2.0, l[1] + size[1] / 2.0]);
        }
    }
    for r in out.clusters.iter().flatten() {
        grow(*r);
    }
    if b[0] > b[2] {
        return;
    }
    let (dx, dy) = (-b[0], -b[1]);
    for c in &mut out.nodes {
        c[0] += dx;
        c[1] += dy;
    }
    for e in &mut out.edges {
        for p in &mut e.points {
            p[0] += dx;
            p[1] += dy;
        }
        if let Some(l) = &mut e.label {
            l[0] += dx;
            l[1] += dy;
        }
    }
    for r in out.clusters.iter_mut().flatten() {
        r[0] += dx;
        r[2] += dx;
        r[1] += dy;
        r[3] += dy;
    }
    out.width = b[2] - b[0];
    out.height = b[3] - b[1];
}

#[cfg(test)]
mod tests;
