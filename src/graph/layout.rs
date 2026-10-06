//! Force-directed layout (Fruchterman–Reingold style): nodes repel each
//! other, edges pull their endpoints toward a rest length, and weak
//! gravity keeps disconnected components on screen. Repulsion uses a
//! uniform grid so only nearby nodes interact — O(n) per step instead of
//! O(n²) — which keeps vaults of a few thousand notes interactive. One
//! `step` per frame; the simulation cools down and reports when it has
//! settled so the UI can stop requesting repaints.

use std::collections::HashMap;

use super::model::{EdgeKind, GraphData};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ForceParams {
    /// Repulsion strength between any two nearby nodes.
    pub repulsion: f32,
    /// Nodes farther apart than this don't repel (grid cell size).
    pub repulsion_cutoff: f32,
    /// Spring stiffness of link edges.
    pub spring: f32,
    /// Preferred edge length.
    pub rest_length: f32,
    /// Pull toward the origin.
    pub gravity: f32,
    /// Fraction of velocity kept each step.
    pub damping: f32,
}

impl Default for ForceParams {
    fn default() -> Self {
        ForceParams {
            repulsion: 2400.0,
            repulsion_cutoff: 260.0,
            spring: 0.1,
            rest_length: 70.0,
            gravity: 0.004,
            damping: 0.82,
        }
    }
}

/// Positions and simulation state, parallel to `GraphData::nodes`.
#[derive(Debug, Clone)]
pub struct Layout {
    pub positions: Vec<[f32; 2]>,
    velocities: Vec<[f32; 2]>,
    /// Pinned nodes (being dragged) ignore forces.
    pub pinned: Vec<bool>,
    /// Maximum displacement per step; decays toward `MIN_TEMPERATURE`.
    temperature: f32,
    last_energy: f32,
}

const START_TEMPERATURE: f32 = 40.0;
const MIN_TEMPERATURE: f32 = 0.5;
const COOLING: f32 = 0.985;
/// Mean per-node movement below which the layout counts as settled.
const SETTLED_ENERGY: f32 = 0.05;

impl Layout {
    /// Initial positions: `saved(i)` when known (so the graph doesn't
    /// re-explode every time it opens), otherwise a deterministic golden-
    /// angle spiral. A fully restored layout starts cool.
    pub fn new(n: usize, saved: impl Fn(usize) -> Option<[f32; 2]>) -> Layout {
        let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
        let mut restored = 0;
        let positions = (0..n)
            .map(|i| match saved(i) {
                Some(p) if p[0].is_finite() && p[1].is_finite() => {
                    restored += 1;
                    p
                }
                _ => {
                    let r = 30.0 * ((i + 1) as f32).sqrt();
                    let a = i as f32 * golden;
                    [r * a.cos(), r * a.sin()]
                }
            })
            .collect();
        let temperature = if n > 0 && restored == n {
            START_TEMPERATURE * 0.1
        } else {
            START_TEMPERATURE
        };
        Layout {
            positions,
            velocities: vec![[0.0; 2]; n],
            pinned: vec![false; n],
            temperature,
            last_energy: f32::MAX,
        }
    }

    /// Like `new`, but unsaved nodes start clustered by connected component
    /// (each component on its own small spiral, members in BFS order) so
    /// linked nodes begin next to each other. Starting from an arbitrary
    /// spiral lets symmetric configurations — linked pairs stuck on
    /// opposite sides — trap the simulation in a poor local minimum.
    pub fn for_graph(graph: &GraphData, saved: impl Fn(usize) -> Option<[f32; 2]>) -> Layout {
        let n = graph.nodes.len();
        let adj = graph.adjacency();
        let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
        let mut seeded = vec![None; n];
        let mut component = 0usize;
        let mut visited = vec![false; n];
        // Larger components first, so the biggest cluster sits centrally.
        let mut roots: Vec<usize> = (0..n).collect();
        roots.sort_by_key(|&i| std::cmp::Reverse(adj[i].len()));
        for root in roots {
            if visited[root] {
                continue;
            }
            let cr = 160.0 * (component as f32).sqrt();
            let ca = component as f32 * golden;
            let center = [cr * ca.cos(), cr * ca.sin()];
            let mut queue = std::collections::VecDeque::from([root]);
            visited[root] = true;
            let mut k = 0usize;
            while let Some(i) = queue.pop_front() {
                let r = 22.0 * (k as f32).sqrt();
                let a = k as f32 * golden;
                seeded[i] = Some([center[0] + r * a.cos(), center[1] + r * a.sin()]);
                k += 1;
                for &j in &adj[i] {
                    if !visited[j] {
                        visited[j] = true;
                        queue.push_back(j);
                    }
                }
            }
            component += 1;
        }
        let mut layout = Layout::new(n, |i| saved(i).or(seeded[i]));
        if (0..n).any(|i| saved(i).is_none()) {
            layout.temperature = START_TEMPERATURE;
        }
        layout
    }

    /// Wakes the simulation up (after dragging or a data change).
    pub fn reheat(&mut self) {
        self.temperature = self.temperature.max(START_TEMPERATURE * 0.5);
        self.last_energy = f32::MAX;
    }

    pub fn is_settled(&self) -> bool {
        self.last_energy < SETTLED_ENERGY || self.temperature <= MIN_TEMPERATURE
    }

    /// Advances the simulation one step; returns the mean displacement.
    pub fn step(&mut self, graph: &GraphData, params: &ForceParams) -> f32 {
        let n = self.positions.len().min(graph.nodes.len());
        if n == 0 {
            self.last_energy = 0.0;
            return 0.0;
        }
        let mut force = vec![[0.0f32; 2]; n];

        // Repulsion over a uniform grid: each node only looks at its own
        // and the 8 neighboring cells.
        let cell = params.repulsion_cutoff.max(1.0);
        let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for i in 0..n {
            let p = self.positions[i];
            grid.entry(((p[0] / cell).floor() as i32, (p[1] / cell).floor() as i32))
                .or_default()
                .push(i);
        }
        let cutoff_sq = params.repulsion_cutoff * params.repulsion_cutoff;
        for (&(cx, cy), members) in &grid {
            for dx in -1..=1 {
                for dy in -1..=1 {
                    let Some(others) = grid.get(&(cx + dx, cy + dy)) else {
                        continue;
                    };
                    for &i in members {
                        for &j in others {
                            if j <= i {
                                continue;
                            }
                            let mut d = [
                                self.positions[i][0] - self.positions[j][0],
                                self.positions[i][1] - self.positions[j][1],
                            ];
                            let mut dist_sq = d[0] * d[0] + d[1] * d[1];
                            if dist_sq > cutoff_sq {
                                continue;
                            }
                            if dist_sq < 1e-4 {
                                // Coincident nodes: nudge apart deterministically.
                                let angle = (i * 7 + j * 13) as f32;
                                d = [angle.cos() * 0.01, angle.sin() * 0.01];
                                dist_sq = 1e-4;
                            }
                            let dist = dist_sq.sqrt();
                            let f = params.repulsion / dist_sq.max(25.0);
                            let (ux, uy) = (d[0] / dist, d[1] / dist);
                            force[i][0] += ux * f;
                            force[i][1] += uy * f;
                            force[j][0] -= ux * f;
                            force[j][1] -= uy * f;
                        }
                    }
                }
            }
        }

        // Springs along edges; semantic edges pull more gently.
        for e in &graph.edges {
            if e.a >= n || e.b >= n {
                continue;
            }
            let d = [
                self.positions[e.b][0] - self.positions[e.a][0],
                self.positions[e.b][1] - self.positions[e.a][1],
            ];
            let dist = (d[0] * d[0] + d[1] * d[1]).sqrt().max(0.01);
            let stiffness = match e.kind {
                EdgeKind::Link => params.spring,
                EdgeKind::Semantic => params.spring * 0.35,
            };
            let f = stiffness * (dist - params.rest_length);
            let (fx, fy) = (d[0] / dist * f, d[1] / dist * f);
            force[e.a][0] += fx;
            force[e.a][1] += fy;
            force[e.b][0] -= fx;
            force[e.b][1] -= fy;
        }

        let mut moved = 0.0;
        for (i, f) in force.iter().enumerate() {
            if self.pinned[i] {
                self.velocities[i] = [0.0; 2];
                continue;
            }
            let p = self.positions[i];
            let fx = f[0] - p[0] * params.gravity;
            let fy = f[1] - p[1] * params.gravity;
            let v = &mut self.velocities[i];
            v[0] = (v[0] + fx) * params.damping;
            v[1] = (v[1] + fy) * params.damping;
            let speed = (v[0] * v[0] + v[1] * v[1]).sqrt();
            if speed > self.temperature {
                let s = self.temperature / speed;
                v[0] *= s;
                v[1] *= s;
            }
            if !(v[0].is_finite() && v[1].is_finite()) {
                *v = [0.0; 2];
            }
            self.positions[i][0] += v[0];
            self.positions[i][1] += v[1];
            moved += (v[0] * v[0] + v[1] * v[1]).sqrt();
        }

        self.temperature = (self.temperature * COOLING).max(MIN_TEMPERATURE);
        self.last_energy = moved / n as f32;
        self.last_energy
    }

    /// Runs up to `max_steps` steps or until settled.
    pub fn settle(&mut self, graph: &GraphData, params: &ForceParams, max_steps: usize) {
        for _ in 0..max_steps {
            self.step(graph, params);
            if self.is_settled() {
                break;
            }
        }
    }

    /// Axis-aligned bounds `[min_x, min_y, max_x, max_y]` of all nodes.
    pub fn bounds(&self) -> Option<[f32; 4]> {
        let first = self.positions.first()?;
        Some(self.positions.iter().fold(
            [first[0], first[1], first[0], first[1]],
            |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])],
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::model::{GraphEdge, GraphNode, NodeKind};

    fn graph(n: usize, edges: &[(usize, usize)]) -> GraphData {
        GraphData {
            nodes: (0..n)
                .map(|i| GraphNode {
                    key: format!("n{i}"),
                    label: format!("n{i}"),
                    kind: NodeKind::Note,
                    doc_id: None,
                    path: None,
                    tag: None,
                    degree: 0,
                })
                .collect(),
            edges: edges
                .iter()
                .map(|&(a, b)| GraphEdge {
                    a,
                    b,
                    kind: EdgeKind::Link,
                    weight: 1.0,
                })
                .collect(),
        }
    }

    fn dist(l: &Layout, a: usize, b: usize) -> f32 {
        let (p, q) = (l.positions[a], l.positions[b]);
        ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt()
    }

    #[test]
    fn linked_nodes_end_up_closer_than_unlinked_ones() {
        // Two separate pairs: 0-1 and 2-3.
        let g = graph(4, &[(0, 1), (2, 3)]);
        let mut layout = Layout::for_graph(&g, |_| None);
        layout.settle(&g, &ForceParams::default(), 2000);
        assert!(dist(&layout, 0, 1) < dist(&layout, 0, 2));
        assert!(dist(&layout, 2, 3) < dist(&layout, 1, 3));
        assert!(layout.positions.iter().all(|p| p[0].is_finite() && p[1].is_finite()));
    }

    #[test]
    fn simulation_settles() {
        let edges: Vec<(usize, usize)> = (1..30).map(|i| (0, i)).collect();
        let g = graph(30, &edges);
        let mut layout = Layout::for_graph(&g, |_| None);
        layout.settle(&g, &ForceParams::default(), 5000);
        assert!(layout.is_settled());
    }

    #[test]
    fn coincident_nodes_are_pushed_apart() {
        let g = graph(2, &[]);
        let mut layout = Layout::new(2, |_| Some([5.0, 5.0]));
        layout.reheat();
        for _ in 0..50 {
            layout.step(&g, &ForceParams::default());
        }
        assert!(dist(&layout, 0, 1) > 1.0);
    }

    #[test]
    fn pinned_nodes_do_not_move() {
        let g = graph(2, &[(0, 1)]);
        let mut layout = Layout::new(2, |i| Some([i as f32 * 500.0, 0.0]));
        layout.pinned[0] = true;
        layout.reheat();
        for _ in 0..20 {
            layout.step(&g, &ForceParams::default());
        }
        assert_eq!(layout.positions[0], [0.0, 0.0]);
        assert!(layout.positions[1][0] < 500.0);
    }

    #[test]
    fn saved_positions_are_restored_and_start_cool() {
        let layout = Layout::new(2, |i| Some([i as f32, 1.0]));
        assert_eq!(layout.positions, vec![[0.0, 1.0], [1.0, 1.0]]);
        assert_eq!(layout.bounds(), Some([0.0, 1.0, 1.0, 1.0]));
        let fresh = Layout::new(3, |_| None);
        assert_ne!(fresh.positions[1], fresh.positions[2]);
        assert!(Layout::new(0, |_| None).bounds().is_none());
    }
}
