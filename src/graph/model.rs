//! Graph data model: resolves the index's wikilink edges against the
//! vault's notes and files into nodes (notes, canvases, PDFs, CSV/XLSX
//! sheets (§3.8.3), and "ghost" nodes for links to notes that don't exist
//! yet) and undirected edges, optionally adding AI-similarity edges
//! between unlinked documents.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use uuid::Uuid;

use crate::core::LinkEdge;
use crate::core::ingestion::{pdf_doc_id, sheet_doc_id};
use crate::markdown::wikilink::title_key;
use crate::notes::Note;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Note,
    Canvas,
    Pdf,
    /// CSV/XLSX sheet (§3.8.3).
    Sheet,
    /// Target of an unresolved `[[link]]`.
    Ghost,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphNode {
    /// Stable identity across rebuilds (`note:<uuid>`, `pdf:<path>`,
    /// `ghost:<title key>`) — keys saved layout positions.
    pub key: String,
    pub label: String,
    pub kind: NodeKind,
    /// Note id or PDF doc id (none for ghosts).
    pub doc_id: Option<Uuid>,
    pub path: Option<PathBuf>,
    /// First tag, used for coloring.
    pub tag: Option<String>,
    /// Number of incident edges.
    pub degree: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// At least one wikilink between the two nodes.
    Link,
    /// Unlinked but semantically similar (AI).
    Semantic,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphEdge {
    pub a: usize,
    pub b: usize,
    pub kind: EdgeKind,
    /// Link count, or cosine similarity for semantic edges.
    pub weight: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraphOptions {
    pub show_orphans: bool,
    pub show_ghosts: bool,
    pub show_pdfs: bool,
    pub show_semantic: bool,
}

impl Default for GraphOptions {
    fn default() -> Self {
        GraphOptions {
            show_orphans: true,
            show_ghosts: true,
            show_pdfs: true,
            show_semantic: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

pub fn note_key(id: Uuid) -> String {
    format!("note:{id}")
}

impl GraphData {
    /// Builds the graph from non-trashed `notes`, `files` (imported PDFs
    /// and vault sheets, told apart by extension; hidden by
    /// `!opts.show_pdfs`), the
    /// index's `links`, and `semantic` similarity pairs `(doc, doc, sim)`
    /// (ignored unless `opts.show_semantic`; pairs that are already linked
    /// are skipped).
    pub fn build(
        notes: &[Note],
        files: &[PathBuf],
        links: &[LinkEdge],
        semantic: &[(Uuid, Uuid, f32)],
        opts: GraphOptions,
    ) -> GraphData {
        let mut g = GraphData::default();
        let mut by_key: HashMap<String, usize> = HashMap::new();
        let mut by_doc: HashMap<Uuid, usize> = HashMap::new();
        let mut by_title: HashMap<String, usize> = HashMap::new();

        for note in notes.iter().filter(|n| !n.frontmatter.trashed) {
            let fm = &note.frontmatter;
            let idx = g.push_node(
                &mut by_key,
                GraphNode {
                    key: note_key(fm.id),
                    label: fm.title.clone(),
                    kind: if note.is_canvas() {
                        NodeKind::Canvas
                    } else {
                        NodeKind::Note
                    },
                    doc_id: Some(fm.id),
                    path: Some(note.path.clone()),
                    tag: note.effective_tags().first().cloned(),
                    degree: 0,
                },
            );
            by_doc.insert(fm.id, idx);
            // Reachable by title, file stem and aliases (title wins).
            for key in crate::markdown::wikilink::link_keys_for(note) {
                by_title.entry(key).or_insert(idx);
            }
        }
        if opts.show_pdfs {
            for file in files {
                let name = file
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let (kind, prefix, id) = if crate::sheet::is_sheet_path(file) {
                    (NodeKind::Sheet, "sheet", sheet_doc_id(file))
                } else {
                    (NodeKind::Pdf, "pdf", pdf_doc_id(file))
                };
                let idx = g.push_node(
                    &mut by_key,
                    GraphNode {
                        key: format!("{prefix}:{}", file.display()),
                        label: name.clone(),
                        kind,
                        doc_id: Some(id),
                        path: Some(file.clone()),
                        tag: None,
                        degree: 0,
                    },
                );
                by_doc.insert(id, idx);
                by_title.entry(title_key(&name)).or_insert(idx);
            }
        }

        let mut edge_index: HashMap<(usize, usize), usize> = HashMap::new();
        for link in links {
            let Some(&src) = by_doc.get(&link.src_id) else {
                continue;
            };
            let dst = match by_title.get(&link.target_key) {
                Some(&dst) => dst,
                // Missing files never become ghost *notes* (clicking a
                // ghost creates a note of that name).
                None if opts.show_ghosts
                    && !link.target_key.ends_with(".pdf")
                    && !crate::sheet::is_sheet_path(std::path::Path::new(&link.target_key)) =>
                {
                    g.push_node(
                        &mut by_key,
                        GraphNode {
                            key: format!("ghost:{}", link.target_key),
                            label: link.target.clone(),
                            kind: NodeKind::Ghost,
                            doc_id: None,
                            path: None,
                            tag: None,
                            degree: 0,
                        },
                    )
                }
                None => continue,
            };
            if src == dst {
                continue;
            }
            let pair = (src.min(dst), src.max(dst));
            match edge_index.get(&pair) {
                Some(&e) => g.edges[e].weight += 1.0,
                None => {
                    edge_index.insert(pair, g.edges.len());
                    g.edges.push(GraphEdge {
                        a: pair.0,
                        b: pair.1,
                        kind: EdgeKind::Link,
                        weight: 1.0,
                    });
                }
            }
        }

        if opts.show_semantic {
            for (x, y, sim) in semantic {
                let (Some(&a), Some(&b)) = (by_doc.get(x), by_doc.get(y)) else {
                    continue;
                };
                if a == b {
                    continue;
                }
                let pair = (a.min(b), a.max(b));
                if edge_index.contains_key(&pair) {
                    continue;
                }
                edge_index.insert(pair, g.edges.len());
                g.edges.push(GraphEdge {
                    a: pair.0,
                    b: pair.1,
                    kind: EdgeKind::Semantic,
                    weight: *sim,
                });
            }
        }

        for e in &g.edges {
            g.nodes[e.a].degree += 1;
            g.nodes[e.b].degree += 1;
        }
        if !opts.show_orphans {
            g = g.retain_nodes(|n| n.degree > 0);
        }
        g
    }

    fn push_node(&mut self, by_key: &mut HashMap<String, usize>, node: GraphNode) -> usize {
        if let Some(&idx) = by_key.get(&node.key) {
            return idx;
        }
        by_key.insert(node.key.clone(), self.nodes.len());
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    /// Index of the node with `key`.
    pub fn index_of(&self, key: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.key == key)
    }

    /// Adjacency list (undirected).
    pub fn adjacency(&self) -> Vec<Vec<usize>> {
        let mut adj = vec![Vec::new(); self.nodes.len()];
        for e in &self.edges {
            adj[e.a].push(e.b);
            adj[e.b].push(e.a);
        }
        adj
    }

    /// The subgraph within `depth` hops of `center` (Obsidian's "local
    /// graph"). Degrees are recomputed for the subgraph.
    pub fn neighborhood(&self, center: usize, depth: usize) -> GraphData {
        if center >= self.nodes.len() {
            return GraphData::default();
        }
        let adj = self.adjacency();
        let mut dist = vec![usize::MAX; self.nodes.len()];
        let mut queue = VecDeque::from([center]);
        dist[center] = 0;
        while let Some(i) = queue.pop_front() {
            if dist[i] >= depth {
                continue;
            }
            for &j in &adj[i] {
                if dist[j] == usize::MAX {
                    dist[j] = dist[i] + 1;
                    queue.push_back(j);
                }
            }
        }
        let keep: HashSet<usize> = (0..self.nodes.len()).filter(|&i| dist[i] != usize::MAX).collect();
        let mut sub = self.retain_indices(&keep);
        for n in &mut sub.nodes {
            n.degree = 0;
        }
        for e in &sub.edges {
            sub.nodes[e.a].degree += 1;
            sub.nodes[e.b].degree += 1;
        }
        sub
    }

    fn retain_nodes(&self, keep: impl Fn(&GraphNode) -> bool) -> GraphData {
        let set: HashSet<usize> = (0..self.nodes.len()).filter(|&i| keep(&self.nodes[i])).collect();
        self.retain_indices(&set)
    }

    fn retain_indices(&self, keep: &HashSet<usize>) -> GraphData {
        let mut remap = vec![usize::MAX; self.nodes.len()];
        let mut nodes = Vec::with_capacity(keep.len());
        for (i, node) in self.nodes.iter().enumerate() {
            if keep.contains(&i) {
                remap[i] = nodes.len();
                nodes.push(node.clone());
            }
        }
        let edges = self
            .edges
            .iter()
            .filter(|e| remap[e.a] != usize::MAX && remap[e.b] != usize::MAX)
            .map(|e| GraphEdge {
                a: remap[e.a],
                b: remap[e.b],
                ..e.clone()
            })
            .collect();
        GraphData { nodes, edges }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn link(src: &Note, target: &str) -> LinkEdge {
        LinkEdge {
            src_id: src.frontmatter.id,
            target: target.to_string(),
            target_key: title_key(target),
        }
    }

    fn labels(g: &GraphData) -> Vec<&str> {
        g.nodes.iter().map(|n| n.label.as_str()).collect()
    }

    #[test]
    fn links_resolve_to_notes_pdfs_and_ghosts_and_merge_duplicates() {
        let dir = tempdir().unwrap();
        let mut a = Note::create(dir.path(), "A", "").unwrap();
        a.frontmatter.tags = vec!["kerja".into()];
        let b = Note::create(dir.path(), "B", "").unwrap();
        let pdf = dir.path().join("Laporan.pdf");
        let links = vec![
            link(&a, "b"),
            link(&a, "B"),
            link(&b, "A"),
            link(&a, "laporan.pdf"),
            link(&b, "Belum Ada"),
            link(&a, "A"), // self-link ignored
        ];

        let g = GraphData::build(&[a, b], &[pdf], &links, &[], GraphOptions::default());
        assert_eq!(labels(&g), vec!["A", "B", "Laporan.pdf", "Belum Ada"]);
        assert_eq!(g.nodes[0].tag.as_deref(), Some("kerja"));
        assert_eq!(g.nodes[3].kind, NodeKind::Ghost);
        assert_eq!(g.edges.len(), 3);
        let ab = g.edges.iter().find(|e| (e.a, e.b) == (0, 1)).unwrap();
        assert_eq!(ab.weight, 3.0);
        assert_eq!(g.nodes[0].degree, 2);
    }

    #[test]
    fn sheets_become_sheet_nodes_and_never_ghosts() {
        let dir = tempdir().unwrap();
        let a = Note::create(dir.path(), "A", "").unwrap();
        let sheet = dir.path().join("Kas.csv");
        let links = vec![link(&a, "kas.csv"), link(&a, "hilang.csv")];
        let g = GraphData::build(&[a], std::slice::from_ref(&sheet), &links, &[], GraphOptions::default());
        assert_eq!(labels(&g), vec!["A", "Kas.csv"]);
        assert_eq!(g.nodes[1].kind, NodeKind::Sheet);
        assert_eq!(g.nodes[1].doc_id, Some(sheet_doc_id(&sheet)));
        assert_eq!(g.edges.len(), 1);
    }

    #[test]
    fn options_hide_ghosts_pdfs_and_orphans() {
        let dir = tempdir().unwrap();
        let a = Note::create(dir.path(), "A", "").unwrap();
        let b = Note::create(dir.path(), "B", "").unwrap();
        let orphan = Note::create(dir.path(), "Sendiri", "").unwrap();
        let mut trashed = Note::create(dir.path(), "Sampah", "").unwrap();
        trashed.frontmatter.trashed = true;
        let pdf = dir.path().join("x.pdf");
        let links = vec![link(&a, "B"), link(&a, "Hantu"), link(&b, "x.pdf")];

        let opts = GraphOptions {
            show_orphans: false,
            show_ghosts: false,
            show_pdfs: false,
            show_semantic: false,
        };
        let g = GraphData::build(&[a, b, orphan, trashed], &[pdf], &links, &[], opts);
        assert_eq!(labels(&g), vec!["A", "B"]);
        assert_eq!(g.edges.len(), 1);
        assert_eq!((g.edges[0].a, g.edges[0].b), (0, 1));
    }

    #[test]
    fn semantic_edges_are_added_only_when_enabled_and_not_already_linked() {
        let dir = tempdir().unwrap();
        let a = Note::create(dir.path(), "A", "").unwrap();
        let b = Note::create(dir.path(), "B", "").unwrap();
        let c = Note::create(dir.path(), "C", "").unwrap();
        let (ia, ib, ic) = (a.frontmatter.id, b.frontmatter.id, c.frontmatter.id);
        let links = vec![link(&a, "B")];
        let semantic = vec![(ia, ib, 0.9), (ia, ic, 0.85), (ic, ia, 0.85)];
        let notes = [a, b, c];

        let off = GraphData::build(&notes, &[], &links, &semantic, GraphOptions::default());
        assert_eq!(off.edges.len(), 1);

        let on = GraphData::build(
            &notes,
            &[],
            &links,
            &semantic,
            GraphOptions {
                show_semantic: true,
                ..Default::default()
            },
        );
        assert_eq!(on.edges.len(), 2);
        let sem = on.edges.iter().find(|e| e.kind == EdgeKind::Semantic).unwrap();
        assert_eq!((sem.a, sem.b), (0, 2));
        assert_eq!(sem.weight, 0.85);
    }

    #[test]
    fn neighborhood_limits_depth_and_recomputes_degree() {
        let dir = tempdir().unwrap();
        let notes: Vec<Note> = ["A", "B", "C", "D"]
            .iter()
            .map(|t| Note::create(dir.path(), t, "").unwrap())
            .collect();
        // Chain A - B - C - D
        let links = vec![
            link(&notes[0], "B"),
            link(&notes[1], "C"),
            link(&notes[2], "D"),
        ];
        let g = GraphData::build(&notes, &[], &links, &[], GraphOptions::default());
        let b = g.index_of(&note_key(notes[1].frontmatter.id)).unwrap();

        let one = g.neighborhood(b, 1);
        assert_eq!(labels(&one), vec!["A", "B", "C"]);
        assert_eq!(one.edges.len(), 2);
        assert_eq!(one.nodes[2].degree, 1); // C lost its edge to D

        let two = g.neighborhood(b, 2);
        assert_eq!(two.nodes.len(), 4);
        assert!(g.neighborhood(99, 1).nodes.is_empty());
    }
}
