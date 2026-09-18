//! Crossing minimisation for the layered layout: DFS initial order, then
//! alternating barycenter sweeps. Sorting is cluster-aware — members of a
//! subgraph stay contiguous in every rank (recursively for nested
//! subgraphs), which is what keeps cluster boxes from interleaving. The
//! best order seen (by exact crossing count, Fenwick-tree bilayer counting)
//! wins. Callers: `layered::flat::run`.

use super::flat::LNode;

pub(super) fn init_order(lnodes: &[LNode], down: &[Vec<(usize, f32)>], max_rank: usize) -> Vec<Vec<usize>> {
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); max_rank + 1];
    let mut visited = vec![false; lnodes.len()];
    let mut starts: Vec<usize> = (0..lnodes.len()).collect();
    starts.sort_by_key(|&v| lnodes[v].rank);
    for s in starts {
        let mut stack = vec![s];
        while let Some(v) = stack.pop() {
            if visited[v] {
                continue;
            }
            visited[v] = true;
            layers[lnodes[v].rank].push(v);
            for &(w, _) in down[v].iter().rev() {
                if !visited[w] {
                    stack.push(w);
                }
            }
        }
    }
    layers
}

const SWEEPS: usize = 24;

pub(super) fn minimize(
    layers: &mut Vec<Vec<usize>>,
    up: &[Vec<(usize, f32)>],
    down: &[Vec<(usize, f32)>],
    paths: &[&[usize]],
    sides: &[i8],
) {
    let ctx = Ctx { paths, sides };
    let mut pos = vec![0usize; up.len()];
    // Group clusters once up front so even a crossing-free input is contiguous.
    for layer in layers.iter_mut() {
        let items: Vec<(usize, f32)> = layer.iter().enumerate().map(|(i, &v)| (v, i as f32)).collect();
        *layer = arrange(&items, 0, &ctx);
    }
    set_positions(layers, &mut pos);
    let mut best = layers.clone();
    let mut best_cc = crossings(layers, down, &pos);
    let n = layers.len();
    for iter in 0..SWEEPS {
        if best_cc == 0 {
            break;
        }
        if iter % 2 == 0 {
            for l in 1..n {
                let adj = layers[l - 1].len();
                sort_layer(&mut layers[l], adj, &mut pos, up, &ctx);
            }
        } else {
            for l in (0..n.saturating_sub(1)).rev() {
                let adj = layers[l + 1].len();
                sort_layer(&mut layers[l], adj, &mut pos, down, &ctx);
            }
        }
        let cc = crossings(layers, down, &pos);
        if cc < best_cc {
            best_cc = cc;
            best = layers.clone();
        }
    }
    *layers = best;
}

fn set_positions(layers: &[Vec<usize>], pos: &mut [usize]) {
    for layer in layers {
        for (i, &v) in layer.iter().enumerate() {
            pos[v] = i;
        }
    }
}

fn sort_layer(layer: &mut Vec<usize>, adj_len: usize, pos: &mut [usize], nbrs: &[Vec<(usize, f32)>], ctx: &Ctx<'_>) {
    let len = layer.len();
    if len < 2 {
        return;
    }
    // Nodes without neighbours keep their relative slot, scaled to the
    // adjacent layer's index range.
    let scale = adj_len.saturating_sub(1) as f32 / (len - 1) as f32;
    let items: Vec<(usize, f32)> = layer
        .iter()
        .map(|&v| {
            let (s, w) = nbrs[v].iter().fold((0.0, 0.0), |(s, w), &(u, wt)| (s + pos[u] as f32 * wt, w + wt));
            (v, if w > 0.0 { s / w } else { pos[v] as f32 * scale })
        })
        .collect();
    *layer = arrange(&items, 0, ctx);
    for (i, &v) in layer.iter().enumerate() {
        pos[v] = i;
    }
}

/// Cluster path per node, and border side (-1 left, 1 right, 0 none).
struct Ctx<'a> {
    paths: &'a [&'a [usize]],
    sides: &'a [i8],
}

enum Entry {
    Node(usize),
    Group(Vec<(usize, f32)>),
}

/// Stable sort by barycenter where every cluster at `level` of the path
/// moves as one block (its barycenter = mean of its members'), recursing
/// into each block; a cluster's border nodes stay at its two ends.
fn arrange(items: &[(usize, f32)], level: usize, ctx: &Ctx<'_>) -> Vec<usize> {
    let paths = ctx.paths;
    let mut entries: Vec<(f32, Entry)> = Vec::new();
    let mut group_of: Vec<(usize, usize)> = Vec::new();
    for &(v, b) in items {
        if let Some(&c) = paths[v].get(level) {
            if let Some(&(_, idx)) = group_of.iter().find(|(g, _)| *g == c) {
                if let Entry::Group(members) = &mut entries[idx].1 {
                    members.push((v, b));
                }
            } else {
                group_of.push((c, entries.len()));
                entries.push((0.0, Entry::Group(vec![(v, b)])));
            }
        } else {
            let key = match ctx.sides[v] {
                -1 => f32::NEG_INFINITY,
                1 => f32::INFINITY,
                _ => b,
            };
            entries.push((key, Entry::Node(v)));
        }
    }
    for (key, entry) in &mut entries {
        if let Entry::Group(members) = entry {
            *key = members.iter().map(|(_, b)| b).sum::<f32>() / members.len() as f32;
        }
    }
    entries.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = Vec::with_capacity(items.len());
    for (_, entry) in entries {
        match entry {
            Entry::Node(v) => out.push(v),
            Entry::Group(members) => out.extend(arrange(&members, level + 1, ctx)),
        }
    }
    out
}

/// Total edge crossings between consecutive layers.
pub(super) fn crossings(layers: &[Vec<usize>], down: &[Vec<(usize, f32)>], pos: &[usize]) -> usize {
    let mut total = 0;
    for l in 0..layers.len().saturating_sub(1) {
        let mut pairs: Vec<(usize, usize)> = Vec::new();
        for &u in &layers[l] {
            for &(v, _) in &down[u] {
                pairs.push((pos[u], pos[v]));
            }
        }
        pairs.sort_unstable();
        let size = layers[l + 1].len();
        let mut tree = vec![0usize; size + 1];
        for (inserted, (_, pv)) in pairs.into_iter().enumerate() {
            // Already-inserted endpoints at or left of pv (Fenwick prefix sum).
            let mut i = pv + 1;
            let mut le = 0;
            while i > 0 {
                le += tree[i];
                i &= i - 1;
            }
            total += inserted - le;
            let mut i = pv + 1;
            while i <= size {
                tree[i] += 1;
                i += i & i.wrapping_neg();
            }
        }
    }
    total
}
