//! Layered layout of a flat graph (isolated clusters already collapsed
//! into proxy nodes by `layered::layout`). Works internally top-to-bottom
//! and transforms to the requested direction at the end. See the parent
//! module for the pipeline. Callers: `layered::layout_raw`.

use std::collections::HashMap;

use super::{order, position, title_band};
use crate::mermaid::layout::Dir;
use crate::mermaid::scene::P;

pub(super) struct FlatEdge {
    pub from: usize,
    pub to: usize,
    pub minlen: u32,
    pub weight: f32,
    pub label: Option<[f32; 2]>,
}

pub(super) struct FlatInput<'a> {
    pub dir: Dir,
    pub node_sep: f32,
    pub rank_sep: f32,
    pub cluster_pad: f32,
    /// Final-orientation node sizes.
    pub sizes: &'a [[f32; 2]],
    /// Cluster chain per node (outermost first), non-collapsed clusters only.
    pub paths: &'a [Vec<usize>],
    /// Title size per cluster id.
    pub titles: &'a [[f32; 2]],
    pub edges: &'a [FlatEdge],
}

/// Per input edge: polyline, label centre, is-self-loop.
pub(super) type FlatEdgeOut = (Vec<P>, Option<P>, bool);

pub(super) struct FlatOutput {
    pub centers: Vec<P>,
    pub edges: Vec<FlatEdgeOut>,
    pub clusters: Vec<(usize, [f32; 4])>,
}

/// Room reserved beside a node for its self-loop.
const LOOP_REACH: f32 = 26.0;
/// Separation between neighbouring dummy nodes (parallel edge strands).
const EDGE_SEP: f32 = 12.0;

/// A node of the layered graph (real or dummy), internal TB orientation:
/// `size[0]` across ranks, `size[1]` along them.
pub(super) struct LNode {
    pub size: [f32; 2],
    pub path: Vec<usize>,
    pub real: bool,
    pub rank: usize,
}

pub(super) fn run(inp: &FlatInput<'_>) -> FlatOutput {
    let n = inp.sizes.len();
    let horiz = inp.dir.is_horizontal();
    let swap = |s: [f32; 2]| if horiz { [s[1], s[0]] } else { s };
    let valid = |e: &FlatEdge| e.from < n && e.to < n;

    let mut has_loop = vec![false; n];
    for e in inp.edges.iter().filter(|e| valid(e) && e.from == e.to) {
        has_loop[e.from] = true;
    }
    let mut lnodes: Vec<LNode> = (0..n)
        .map(|i| {
            let mut size = swap(inp.sizes[i]);
            if has_loop[i] {
                size[0] += 2.0 * LOOP_REACH;
            }
            LNode { size, path: inp.paths[i].clone(), real: true, rank: 0 }
        })
        .collect();

    // 1. Break cycles.
    let active: Vec<usize> = (0..inp.edges.len())
        .filter(|&i| valid(&inp.edges[i]) && inp.edges[i].from != inp.edges[i].to)
        .collect();
    let mut out_adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &i in &active {
        out_adj[inp.edges[i].from].push(i);
    }
    let reversed = back_edges(n, &out_adj, inp.edges);
    let oriented = |i: usize| {
        let e = &inp.edges[i];
        if reversed[i] { (e.to, e.from) } else { (e.from, e.to) }
    };

    // 2. Ranks (minlen doubled so every edge gets a label slot).
    let rank = rank_nodes(n, &active, &oriented, inp.edges);
    for (i, r) in rank.iter().enumerate() {
        lnodes[i].rank = *r;
    }

    // 3. Split edges into chains of dummies.
    let mut chains: Vec<Vec<usize>> = vec![Vec::new(); inp.edges.len()];
    let mut label_node: Vec<Option<usize>> = vec![None; inp.edges.len()];
    for &i in &active {
        let (a, b) = oriented(i);
        let (ra, rb) = (rank[a], rank[b]);
        let path = common_prefix(&inp.paths[a], &inp.paths[b]);
        let mid = ra + (rb - ra) / 2;
        let mut chain = vec![a];
        for r in ra + 1..rb {
            let label = if r == mid { inp.edges[i].label } else { None };
            if label.is_some() {
                label_node[i] = Some(lnodes.len());
            }
            chain.push(lnodes.len());
            lnodes.push(LNode { size: label.map_or([0.0, 0.0], swap), path: path.clone(), real: false, rank: r });
        }
        chain.push(b);
        chains[i] = chain;
    }
    // 3b. Border nodes: every cluster gets a left and right border node on
    // each rank it spans, chained vertically, so ordering keeps the
    // cluster's column free of outsiders on every rank (dagre's approach).
    let mut border: Vec<(usize, i8)> = vec![(0, 0); lnodes.len()];
    let mut border_chain: Vec<(usize, usize)> = Vec::new();
    let mut span: HashMap<usize, (usize, usize, Vec<usize>)> = HashMap::new();
    for l in &lnodes {
        for (k, &c) in l.path.iter().enumerate() {
            let e = span.entry(c).or_insert_with(|| (l.rank, l.rank, l.path[..=k].to_vec()));
            e.0 = e.0.min(l.rank);
            e.1 = e.1.max(l.rank);
        }
    }
    let mut ids: Vec<usize> = span.keys().copied().collect();
    ids.sort_unstable();
    for c in ids {
        let (top, bottom, chain) = span[&c].clone();
        let mut prev: Option<(usize, usize)> = None;
        for r in top..=bottom {
            let left = lnodes.len();
            lnodes.push(LNode { size: [0.0, 0.0], path: chain.clone(), real: false, rank: r });
            border.push((c, -1));
            lnodes.push(LNode { size: [0.0, 0.0], path: chain.clone(), real: false, rank: r });
            border.push((c, 1));
            if let Some((pl, pr)) = prev {
                border_chain.push((pl, left));
                border_chain.push((pr, left + 1));
            }
            prev = Some((left, left + 1));
        }
    }

    let total = lnodes.len();
    let mut up: Vec<Vec<(usize, f32)>> = vec![Vec::new(); total];
    let mut down: Vec<Vec<(usize, f32)>> = vec![Vec::new(); total];
    for &(u, v) in &border_chain {
        down[u].push((v, 8.0));
        up[v].push((u, 8.0));
    }
    for &i in &active {
        let w = inp.edges[i].weight.max(0.01);
        for pair in chains[i].windows(2) {
            let (u, v) = (pair[0], pair[1]);
            // Straight long edges matter more than straight short ones.
            let k = match (lnodes[u].real, lnodes[v].real) {
                (false, false) => 8.0,
                (true, true) => 1.0,
                _ => 2.0,
            };
            down[u].push((v, w * k));
            up[v].push((u, w * k));
        }
    }

    // 4. Order within ranks.
    let max_rank = lnodes.iter().map(|l| l.rank).max().unwrap_or(0);
    let paths: Vec<&[usize]> = lnodes.iter().map(|l| l.path.as_slice()).collect();
    let sides: Vec<i8> = border.iter().map(|b| b.1).collect();
    let mut layers = order::init_order(&lnodes, &down, max_rank);
    order::minimize(&mut layers, &up, &down, &paths, &sides);

    // 5. Coordinates across ranks.
    let widths: Vec<f32> = lnodes.iter().map(|l| l.size[0]).collect();
    let real: Vec<bool> = lnodes.iter().map(|l| l.real).collect();
    let params = position::SepParams {
        widths: &widths,
        real: &real,
        paths: &paths,
        node_sep: inp.node_sep,
        edge_sep: EDGE_SEP,
        cluster_pad: inp.cluster_pad,
        titles: inp.titles,
        title_cross: horiz,
        border: &border,
    };
    let x = position::assign_x(&layers, &params, &up, &down);

    // 6. Coordinates along ranks.
    let y = assign_y(inp, &lnodes, &layers, n);

    // 7. Back to the requested direction.
    let tf = |p: P| -> P {
        match inp.dir {
            Dir::TB => p,
            Dir::BT => [p[0], -p[1]],
            Dir::LR => [p[1], p[0]],
            Dir::RL => [-p[1], p[0]],
        }
    };
    let centers: Vec<P> = (0..n).map(|i| tf([x[i], y[i]])).collect();
    let edges = (0..inp.edges.len())
        .map(|i| {
            let e = &inp.edges[i];
            if valid(e) && e.from == e.to {
                let (pts, label) = loop_points(centers[e.from], inp.sizes[e.from], horiz, e.label);
                return (pts, label, true);
            }
            if chains[i].is_empty() {
                return (Vec::new(), None, false);
            }
            let mut pts: Vec<P> = chains[i].iter().map(|&v| tf([x[v], y[v]])).collect();
            if reversed[i] {
                pts.reverse();
            }
            (pts, label_node[i].map(|v| tf([x[v], y[v]])), false)
        })
        .collect();

    let node_rect = |v: usize| -> [f32; 4] {
        let s = lnodes[v].size;
        let a = tf([x[v] - s[0] / 2.0, y[v] - s[1] / 2.0]);
        let b = tf([x[v] + s[0] / 2.0, y[v] + s[1] / 2.0]);
        [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])]
    };
    let clusters = cluster_rects(inp, &lnodes, &node_rect);
    FlatOutput { centers, edges, clusters }
}

/// DFS; edges into a node still on the stack are back edges.
fn back_edges(n: usize, out_adj: &[Vec<usize>], edges: &[FlatEdge]) -> Vec<bool> {
    let mut state = vec![0u8; n];
    let mut rev = vec![false; edges.len()];
    for s in 0..n {
        if state[s] != 0 {
            continue;
        }
        state[s] = 1;
        let mut stack: Vec<(usize, usize)> = vec![(s, 0)];
        while let Some(top) = stack.last_mut() {
            let v = top.0;
            if top.1 < out_adj[v].len() {
                let e = out_adj[v][top.1];
                top.1 += 1;
                let w = edges[e].to;
                match state[w] {
                    0 => {
                        state[w] = 1;
                        stack.push((w, 0));
                    }
                    1 => rev[e] = true,
                    _ => {}
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }
    rev
}

/// Longest-path ranking, then each node is pulled towards its successors
/// when that shortens more edge weight than it lengthens.
fn rank_nodes(n: usize, active: &[usize], oriented: &dyn Fn(usize) -> (usize, usize), edges: &[FlatEdge]) -> Vec<usize> {
    let minlen = |i: usize| 2 * edges[i].minlen.max(1) as i64;
    let mut outs: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut ins: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &i in active {
        let (a, b) = oriented(i);
        outs[a].push(i);
        ins[b].push(i);
    }
    let mut indeg: Vec<usize> = ins.iter().map(Vec::len).collect();
    let mut queue: std::collections::VecDeque<usize> = (0..n).filter(|&v| indeg[v] == 0).collect();
    let mut topo = Vec::with_capacity(n);
    let mut rank = vec![0i64; n];
    while let Some(u) = queue.pop_front() {
        topo.push(u);
        for &i in &outs[u] {
            let (_, v) = oriented(i);
            rank[v] = rank[v].max(rank[u] + minlen(i));
            indeg[v] -= 1;
            if indeg[v] == 0 {
                queue.push_back(v);
            }
        }
    }
    for &u in topo.iter().rev() {
        if outs[u].is_empty() {
            continue;
        }
        let hi = outs[u].iter().map(|&i| rank[oriented(i).1] - minlen(i)).min().unwrap_or(rank[u]);
        let in_w: f32 = ins[u].iter().map(|&i| edges[i].weight).sum();
        let out_w: f32 = outs[u].iter().map(|&i| edges[i].weight).sum();
        if hi > rank[u] && (ins[u].is_empty() || out_w > in_w) {
            rank[u] = hi;
        }
    }
    let min = rank.iter().copied().min().unwrap_or(0);
    rank.iter().map(|r| (r - min) as usize).collect()
}

fn common_prefix(a: &[usize], b: &[usize]) -> Vec<usize> {
    a.iter().zip(b).take_while(|(x, y)| x == y).map(|(x, _)| *x).collect()
}

/// Rank centres along the flow, leaving room for cluster padding and (for
/// vertical flows) cluster titles above/below the ranks where they start/end.
fn assign_y(inp: &FlatInput<'_>, lnodes: &[LNode], layers: &[Vec<usize>], n: usize) -> Vec<f32> {
    let ranks = layers.len();
    let thick: Vec<f32> = layers
        .iter()
        .map(|layer| layer.iter().map(|&v| lnodes[v].size[1]).fold(0.0, f32::max))
        .collect();
    let mut span: HashMap<usize, (usize, usize)> = HashMap::new();
    for l in lnodes.iter().take(n) {
        for &c in &l.path {
            let e = span.entry(c).or_insert((l.rank, l.rank));
            e.0 = e.0.min(l.rank);
            e.1 = e.1.max(l.rank);
        }
    }
    let title_top = inp.dir == Dir::TB;
    let title_bottom = inp.dir == Dir::BT;
    let mut before = vec![0.0f32; ranks];
    let mut after = vec![0.0f32; ranks];
    for l in lnodes.iter().take(n) {
        let (mut sb, mut sa) = (0.0, 0.0);
        for &c in &l.path {
            let (top, bottom) = span[&c];
            let band = title_band(inp.titles.get(c).copied().unwrap_or([0.0; 2]));
            if top == l.rank {
                sb += inp.cluster_pad + if title_top { band } else { 0.0 };
            }
            if bottom == l.rank {
                sa += inp.cluster_pad + if title_bottom { band } else { 0.0 };
            }
        }
        before[l.rank] = before[l.rank].max(sb);
        after[l.rank] = after[l.rank].max(sa);
    }
    let half = inp.rank_sep / 2.0;
    let mut centers = vec![0.0f32; ranks];
    let mut y = 0.0;
    for r in 0..ranks {
        y += before[r];
        centers[r] = y + thick[r] / 2.0;
        y += thick[r] + after[r] + half;
    }
    lnodes.iter().map(|l| centers[l.rank]).collect()
}

/// Final-orientation cluster boxes: members (nodes, proxies, dummies) and
/// child clusters, padded, with the title band on top.
fn cluster_rects(inp: &FlatInput<'_>, lnodes: &[LNode], node_rect: &dyn Fn(usize) -> [f32; 4]) -> Vec<(usize, [f32; 4])> {
    let mut parent: HashMap<usize, Option<usize>> = HashMap::new();
    let mut depth: HashMap<usize, usize> = HashMap::new();
    for l in lnodes {
        for (k, &c) in l.path.iter().enumerate() {
            parent.insert(c, if k > 0 { Some(l.path[k - 1]) } else { None });
            depth.insert(c, k);
        }
    }
    let mut ids: Vec<usize> = depth.keys().copied().collect();
    ids.sort_by_key(|c| (std::cmp::Reverse(depth[c]), *c));
    let mut rects: HashMap<usize, [f32; 4]> = HashMap::new();
    let pad = inp.cluster_pad;
    for c in ids {
        let mut b: Option<[f32; 4]> = None;
        let mut grow = |r: [f32; 4]| {
            b = Some(match b {
                None => r,
                Some(o) => [o[0].min(r[0]), o[1].min(r[1]), o[2].max(r[2]), o[3].max(r[3])],
            });
        };
        for (v, l) in lnodes.iter().enumerate() {
            if l.path.contains(&c) {
                grow(node_rect(v));
            }
        }
        for (&d, &p) in &parent {
            if p == Some(c)
                && let Some(r) = rects.get(&d)
            {
                grow(*r);
            }
        }
        let Some(mut r) = b else { continue };
        let title = inp.titles.get(c).copied().unwrap_or([0.0; 2]);
        r = [r[0] - pad, r[1] - pad - title_band(title), r[2] + pad, r[3] + pad];
        let need = title[0] + 2.0 * pad - (r[2] - r[0]);
        if need > 0.0 {
            r[0] -= need / 2.0;
            r[2] += need / 2.0;
        }
        rects.insert(c, r);
    }
    let mut out: Vec<(usize, [f32; 4])> = rects.into_iter().collect();
    out.sort_by_key(|(c, _)| *c);
    out
}

/// A loop beside the node, on the side across the flow.
fn loop_points(c: P, s: [f32; 2], horiz: bool, label: Option<[f32; 2]>) -> (Vec<P>, Option<P>) {
    let (hw, hh) = (s[0] / 2.0, s[1] / 2.0);
    let r = LOOP_REACH - 4.0;
    let l = label.unwrap_or([0.0, 0.0]);
    if !horiz {
        let x = c[0] + hw;
        let pts = vec![
            [x, c[1] - hh * 0.5],
            [x + r, c[1] - hh * 0.5 - 4.0],
            [x + r, c[1] + hh * 0.5 + 4.0],
            [x, c[1] + hh * 0.5],
        ];
        (pts, label.map(|_| [x + r + l[0] / 2.0 + 4.0, c[1]]))
    } else {
        let y = c[1] + hh;
        let pts = vec![
            [c[0] - hw * 0.5, y],
            [c[0] - hw * 0.5 - 4.0, y + r],
            [c[0] + hw * 0.5 + 4.0, y + r],
            [c[0] + hw * 0.5, y],
        ];
        (pts, label.map(|_| [c[0], y + r + l[1] / 2.0 + 4.0]))
    }
}
