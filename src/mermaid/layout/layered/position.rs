//! Coordinate assignment across ranks for the layered layout. Each pass
//! moves every node of a layer towards the weighted mean of its neighbours
//! in the adjacent layer, solving the layer exactly: minimise
//! Σ wᵢ(xᵢ − dᵢ)² subject to xᵢ₊₁ − xᵢ ≥ sepᵢ — an isotonic regression,
//! which pool-adjacent-violators solves in O(n). Heavier weights on dummy
//! chains keep long edges straight. Callers: `layered::flat::run`.

use super::title_band;

pub(super) struct SepParams<'a> {
    pub widths: &'a [f32],
    pub real: &'a [bool],
    pub paths: &'a [&'a [usize]],
    pub node_sep: f32,
    pub edge_sep: f32,
    pub cluster_pad: f32,
    pub titles: &'a [[f32; 2]],
    /// Titles sit across the flow (LR/RL): entering a cluster needs room.
    pub title_cross: bool,
    /// Per node: (cluster, side) for cluster border nodes (side -1 left,
    /// 1 right), side 0 otherwise.
    pub border: &'a [(usize, i8)],
}

/// Minimum centre distance between neighbours `a` (left) and `b`.
fn sep(p: &SepParams<'_>, a: usize, b: usize) -> f32 {
    let (pa, pb) = (p.paths[a], p.paths[b]);
    let (ba, bb) = (p.border[a].1 != 0, p.border[b].1 != 0);
    let nested = pa.starts_with(pb) || pb.starts_with(pa);
    // Borders sit on their cluster's content edge; the gap to anything
    // outside the cluster comes from padding plus half a node gap.
    let base = if ba && bb {
        if nested { 0.0 } else { p.node_sep / 2.0 }
    } else if ba || bb {
        if pa == pb { 0.0 } else { p.node_sep / 2.0 }
    } else {
        match (p.real[a], p.real[b]) {
            (true, true) => p.node_sep,
            (false, false) => p.edge_sep,
            _ => (p.node_sep + p.edge_sep) / 2.0,
        }
    };
    let mut extra = 0.0;
    for c in pa {
        if !pb.contains(c) {
            extra += p.cluster_pad;
        }
    }
    for c in pb {
        if !pa.contains(c) {
            extra += p.cluster_pad;
            if p.title_cross {
                extra += title_band(p.titles.get(*c).copied().unwrap_or([0.0; 2]));
            }
        }
    }
    (p.widths[a] + p.widths[b]) / 2.0 + base + extra
}

const PASSES: usize = 8;

pub(super) fn assign_x(
    layers: &[Vec<usize>],
    p: &SepParams<'_>,
    up: &[Vec<(usize, f32)>],
    down: &[Vec<(usize, f32)>],
) -> Vec<f32> {
    let mut x = vec![0.0f32; p.widths.len()];
    for layer in layers {
        let mut cur = 0.0;
        for (i, &v) in layer.iter().enumerate() {
            if i > 0 {
                cur += sep(p, layer[i - 1], v);
            }
            x[v] = cur;
        }
        for &v in layer {
            x[v] -= cur / 2.0;
        }
    }
    let n = layers.len();
    for it in 0..PASSES {
        let ext = extents(p, &x, false);
        if it % 2 == 0 {
            for layer in layers.iter().skip(1) {
                align(layer, p, &mut x, &[up], &ext);
            }
        } else {
            for layer in layers[..n.saturating_sub(1)].iter().rev() {
                align(layer, p, &mut x, &[down], &ext);
            }
        }
    }
    for _ in 0..3 {
        let ext = extents(p, &x, true);
        for layer in layers {
            align(layer, p, &mut x, &[up, down], &ext);
        }
    }
    x
}

/// Per cluster, where its left/right borders should line up: the mean
/// border position while settling, the outermost one at the end (which
/// makes every rank's border meet the cluster's bounding box).
fn extents(p: &SepParams<'_>, x: &[f32], outermost: bool) -> std::collections::HashMap<usize, (f32, f32)> {
    let mut acc: std::collections::HashMap<usize, (f32, f32, f32, f32, f32)> = std::collections::HashMap::new();
    for (v, &(c, side)) in p.border.iter().enumerate() {
        if side == 0 {
            continue;
        }
        let e = acc.entry(c).or_insert((f32::MAX, f32::MIN, 0.0, 0.0, 0.0));
        if side < 0 {
            e.0 = e.0.min(x[v]);
            e.2 += x[v];
            e.4 += 1.0;
        } else {
            e.1 = e.1.max(x[v]);
            e.3 += x[v];
        }
    }
    acc.into_iter()
        .map(|(c, (lo, hi, sl, sr, n))| (c, if outermost { (lo, hi) } else { (sl / n, sr / n) }))
        .collect()
}

/// Border nodes are pulled hard towards their cluster's line.
const BORDER_WEIGHT: f32 = 1000.0;

fn align(
    layer: &[usize],
    p: &SepParams<'_>,
    x: &mut [f32],
    nbr_sets: &[&[Vec<(usize, f32)>]],
    ext: &std::collections::HashMap<usize, (f32, f32)>,
) {
    if layer.is_empty() {
        return;
    }
    let mut targets = Vec::with_capacity(layer.len());
    let mut weights = Vec::with_capacity(layer.len());
    for &v in layer {
        let (c, side) = p.border[v];
        if side != 0
            && let Some(&(lo, hi)) = ext.get(&c)
        {
            targets.push(if side < 0 { lo } else { hi });
            weights.push(BORDER_WEIGHT);
            continue;
        }
        let (mut s, mut w) = (0.0, 0.0);
        for set in nbr_sets {
            for &(u, wt) in &set[v] {
                s += x[u] * wt;
                w += wt;
            }
        }
        if w > 0.0 {
            targets.push(s / w);
            weights.push(w);
        } else {
            targets.push(x[v]);
            weights.push(0.5);
        }
    }
    let seps: Vec<f32> = layer.windows(2).map(|w| sep(p, w[0], w[1])).collect();
    let solved = pava(&targets, &weights, &seps);
    for (i, &v) in layer.iter().enumerate() {
        x[v] = solved[i];
    }
}

/// Weighted least squares to `targets` with `x[i+1] - x[i] >= seps[i]`.
pub(super) fn pava(targets: &[f32], weights: &[f32], seps: &[f32]) -> Vec<f32> {
    let n = targets.len();
    let mut offs = vec![0.0f32; n];
    for i in 1..n {
        offs[i] = offs[i - 1] + seps[i - 1];
    }
    // Blocks of (Σ w·t, Σ w, count).
    let mut blocks: Vec<(f32, f32, usize)> = Vec::with_capacity(n);
    for i in 0..n {
        let w = weights[i].max(1e-6);
        blocks.push((w * (targets[i] - offs[i]), w, 1));
        while blocks.len() >= 2 {
            let (s2, w2, c2) = blocks[blocks.len() - 1];
            let (s1, w1, c1) = blocks[blocks.len() - 2];
            if s1 / w1 <= s2 / w2 {
                break;
            }
            blocks.pop();
            let last = blocks.len() - 1;
            blocks[last] = (s1 + s2, w1 + w2, c1 + c2);
        }
    }
    let mut out = Vec::with_capacity(n);
    for (s, w, c) in blocks {
        out.extend(std::iter::repeat_n(s / w, c));
    }
    for (o, off) in out.iter_mut().zip(&offs) {
        *o += off;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::pava;

    #[test]
    fn pava_respects_separation_and_targets() {
        // Two nodes wanting the same spot get split symmetrically.
        let x = pava(&[0.0, 0.0], &[1.0, 1.0], &[10.0]);
        assert!((x[0] + 5.0).abs() < 1e-4 && (x[1] - 5.0).abs() < 1e-4);
        // Already feasible targets are kept.
        let x = pava(&[0.0, 50.0], &[1.0, 1.0], &[10.0]);
        assert_eq!(x, vec![0.0, 50.0]);
        // Heavier node moves less.
        let x = pava(&[0.0, 0.0], &[9.0, 1.0], &[10.0]);
        assert!(x[0].abs() < 1.5 && x[1] - x[0] >= 10.0 - 1e-4);
    }
}
