//! Section outline on the canvas (§3.9.2): one bound box per Markdown
//! segment (`markdown::sections`), laid out as a left-to-right mind map
//! (tidy tree: depth → column, subtrees stacked), with `outline` connectors
//! from each section to its children that follow the heading nesting.
//! Boxes the user moved stay where they are; only new segments are placed
//! (next to their parent). Also keeps attached connectors glued to their
//! nodes when a node moves. Pure geometry, no egui painting.
//! Callers: `markdown::editor` (canvas sync), `app::editor::canvas_surface`.

use std::collections::{HashMap, HashSet};

use egui::{Pos2, Rect};

use super::diagram_kinds::ConnectorMeta;
use super::element::{BlockBinding, CanvasElement, CanvasElementId, ConnectorRouting, ShapeKind};
use super::{CanvasDocument, tools};
use crate::markdown::sections::{Segment, SegmentKind};

/// Horizontal gap between mind-map columns.
const COL_GAP: f32 = 90.0;
/// Vertical gap between stacked boxes.
const ROW_GAP: f32 = 28.0;
const OUTLINE_EDGE_COLOR: [f32; 3] = [0.55, 0.60, 0.70];

/// World size a segment's box starts with.
pub fn box_size(seg: &Segment) -> [f32; 2] {
    let lines = seg.text.lines().count().max(1) as f32;
    let longest = seg.text.lines().map(|l| l.chars().count()).max().unwrap_or(10) as f32;
    match &seg.kind {
        SegmentKind::Section { .. } => {
            let w = (longest * 7.2 + 40.0).clamp(260.0, 420.0);
            let wraps: f32 = seg
                .text
                .lines()
                .map(|l| (l.chars().count() as f32 * 7.2 / (w - 30.0)).ceil().max(1.0))
                .sum();
            [w, (wraps * 20.0 + 44.0).clamp(64.0, 460.0)]
        }
        SegmentKind::Table => {
            let cols = seg
                .text
                .lines()
                .next()
                .map(|l| l.matches('|').count().saturating_sub(1))
                .unwrap_or(1)
                .max(1) as f32;
            let rows = seg.text.lines().filter(|l| !l.contains("---")).count() as f32;
            [(cols * 120.0).clamp(240.0, 720.0), (rows * 24.0 + 24.0).clamp(60.0, 480.0)]
        }
        SegmentKind::Mermaid => [460.0, 320.0],
        SegmentKind::Code { .. } => [(longest * 7.6 + 30.0).clamp(240.0, 520.0), (lines * 18.0 + 24.0).clamp(60.0, 400.0)],
        SegmentKind::Text => {
            let w = 300.0;
            let wraps = (seg.text.chars().count() as f32 * 7.2 / (w - 30.0)).ceil().max(lines);
            [w, (wraps * 19.0 + 28.0).clamp(56.0, 400.0)]
        }
    }
}

/// `(fill, stroke)` for a segment box. Light fills with dark text read in
/// both app themes.
fn box_colors(kind: &SegmentKind) -> ([f32; 3], [f32; 3]) {
    match kind {
        SegmentKind::Section { level: 1 } => ([0.90, 0.94, 1.0], tools::PALETTE_PRIMARY_ACCENT),
        SegmentKind::Section { .. } => ([0.95, 0.96, 0.99], [0.55, 0.62, 0.78]),
        SegmentKind::Table => ([0.94, 0.98, 0.94], [0.45, 0.70, 0.50]),
        SegmentKind::Mermaid => ([1.0, 1.0, 1.0], [0.62, 0.52, 0.85]),
        SegmentKind::Code { .. } => ([0.95, 0.95, 0.95], [0.55, 0.55, 0.60]),
        SegmentKind::Text => ([1.0, 0.98, 0.90], [0.80, 0.70, 0.45]),
    }
}

/// The bound box for `seg` at top-left `pos`.
pub fn segment_box(seg: &Segment, pos: [f32; 2], size: [f32; 2]) -> CanvasElement {
    let (fill, stroke) = box_colors(&seg.kind);
    CanvasElement::Shape {
        id: CanvasElementId::new(),
        kind: if seg.kind.is_component() { ShapeKind::Rectangle } else { ShapeKind::RoundedRect },
        rect: [pos[0], pos[1], pos[0] + size[0], pos[1] + size[1]],
        stroke_color: stroke,
        stroke_width: if matches!(seg.kind, SegmentKind::Section { level: 1 }) { 2.0 } else { 1.4 },
        fill_color: Some(fill),
        text: seg.text.clone(),
        text_color: None,
        binding: seg.id.clone().map(BlockBinding::segment),
    }
}

/// Top-left positions for a forest (`parents[i]` < `i`), left to right.
pub fn tree_layout(sizes: &[[f32; 2]], parents: &[Option<usize>], origin: [f32; 2]) -> Vec<[f32; 2]> {
    let n = sizes.len();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut depth = vec![0usize; n];
    for i in 0..n {
        if let Some(p) = parents[i].filter(|p| *p < i) {
            children[p].push(i);
            depth[i] = depth[p] + 1;
        }
    }
    let max_depth = depth.iter().copied().max().unwrap_or(0);
    let mut col_w = vec![0f32; max_depth + 1];
    for i in 0..n {
        col_w[depth[i]] = col_w[depth[i]].max(sizes[i][0]);
    }
    let mut col_x = vec![origin[0]; max_depth + 1];
    for d in 1..=max_depth {
        col_x[d] = col_x[d - 1] + col_w[d - 1] + COL_GAP;
    }
    let band = |kids: &[usize], span: &[f32]| {
        kids.iter().map(|c| span[*c]).sum::<f32>() + ROW_GAP * kids.len().saturating_sub(1) as f32
    };
    // Subtree heights, bottom-up (children always come after parents).
    let mut span = vec![0f32; n];
    for i in (0..n).rev() {
        span[i] = sizes[i][1].max(band(&children[i], &span));
    }
    let mut pos = vec![[0f32; 2]; n];
    let mut y = origin[1];
    let mut stack: Vec<(usize, f32)> = Vec::new();
    for r in (0..n).filter(|i| parents[*i].is_none_or(|p| p >= *i)) {
        stack.push((r, y));
        y += span[r] + ROW_GAP * 1.5;
    }
    while let Some((i, top)) = stack.pop() {
        let kids = band(&children[i], &span);
        // Parent centred on its children's band.
        pos[i] = [col_x[depth[i]], top + (span[i] - sizes[i][1]) / 2.0];
        let mut child_top = top + (span[i] - kids) / 2.0;
        for c in &children[i] {
            stack.push((*c, child_top));
            child_top += span[*c] + ROW_GAP;
        }
    }
    pos
}

/// A fresh canvas: one box per anchored segment, laid out as a mind map.
pub fn build_outline_canvas(title: &str, segments: &[Segment]) -> CanvasDocument {
    let mut doc = CanvasDocument::new(title);
    let sizes: Vec<[f32; 2]> = segments.iter().map(box_size).collect();
    let parents: Vec<Option<usize>> = segments.iter().map(|s| s.parent).collect();
    let positions = tree_layout(&sizes, &parents, [60.0, 60.0]);
    for (i, seg) in segments.iter().enumerate() {
        if seg.id.is_some() {
            doc.add_element(segment_box(seg, positions[i], sizes[i]));
        }
    }
    sync_outline_edges(&mut doc, segments);
    doc
}

/// Element bound (as a segment) to each local segment id.
pub fn segment_elements(doc: &CanvasDocument) -> HashMap<String, CanvasElementId> {
    doc.elements
        .iter()
        .filter_map(|e| {
            e.binding()
                .filter(|b| b.is_segment() && b.file.is_none())
                .map(|b| (b.block_id.clone(), e.id()))
        })
        .collect()
}

/// Whether the canvas mirrors the note's sections (any segment-bound box).
pub fn is_section_canvas(doc: &CanvasDocument) -> bool {
    doc.elements.iter().any(|e| e.binding().is_some_and(|b| b.is_segment()))
}

/// Adds a box for every anchored segment that has none (and isn't hidden),
/// placed beside its parent's box or below everything. Returns how many.
pub fn add_missing_boxes(doc: &mut CanvasDocument, segments: &[Segment]) -> usize {
    let hidden: HashSet<String> = doc.hidden_segments.iter().cloned().collect();
    let mut placed = segment_elements(doc);
    let mut added = 0;
    for seg in segments {
        let Some(id) = seg.id.as_deref() else { continue };
        if placed.contains_key(id) || hidden.contains(id) {
            continue;
        }
        let size = box_size(seg);
        let parent_rect = seg
            .parent
            .and_then(|p| segments[p].id.as_deref())
            .and_then(|pid| placed.get(pid))
            .and_then(|eid| doc.get_element(*eid))
            .map(|e| e.bounding_rect());
        let pos = match parent_rect {
            Some(pr) => {
                // Right of the parent, below its lowest box in that column.
                let x = pr.max.x + COL_GAP;
                let lowest = doc
                    .elements
                    .iter()
                    .filter(|e| e.is_node())
                    .map(|e| e.bounding_rect())
                    .filter(|r| (r.min.x - x).abs() < 1.0 && r.max.y >= pr.min.y)
                    .map(|r| r.max.y + ROW_GAP)
                    .fold(pr.min.y, f32::max);
                [x, lowest]
            }
            None => {
                let bounds = doc
                    .elements
                    .iter()
                    .filter(|e| e.is_node())
                    .map(|e| e.bounding_rect())
                    .fold(Rect::NOTHING, |a, r| a.union(r));
                if bounds.is_positive() { [bounds.min.x, bounds.max.y + ROW_GAP * 2.0] } else { [60.0, 60.0] }
            }
        };
        let elem = segment_box(seg, pos, size);
        placed.insert(id.to_string(), elem.id());
        doc.add_element(elem);
        added += 1;
    }
    added
}

/// Makes the `outline` connectors match the heading nesting: one per
/// (parent box, child box); stale ones removed. User connectors untouched.
/// Returns whether anything changed.
pub fn sync_outline_edges(doc: &mut CanvasDocument, segments: &[Segment]) -> bool {
    let boxes = segment_elements(doc);
    let mut wanted: Vec<(CanvasElementId, CanvasElementId)> = Vec::new();
    for seg in segments {
        let (Some(id), Some(p)) = (seg.id.as_deref(), seg.parent) else { continue };
        let Some(pid) = segments[p].id.as_deref() else { continue };
        if let (Some(from), Some(to)) = (boxes.get(pid), boxes.get(id)) {
            wanted.push((*from, *to));
        }
    }
    let before = doc.elements.len();
    doc.elements.retain(|e| match e {
        CanvasElement::Connector { from_elem: Some(f), to_elem: Some(t), meta, .. } if meta.outline => {
            wanted.contains(&(*f, *t))
        }
        CanvasElement::Connector { meta, .. } => !meta.outline,
        _ => true,
    });
    let mut changed = doc.elements.len() != before;
    let have: HashSet<(CanvasElementId, CanvasElementId)> = doc
        .elements
        .iter()
        .filter_map(|e| match e {
            CanvasElement::Connector { from_elem: Some(f), to_elem: Some(t), meta, .. } if meta.outline => {
                Some((*f, *t))
            }
            _ => None,
        })
        .collect();
    for (from, to) in wanted {
        if have.contains(&(from, to)) {
            continue;
        }
        let (Some(fr), Some(tr)) = (
            doc.get_element(from).map(|e| e.bounding_rect()),
            doc.get_element(to).map(|e| e.bounding_rect()),
        ) else {
            continue;
        };
        let (a, b) = attach_points(fr, tr);
        // Outline edges go under the boxes.
        doc.elements.insert(
            0,
            CanvasElement::Connector {
                id: CanvasElementId::new(),
                from_elem: Some(from),
                to_elem: Some(to),
                from_pos: a,
                to_pos: b,
                routing: ConnectorRouting::Orthogonal,
                stroke_color: OUTLINE_EDGE_COLOR,
                stroke_width: 1.5,
                label: String::new(),
                arrow_end: false,
                waypoints: Vec::new(),
                meta: ConnectorMeta::outline(),
            },
        );
        changed = true;
    }
    changed
}

/// Border midpoints two rects most naturally connect through.
pub fn attach_points(from: Rect, to: Rect) -> ([f32; 2], [f32; 2]) {
    let d = to.center() - from.center();
    let (a, b): (Pos2, Pos2) = if d.x.abs() >= d.y.abs() {
        if d.x >= 0.0 {
            (from.right_center(), to.left_center())
        } else {
            (from.left_center(), to.right_center())
        }
    } else if d.y >= 0.0 {
        (from.center_bottom(), to.center_top())
    } else {
        (from.center_top(), to.center_bottom())
    };
    ([a.x, a.y], [b.x, b.y])
}

/// Re-glues connectors attached to `moved` (or to anything, when `None`)
/// to the nearest sides of their nodes. Waypoints of a re-glued connector
/// are dropped, as they no longer lead anywhere sensible.
pub fn reattach_connectors(doc: &mut CanvasDocument, moved: Option<CanvasElementId>) {
    let rects: HashMap<CanvasElementId, Rect> =
        doc.elements.iter().filter(|e| e.is_node()).map(|e| (e.id(), e.bounding_rect())).collect();
    for elem in &mut doc.elements {
        let CanvasElement::Connector { from_elem, to_elem, from_pos, to_pos, waypoints, .. } = elem else {
            continue;
        };
        let touches = |id: &Option<CanvasElementId>| moved.is_none() || (id.is_some() && *id == moved);
        if !touches(from_elem) && !touches(to_elem) {
            continue;
        }
        let fr = from_elem.and_then(|id| rects.get(&id).copied());
        let tr = to_elem.and_then(|id| rects.get(&id).copied());
        let point = |p: &[f32; 2]| Rect::from_center_size(Pos2::new(p[0], p[1]), egui::Vec2::ZERO);
        match (fr, tr) {
            (Some(f), Some(t)) => {
                (*from_pos, *to_pos) = attach_points(f, t);
                waypoints.clear();
            }
            (Some(f), None) => *from_pos = attach_points(f, point(to_pos)).0,
            (None, Some(t)) => *to_pos = attach_points(point(from_pos), t).1,
            (None, None) => {}
        }
    }
}

/// Re-lays out every segment box as a fresh mind map (keeping ids, sizes
/// and bindings) and re-routes the connectors.
pub fn relayout(doc: &mut CanvasDocument, segments: &[Segment]) {
    let boxes = segment_elements(doc);
    let present: Vec<(usize, CanvasElementId)> = segments
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.id.as_deref().and_then(|id| boxes.get(id)).map(|e| (i, *e)))
        .collect();
    let index: HashMap<usize, usize> = present.iter().enumerate().map(|(k, (i, _))| (*i, k)).collect();
    let sizes: Vec<[f32; 2]> = present
        .iter()
        .map(|(i, eid)| {
            doc.get_element(*eid)
                .map(|e| e.bounding_rect())
                .map(|r| [r.width(), r.height()])
                .unwrap_or_else(|| box_size(&segments[*i]))
        })
        .collect();
    // Nearest present ancestor becomes the layout parent.
    let parents: Vec<Option<usize>> = present
        .iter()
        .map(|(i, _)| {
            let mut p = segments[*i].parent;
            while let Some(pi) = p {
                if let Some(k) = index.get(&pi) {
                    return Some(*k);
                }
                p = segments[pi].parent;
            }
            None
        })
        .collect();
    let positions = tree_layout(&sizes, &parents, [60.0, 60.0]);
    for (k, (_, eid)) in present.iter().enumerate() {
        if let Some(e) = doc.get_element_mut(*eid) {
            let r = e.bounding_rect();
            e.translate(egui::Vec2::new(positions[k][0] - r.min.x, positions[k][1] - r.min.y));
        }
    }
    sync_outline_edges(doc, segments);
    reattach_connectors(doc, None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::sections::segments;

    const BODY: &str = "# Produk ^p\nVisi.\n\n## Data ^d\nisi\n\n| a | b |\n|---|---|\n^t\n\n## Tim ^tm\norang\n";

    #[test]
    fn outline_canvas_has_one_box_per_segment_and_parent_edges() {
        let segs = segments(BODY);
        let doc = build_outline_canvas("N", &segs);
        let boxes = segment_elements(&doc);
        assert_eq!(boxes.len(), 4);
        let edges = doc
            .elements
            .iter()
            .filter(|e| matches!(e, CanvasElement::Connector { meta, .. } if meta.outline))
            .count();
        assert_eq!(edges, 3, "p→d, d→t, p→tm");
        // Children sit to the right of their parent.
        let x = |id: &str| doc.get_element(boxes[id]).unwrap().bounding_rect().min.x;
        assert!(x("d") > x("p") && x("t") > x("d") && x("tm") > x("p"));
    }

    #[test]
    fn missing_boxes_and_edges_follow_markdown_changes() {
        let segs = segments(BODY);
        let mut doc = build_outline_canvas("N", &segs);
        let body = format!("{BODY}\n### Baru ^b\nteks\n");
        let segs = segments(&body);
        assert_eq!(add_missing_boxes(&mut doc, &segs), 1);
        assert!(sync_outline_edges(&mut doc, &segs));
        assert_eq!(add_missing_boxes(&mut doc, &segs), 0);
        assert!(!sync_outline_edges(&mut doc, &segs));
        // Hidden segments don't come back.
        let mut doc2 = build_outline_canvas("N", &segments(BODY));
        doc2.hidden_segments.push("b".into());
        assert_eq!(add_missing_boxes(&mut doc2, &segs), 0);
    }

    #[test]
    fn reattach_follows_a_moved_box() {
        let segs = segments(BODY);
        let mut doc = build_outline_canvas("N", &segs);
        let d = segment_elements(&doc)["d"];
        doc.get_element_mut(d).unwrap().translate(egui::Vec2::new(0.0, 500.0));
        reattach_connectors(&mut doc, Some(d));
        let r = doc.get_element(d).unwrap().bounding_rect().expand(1.0);
        let glued = doc.elements.iter().any(|e| match e {
            CanvasElement::Connector { to_elem: Some(t), to_pos, .. } if *t == d => {
                r.contains(Pos2::new(to_pos[0], to_pos[1]))
            }
            _ => false,
        });
        assert!(glued);
    }

    #[test]
    fn relayout_keeps_bindings() {
        let segs = segments(BODY);
        let mut doc = build_outline_canvas("N", &segs);
        for e in &mut doc.elements {
            e.translate(egui::Vec2::new(999.0, 999.0));
        }
        relayout(&mut doc, &segs);
        assert_eq!(segment_elements(&doc).len(), 4);
        let p = doc.get_element(segment_elements(&doc)["p"]).unwrap().bounding_rect();
        assert!((p.min.x - 60.0).abs() < 0.5);
    }
}
