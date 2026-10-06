use super::*;

fn node(g: &mut Graph, w: f32, h: f32, cluster: Option<usize>) -> usize {
    g.nodes.push(NodeIn { size: [w, h], cluster });
    g.nodes.len() - 1
}

fn edge(g: &mut Graph, a: End, b: End) {
    g.edges.push(EdgeIn { from: a, to: b, minlen: 1, weight: 1.0, label: None });
}

fn rect_of(l: &Layout, g: &Graph, i: usize) -> [f32; 4] {
    let (c, s) = (l.nodes[i], g.nodes[i].size);
    [c[0] - s[0] / 2.0, c[1] - s[1] / 2.0, c[0] + s[0] / 2.0, c[1] + s[1] / 2.0]
}

fn overlap(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[2] - 0.5 && b[0] < a[2] - 0.5 && a[1] < b[3] - 0.5 && b[1] < a[3] - 0.5
}

fn assert_no_overlaps(l: &Layout, g: &Graph) {
    for i in 0..g.nodes.len() {
        for j in i + 1..g.nodes.len() {
            assert!(!overlap(rect_of(l, g, i), rect_of(l, g, j)), "nodes {i} and {j} overlap");
        }
    }
}

#[test]
fn chain_flows_in_each_direction() {
    for dir in [Dir::TB, Dir::BT, Dir::LR, Dir::RL] {
        let mut g = Graph::new(dir);
        let a = node(&mut g, 60.0, 30.0, None);
        let b = node(&mut g, 60.0, 30.0, None);
        let c = node(&mut g, 60.0, 30.0, None);
        edge(&mut g, End::Node(a), End::Node(b));
        edge(&mut g, End::Node(b), End::Node(c));
        let l = layout(&g);
        let axis = |p: P| match dir {
            Dir::TB => p[1],
            Dir::BT => -p[1],
            Dir::LR => p[0],
            Dir::RL => -p[0],
        };
        assert!(axis(l.nodes[a]) < axis(l.nodes[b]) && axis(l.nodes[b]) < axis(l.nodes[c]), "{dir:?}");
        assert!(l.width > 0.0 && l.height > 0.0);
        assert!(l.nodes.iter().all(|p| p[0] >= 0.0 && p[1] >= 0.0));
    }
}

#[test]
fn fan_out_is_centered_and_free_of_overlaps() {
    let mut g = Graph::new(Dir::TB);
    let root = node(&mut g, 80.0, 40.0, None);
    let kids: Vec<usize> = (0..4).map(|_| node(&mut g, 80.0, 40.0, None)).collect();
    for &k in &kids {
        edge(&mut g, End::Node(root), End::Node(k));
    }
    let l = layout(&g);
    assert_no_overlaps(&l, &g);
    let mean = kids.iter().map(|&k| l.nodes[k][0]).sum::<f32>() / 4.0;
    assert!((l.nodes[root][0] - mean).abs() < 2.0, "parent centred over children");
    assert!(kids.iter().all(|&k| (l.nodes[k][1] - l.nodes[kids[0]][1]).abs() < 1e-3));
}

#[test]
fn cycles_and_self_loops_are_laid_out() {
    let mut g = Graph::new(Dir::TB);
    let a = node(&mut g, 40.0, 30.0, None);
    let b = node(&mut g, 40.0, 30.0, None);
    edge(&mut g, End::Node(a), End::Node(b));
    edge(&mut g, End::Node(b), End::Node(a));
    edge(&mut g, End::Node(a), End::Node(a));
    let l = layout(&g);
    assert_eq!(l.edges.len(), 3);
    assert!(l.edges.iter().all(|e| e.points.len() >= 2));
    assert!(l.edges[2].self_loop);
    // The back edge still runs from b to a.
    let back = &l.edges[1].points;
    let near = |p: P, q: P| (p[0] - q[0]).abs() < 1.0 && (p[1] - q[1]).abs() < 1.0;
    assert!(near(back[0], l.nodes[b]) && near(*back.last().unwrap(), l.nodes[a]));
}

#[test]
fn edge_labels_sit_between_their_nodes() {
    let mut g = Graph::new(Dir::TB);
    let a = node(&mut g, 40.0, 30.0, None);
    let b = node(&mut g, 40.0, 30.0, None);
    g.edges.push(EdgeIn { from: End::Node(a), to: End::Node(b), minlen: 1, weight: 1.0, label: Some([50.0, 20.0]) });
    let l = layout(&g);
    let lab = l.edges[0].label.expect("label position");
    assert!(lab[1] > l.nodes[a][1] + 15.0 && lab[1] < l.nodes[b][1] - 15.0);
}

#[test]
fn clusters_contain_their_members_and_do_not_overlap_outsiders() {
    // Isolated cluster with its own direction + a crossing cluster.
    let mut g = Graph::new(Dir::TB);
    g.clusters.push(ClusterIn { parent: None, dir: Some(Dir::LR), title: [60.0, 20.0] });
    g.clusters.push(ClusterIn { parent: None, dir: None, title: [40.0, 20.0] });
    let a1 = node(&mut g, 40.0, 30.0, Some(0));
    let a2 = node(&mut g, 40.0, 30.0, Some(0));
    edge(&mut g, End::Node(a1), End::Node(a2));
    let b1 = node(&mut g, 40.0, 30.0, Some(1));
    let b2 = node(&mut g, 40.0, 30.0, Some(1));
    edge(&mut g, End::Node(b1), End::Node(b2));
    let out = node(&mut g, 40.0, 30.0, None);
    edge(&mut g, End::Node(out), End::Node(b1));
    edge(&mut g, End::Cluster(0), End::Node(out));
    let l = layout(&g);
    assert_no_overlaps(&l, &g);
    let c0 = l.clusters[0].expect("cluster 0 rect");
    let c1 = l.clusters[1].expect("cluster 1 rect");
    let inside = |r: [f32; 4], c: [f32; 4]| r[0] >= c[0] && r[1] >= c[1] && r[2] <= c[2] && r[3] <= c[3];
    assert!(inside(rect_of(&l, &g, a1), c0) && inside(rect_of(&l, &g, a2), c0));
    assert!(inside(rect_of(&l, &g, b1), c1) && inside(rect_of(&l, &g, b2), c1));
    assert!(!overlap(rect_of(&l, &g, out), c0) && !overlap(rect_of(&l, &g, out), c1));
    // Cluster 0 is isolated and LR: its members sit side by side.
    assert!((l.nodes[a1][1] - l.nodes[a2][1]).abs() < 1e-3 && l.nodes[a1][0] < l.nodes[a2][0]);
    // The edge from the cluster is marked for cluster clipping.
    assert_eq!(l.edges[3].from_cluster, Some(0));
    assert!(!overlap(c0, c1));
}

#[test]
fn larger_graph_has_no_overlaps() {
    let mut g = Graph::new(Dir::LR);
    let ids: Vec<usize> = (0..40).map(|i| node(&mut g, 30.0 + (i % 5) as f32 * 10.0, 30.0, None)).collect();
    for i in 0..40 {
        for j in [i * 2 + 1, i * 3 + 2] {
            if j < 40 {
                edge(&mut g, End::Node(ids[i]), End::Node(ids[j]));
            }
        }
    }
    let l = layout(&g);
    assert_no_overlaps(&l, &g);
}
