//! Edge routing shared by the graph diagrams (§3.7.3): turn a layered
//! layout polyline (centre → dummies → centre) into the drawn curve —
//! ends clipped to node outlines, or cut where the line crosses a
//! subgraph's box — then smoothed with the configured curve. Callers:
//! `flowchart::build`, `state`, `class`, `er`.

use super::layout::layered::{EdgeOut, Layout};
use super::scene::{ClipShape, P, basis_curve, clip_to_box};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    Basis,
    Linear,
    Step,
}

impl Curve {
    /// Mermaid `curve` config names; unknown ones fall back to `basis`.
    pub fn from_name(name: Option<&str>) -> Curve {
        match name {
            Some("linear") => Curve::Linear,
            Some("step" | "stepBefore" | "stepAfter") => Curve::Step,
            _ => Curve::Basis,
        }
    }
}

/// Where an edge end attaches: centre, half-size, outline.
pub type EndGeom = (P, P, ClipShape);

/// Final drawable points of a laid-out edge; `None` when not drawable.
pub fn route(out: &EdgeOut, lay: &Layout, from: EndGeom, to: EndGeom, curve: Curve) -> Option<Vec<P>> {
    if out.points.len() < 2 {
        return None;
    }
    let mut pts = out.points.clone();
    if !out.self_loop {
        match out.from_cluster.and_then(|c| lay.clusters.get(c).copied().flatten()) {
            Some(r) => cut_start(&mut pts, r),
            None => pts[0] = clip_to_box(from.0, from.1, pts[1], from.2),
        }
        match out.to_cluster.and_then(|c| lay.clusters.get(c).copied().flatten()) {
            Some(r) => {
                pts.reverse();
                cut_start(&mut pts, r);
                pts.reverse();
            }
            None => {
                let n = pts.len();
                pts[n - 1] = clip_to_box(to.0, to.1, pts[n - 2], to.2);
            }
        }
    }
    Some(match curve {
        Curve::Linear => pts,
        Curve::Step => orthogonal(&pts),
        Curve::Basis => basis_curve(&pts, 10),
    })
}

/// Drop the part of a polyline that starts inside `r` (x0, y0, x1, y1),
/// so it begins exactly where it leaves the box.
pub fn cut_start(pts: &mut Vec<P>, r: [f32; 4]) {
    let inside = |p: P| p[0] > r[0] && p[0] < r[2] && p[1] > r[1] && p[1] < r[3];
    if pts.len() < 2 || !inside(pts[0]) {
        return;
    }
    for i in 0..pts.len() - 1 {
        let (a, b) = (pts[i], pts[i + 1]);
        if inside(a) && !inside(b) {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let mut t = 1.0f32;
            if dx > 0.0 {
                t = t.min((r[2] - a[0]) / dx);
            } else if dx < 0.0 {
                t = t.min((r[0] - a[0]) / dx);
            }
            if dy > 0.0 {
                t = t.min((r[3] - a[1]) / dy);
            } else if dy < 0.0 {
                t = t.min((r[1] - a[1]) / dy);
            }
            pts.drain(..=i);
            pts.insert(0, [a[0] + dx * t, a[1] + dy * t]);
            return;
        }
    }
}

/// Horizontal/vertical elbows between consecutive points.
pub fn orthogonal(pts: &[P]) -> Vec<P> {
    let mut out = Vec::with_capacity(pts.len() * 2);
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if out.is_empty() {
            out.push(a);
        }
        if (a[0] - b[0]).abs() > 0.5 && (a[1] - b[1]).abs() > 0.5 {
            let mid = (a[1] + b[1]) / 2.0;
            out.push([a[0], mid]);
            out.push([b[0], mid]);
        }
        out.push(b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_start_trims_to_box_border() {
        let mut pts = vec![[5.0, 5.0], [5.0, 8.0], [5.0, 30.0]];
        cut_start(&mut pts, [0.0, 0.0, 10.0, 10.0]);
        assert_eq!(pts, vec![[5.0, 10.0], [5.0, 30.0]]);
    }

    #[test]
    fn orthogonal_adds_elbows() {
        assert_eq!(orthogonal(&[[0.0, 0.0], [10.0, 10.0]]), vec![[0.0, 0.0], [0.0, 5.0], [10.0, 5.0], [10.0, 10.0]]);
    }
}
