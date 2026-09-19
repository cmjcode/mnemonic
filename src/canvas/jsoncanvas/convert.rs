//! `CanvasDocument` ⇄ [`JsonCanvas`] mapping.
//!
//! | Canvas element                         | JSON Canvas                                  |
//! |----------------------------------------|----------------------------------------------|
//! | bound `StickyNote` / `Shape`           | `file` node (`file` + `subpath = "#^id"`), no text |
//! | section-bound box (§3.9.2)             | `file` node, `subpath = "#Heading"`, `mnemonic.scope = "segment"` + `block_id` |
//! | unbound `StickyNote` / `Shape`         | `text` node                                  |
//! | `Entity` / `ClassBox` (§3.9.3)         | `text` node (readable form) + `mnemonic.kind = "entity"/"class"` data |
//! | connector relation / outline / dashed  | `mnemonic.meta` on the edge                  |
//! | `Frame`                                | `group` node                                 |
//! | `DocCard`                              | `text` node + `mnemonic.kind = "doccard"`    |
//! | `Connector` attached at both ends      | `edge`                                       |
//! | dangling `Connector`, `FreehandStroke` | top-level `mnemonic.free_connectors/strokes` |
//! | `viewport`                             | top-level `mnemonic.viewport`                |

use std::collections::HashMap;

use egui::{Pos2, Rect};
use uuid::Uuid;

use super::model::*;
use crate::canvas::drawio::{parse_hex_color, to_hex_color};
use crate::canvas::element::{
    BindingScope, BlockBinding, CanvasElement, CanvasElementId, ConnectorRouting, ShapeKind,
};
use crate::canvas::{tools, CanvasDocument, Viewport};

/// Resolves the text of a bound block; `None` when the block doesn't exist.
pub type BlockResolver<'a> = dyn Fn(&BlockBinding) -> Option<String> + 'a;

const KIND_STICKY: &str = "sticky";
const KIND_SHAPE: &str = "shape";
const KIND_FRAME: &str = "frame";
const KIND_DOCCARD: &str = "doccard";
const KIND_ENTITY: &str = "entity";
const KIND_CLASS: &str = "class";
const SCOPE_SEGMENT: &str = "segment";

/// Stroke color for edges/connectors that don't carry one.
const DEFAULT_EDGE_COLOR: [f32; 3] = [0.45, 0.47, 0.50];

/// Obsidian's six preset colors (`"1"`..`"6"`): red, orange, yellow, green, cyan, purple.
const PRESET_COLORS: [&str; 6] = ["#fb464c", "#e9973f", "#e0de71", "#44cf6e", "#53dfdd", "#a882ff"];

/// A JSON Canvas `color` value (`"1"`..`"6"` preset or `#rrggbb`) as RGB.
pub fn parse_canvas_color(color: &str) -> Option<[f32; 3]> {
    let c = color.trim();
    if let Some(idx) = c.parse::<usize>().ok().filter(|i| (1..=6).contains(i)) {
        return parse_hex_color(PRESET_COLORS[idx - 1]);
    }
    parse_hex_color(c)
}

fn round_i(v: f32) -> i64 {
    v.round() as i64
}

fn rect_i(rect: [f32; 4]) -> [i64; 4] {
    let x = round_i(rect[0]);
    let y = round_i(rect[1]);
    [x, y, (round_i(rect[2]) - x).max(1), (round_i(rect[3]) - y).max(1)]
}

fn side_name(rect: Rect, point: [f32; 2]) -> &'static str {
    let p = Pos2::new(point[0], point[1]);
    let c = rect.center();
    let candidates = [
        ("top", Pos2::new(c.x, rect.min.y)),
        ("right", Pos2::new(rect.max.x, c.y)),
        ("bottom", Pos2::new(c.x, rect.max.y)),
        ("left", Pos2::new(rect.min.x, c.y)),
    ];
    candidates
        .iter()
        .min_by(|a, b| a.1.distance_sq(p).total_cmp(&b.1.distance_sq(p)))
        .map(|(n, _)| *n)
        .unwrap_or("right")
}

fn side_midpoint(rect: Rect, side: &str) -> [f32; 2] {
    let c = rect.center();
    let p = match side {
        "top" => Pos2::new(c.x, rect.min.y),
        "bottom" => Pos2::new(c.x, rect.max.y),
        "left" => Pos2::new(rect.min.x, c.y),
        _ => Pos2::new(rect.max.x, c.y),
    };
    [p.x, p.y]
}

/// Sides two rects most naturally connect through (used when the file names none).
fn natural_sides(from: Rect, to: Rect) -> (&'static str, &'static str) {
    let d = to.center() - from.center();
    if d.x.abs() >= d.y.abs() {
        if d.x >= 0.0 { ("right", "left") } else { ("left", "right") }
    } else if d.y >= 0.0 {
        ("bottom", "top")
    } else {
        ("top", "bottom")
    }
}

fn shape_kind_name(kind: ShapeKind) -> &'static str {
    kind.name()
}

fn shape_kind_from(name: Option<&str>) -> ShapeKind {
    ShapeKind::from_name(name.unwrap_or(""))
}

fn routing_name(routing: ConnectorRouting) -> &'static str {
    match routing {
        ConnectorRouting::Straight => "Straight",
        ConnectorRouting::Curved => "Curved",
        ConnectorRouting::Orthogonal => "Orthogonal",
    }
}

fn routing_from(name: Option<&str>, default: ConnectorRouting) -> ConnectorRouting {
    match name {
        Some("Straight") => ConnectorRouting::Straight,
        Some("Curved") => ConnectorRouting::Curved,
        Some("Orthogonal") => ConnectorRouting::Orthogonal,
        _ => default,
    }
}

/// The `file` of a bound node: the binding's file, else the owning note, else a
/// name derived from the canvas title (a `file` node must name something).
fn bound_file(binding: &BlockBinding, owner_note: Option<&str>, doc: &CanvasDocument) -> String {
    binding
        .file
        .clone()
        .or_else(|| owner_note.map(str::to_string))
        .unwrap_or_else(|| format!("{}.md", doc.title))
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// Convert a canvas into JSON Canvas. `owner_note` is the vault-relative path of the
/// note that owns the canvas (target of `file` nodes for locally bound elements).
pub fn to_json_canvas(doc: &CanvasDocument, owner_note: Option<&str>) -> JsonCanvas {
    // Nodes = everything that is neither a connector nor a stroke.
    let node_rects: HashMap<CanvasElementId, Rect> = doc
        .elements
        .iter()
        .filter(|e| !matches!(e, CanvasElement::Connector { .. } | CanvasElement::FreehandStroke { .. }))
        .map(|e| (e.id(), e.bounding_rect()))
        .collect();

    let mut jc = JsonCanvas::default();
    let mut ext = JcExtension {
        id: Some(doc.id.to_string()),
        viewport: Some(JcViewport {
            pan: doc.viewport.pan,
            zoom: doc.viewport.zoom,
        }),
        hidden_segments: doc.hidden_segments.clone(),
        ..Default::default()
    };

    for (z, elem) in doc.elements.iter().enumerate() {
        let id = elem.id().to_string();
        match elem {
            CanvasElement::StickyNote { pos, size, text, color, binding, .. } => {
                let rect = rect_i([pos[0], pos[1], pos[0] + size[0], pos[1] + size[1]]);
                let mut node = bound_or_text_node(&id, rect, text, binding.as_ref(), owner_note, doc);
                node.color = Some(to_hex_color(*color));
                node.mnemonic = Some(with_binding_ext(
                    JcNodeExt {
                        kind: Some(KIND_STICKY.into()),
                        color: Some(to_hex_color(*color)),
                        z: Some(z),
                        ..Default::default()
                    },
                    binding.as_ref(),
                ));
                jc.nodes.push(node);
            }
            CanvasElement::Shape {
                kind,
                rect,
                stroke_color,
                stroke_width,
                fill_color,
                text,
                text_color,
                binding,
                ..
            } => {
                let mut node = bound_or_text_node(&id, rect_i(*rect), text, binding.as_ref(), owner_note, doc);
                node.color = Some(to_hex_color(fill_color.unwrap_or(*stroke_color)));
                node.mnemonic = Some(with_binding_ext(
                    JcNodeExt {
                        kind: Some(KIND_SHAPE.into()),
                        shape: Some(shape_kind_name(*kind).into()),
                        stroke_color: Some(to_hex_color(*stroke_color)),
                        stroke_width: Some(*stroke_width),
                        fill_color: fill_color.map(to_hex_color),
                        text_color: text_color.map(to_hex_color),
                        z: Some(z),
                        ..Default::default()
                    },
                    binding.as_ref(),
                ));
                jc.nodes.push(node);
            }
            CanvasElement::Entity { rect, name, attributes, color, .. } => {
                // A text node, so Obsidian shows the entity readably.
                let mut node = JcNode::new(&id, NODE_TEXT, rect_i(*rect));
                node.text = elem.edit_text();
                node.color = Some(to_hex_color(*color));
                node.mnemonic = Some(JcNodeExt {
                    kind: Some(KIND_ENTITY.into()),
                    title: Some(name.clone()),
                    attributes: attributes.clone(),
                    z: Some(z),
                    ..Default::default()
                });
                jc.nodes.push(node);
            }
            CanvasElement::ClassBox { rect, name, annotation, attributes, methods, color, .. } => {
                let mut node = JcNode::new(&id, NODE_TEXT, rect_i(*rect));
                node.text = elem.edit_text();
                node.color = Some(to_hex_color(*color));
                node.mnemonic = Some(JcNodeExt {
                    kind: Some(KIND_CLASS.into()),
                    title: Some(name.clone()),
                    annotation: (!annotation.is_empty()).then(|| annotation.clone()),
                    members: attributes.clone(),
                    methods: methods.clone(),
                    z: Some(z),
                    ..Default::default()
                });
                jc.nodes.push(node);
            }
            CanvasElement::Frame { rect, title, color, .. } => {
                let mut node = JcNode::new(&id, NODE_GROUP, rect_i(*rect));
                node.label = (!title.is_empty()).then(|| title.clone());
                node.color = Some(to_hex_color(*color));
                node.mnemonic = Some(JcNodeExt {
                    kind: Some(KIND_FRAME.into()),
                    z: Some(z),
                    ..Default::default()
                });
                jc.nodes.push(node);
            }
            CanvasElement::DocCard { pos, size, note_id, title, snippet, doc_type, .. } => {
                let rect = rect_i([pos[0], pos[1], pos[0] + size[0], pos[1] + size[1]]);
                let mut node = JcNode::new(&id, NODE_TEXT, rect);
                node.text = Some(title.clone());
                node.mnemonic = Some(JcNodeExt {
                    kind: Some(KIND_DOCCARD.into()),
                    note_id: note_id.map(|u| u.to_string()),
                    title: Some(title.clone()),
                    snippet: Some(snippet.clone()),
                    doc_type: Some(doc_type.clone()),
                    z: Some(z),
                    ..Default::default()
                });
                jc.nodes.push(node);
            }
            CanvasElement::Connector {
                from_elem,
                to_elem,
                from_pos,
                to_pos,
                routing,
                stroke_color,
                stroke_width,
                label,
                arrow_end,
                waypoints,
                meta,
                ..
            } => {
                let from_rect = from_elem.and_then(|e| node_rects.get(&e).copied());
                let to_rect = to_elem.and_then(|e| node_rects.get(&e).copied());
                match (from_elem.zip(from_rect), to_elem.zip(to_rect)) {
                    (Some((from, fr)), Some((to, tr))) => jc.edges.push(JcEdge {
                        id,
                        from_node: from.to_string(),
                        from_side: Some(side_name(fr, *from_pos).into()),
                        from_end: None,
                        to_node: to.to_string(),
                        to_side: Some(side_name(tr, *to_pos).into()),
                        to_end: (!*arrow_end).then(|| END_NONE.to_string()),
                        color: Some(to_hex_color(*stroke_color)),
                        label: (!label.is_empty()).then(|| label.clone()),
                        mnemonic: Some(JcEdgeExt {
                            routing: Some(routing_name(*routing).into()),
                            stroke_width: Some(*stroke_width),
                            waypoints: waypoints.clone(),
                            from_pos: Some(*from_pos),
                            to_pos: Some(*to_pos),
                            z: Some(z),
                            meta: meta.clone(),
                            ..Default::default()
                        }),
                        extra: Default::default(),
                    }),
                    _ => ext.free_connectors.push(JcFreeConnector {
                        id,
                        from_node: from_elem.zip(from_rect).map(|(e, _)| e.to_string()),
                        to_node: to_elem.zip(to_rect).map(|(e, _)| e.to_string()),
                        from_pos: *from_pos,
                        to_pos: *to_pos,
                        routing: Some(routing_name(*routing).into()),
                        color: Some(to_hex_color(*stroke_color)),
                        stroke_width: Some(*stroke_width),
                        label: (!label.is_empty()).then(|| label.clone()),
                        arrow_end: *arrow_end,
                        waypoints: waypoints.clone(),
                        z: Some(z),
                        meta: meta.clone(),
                    }),
                }
            }
            CanvasElement::FreehandStroke { points, color, width, .. } => ext.strokes.push(JcStroke {
                id,
                points: points.clone(),
                color: Some(to_hex_color(*color)),
                width: Some(*width),
                z: Some(z),
            }),
        }
    }

    jc.mnemonic = Some(ext);
    jc
}

/// Records a section binding's scope and id in the node extension.
fn with_binding_ext(mut ext: JcNodeExt, binding: Option<&BlockBinding>) -> JcNodeExt {
    if let Some(b) = binding.filter(|b| b.is_segment()) {
        ext.scope = Some(SCOPE_SEGMENT.into());
        ext.block_id = Some(b.block_id.clone());
    }
    ext
}

/// Obsidian subpath of a bound node: a section whose text starts with a
/// heading embeds as `#Heading` (the whole section, as Obsidian shows
/// it); everything else as the block reference `#^id`.
fn bound_subpath(binding: &BlockBinding, text: &str) -> String {
    if binding.is_segment()
        && let Some(first) = text.lines().next()
        && crate::markdown::sections::heading_level(first).is_some()
    {
        let heading = first.trim_start().trim_start_matches('#').trim();
        if !heading.is_empty() {
            return format!("#{heading}");
        }
    }
    binding.subpath()
}

/// `file` node (text omitted) for a bound element, `text` node otherwise.
fn bound_or_text_node(
    id: &str,
    rect: [i64; 4],
    text: &str,
    binding: Option<&BlockBinding>,
    owner_note: Option<&str>,
    doc: &CanvasDocument,
) -> JcNode {
    match binding {
        Some(b) => {
            let mut node = JcNode::new(id, NODE_FILE, rect);
            node.file = Some(bound_file(b, owner_note, doc));
            node.subpath = Some(bound_subpath(b, text));
            node
        }
        None => {
            let mut node = JcNode::new(id, NODE_TEXT, rect);
            node.text = Some(text.to_string());
            node
        }
    }
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Convert JSON Canvas into a canvas. Bound nodes get their text from
/// `resolve_block_text` (empty when the block is missing). Files written by other
/// tools (no `mnemonic` blocks, preset colors) import with sensible defaults.
pub fn from_json_canvas(
    jc: &JsonCanvas,
    title: &str,
    owner_note: Option<&str>,
    resolve_block_text: &BlockResolver<'_>,
) -> CanvasDocument {
    let mut doc = CanvasDocument::new(title);
    if let Some(id) = jc.mnemonic.as_ref().and_then(|m| m.id.as_deref()).and_then(|s| Uuid::parse_str(s).ok()) {
        doc.id = id;
    }
    if let Some(m) = jc.mnemonic.as_ref() {
        doc.hidden_segments = m.hidden_segments.clone();
    }
    if let Some(vp) = jc.mnemonic.as_ref().and_then(|m| m.viewport.as_ref()) {
        doc.viewport = Viewport {
            pan: vp.pan,
            zoom: vp.zoom.clamp(Viewport::MIN_ZOOM, Viewport::MAX_ZOOM),
        };
    }

    // Element ids: reuse the node id when it is a UUID so ids are stable across saves.
    let ids: HashMap<&str, CanvasElementId> = jc
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), Uuid::parse_str(&n.id).map(CanvasElementId).unwrap_or_default()))
        .collect();

    // (z, element); z falls back to file order after everything that has one.
    let mut ordered: Vec<(usize, CanvasElement)> = Vec::new();
    let fallback_z = |ordinal: usize| usize::MAX / 2 + ordinal;
    let mut rects: HashMap<&str, Rect> = HashMap::new();

    for (ordinal, node) in jc.nodes.iter().enumerate() {
        let id = ids[node.id.as_str()];
        let rect = node.rect();
        rects.insert(node.id.as_str(), Rect::from_min_max(Pos2::new(rect[0], rect[1]), Pos2::new(rect[2], rect[3])));
        let z = node.mnemonic.as_ref().and_then(|m| m.z).unwrap_or_else(|| fallback_z(ordinal));
        if let Some(elem) = node_to_element(node, id, title, owner_note, resolve_block_text) {
            ordered.push((z, elem));
        }
    }

    for (ordinal, edge) in jc.edges.iter().enumerate() {
        let (Some(&from), Some(&to)) = (ids.get(edge.from_node.as_str()), ids.get(edge.to_node.as_str())) else {
            log::warn!("JSON Canvas import: skipping edge '{}' with unknown node", edge.id);
            continue;
        };
        let (Some(&from_rect), Some(&to_rect)) = (rects.get(edge.from_node.as_str()), rects.get(edge.to_node.as_str()))
        else {
            continue;
        };
        let ext = edge.mnemonic.as_ref();
        let (nat_from, nat_to) = natural_sides(from_rect, to_rect);
        let from_pos = ext
            .and_then(|m| m.from_pos)
            .unwrap_or_else(|| side_midpoint(from_rect, edge.from_side.as_deref().unwrap_or(nat_from)));
        let to_pos = ext
            .and_then(|m| m.to_pos)
            .unwrap_or_else(|| side_midpoint(to_rect, edge.to_side.as_deref().unwrap_or(nat_to)));
        let z = ext.and_then(|m| m.z).unwrap_or_else(|| fallback_z(jc.nodes.len() + ordinal));
        ordered.push((
            z,
            CanvasElement::Connector {
                id: Uuid::parse_str(&edge.id).map(CanvasElementId).unwrap_or_default(),
                from_elem: Some(from),
                to_elem: Some(to),
                from_pos,
                to_pos,
                // Obsidian draws edges as curves.
                routing: routing_from(ext.and_then(|m| m.routing.as_deref()), ConnectorRouting::Curved),
                stroke_color: edge.color.as_deref().and_then(parse_canvas_color).unwrap_or(DEFAULT_EDGE_COLOR),
                stroke_width: ext.and_then(|m| m.stroke_width).unwrap_or(2.0),
                label: edge.label.clone().unwrap_or_default(),
                arrow_end: edge.has_arrow_end(),
                waypoints: ext.map(|m| m.waypoints.clone()).unwrap_or_default(),
                meta: ext.map(|m| m.meta.clone()).unwrap_or_default(),
            },
        ));
    }

    if let Some(ext) = &jc.mnemonic {
        let base = jc.nodes.len() + jc.edges.len();
        for (ordinal, c) in ext.free_connectors.iter().enumerate() {
            let lookup = |n: &Option<String>| n.as_deref().and_then(|n| ids.get(n)).copied();
            ordered.push((
                c.z.unwrap_or_else(|| fallback_z(base + ordinal)),
                CanvasElement::Connector {
                    id: Uuid::parse_str(&c.id).map(CanvasElementId).unwrap_or_default(),
                    from_elem: lookup(&c.from_node),
                    to_elem: lookup(&c.to_node),
                    from_pos: c.from_pos,
                    to_pos: c.to_pos,
                    routing: routing_from(c.routing.as_deref(), ConnectorRouting::Straight),
                    stroke_color: c.color.as_deref().and_then(parse_canvas_color).unwrap_or(DEFAULT_EDGE_COLOR),
                    stroke_width: c.stroke_width.unwrap_or(2.0),
                    label: c.label.clone().unwrap_or_default(),
                    arrow_end: c.arrow_end,
                    waypoints: c.waypoints.clone(),
                    meta: c.meta.clone(),
                },
            ));
        }
        let base = base + ext.free_connectors.len();
        for (ordinal, s) in ext.strokes.iter().enumerate() {
            if s.points.is_empty() {
                continue;
            }
            ordered.push((
                s.z.unwrap_or_else(|| fallback_z(base + ordinal)),
                CanvasElement::FreehandStroke {
                    id: Uuid::parse_str(&s.id).map(CanvasElementId).unwrap_or_default(),
                    points: s.points.clone(),
                    color: s.color.as_deref().and_then(parse_canvas_color).unwrap_or(DEFAULT_EDGE_COLOR),
                    width: s.width.unwrap_or(2.0),
                },
            ));
        }
    }

    ordered.sort_by_key(|(z, _)| *z);
    doc.elements = ordered.into_iter().map(|(_, e)| e).collect();
    doc
}

fn node_to_element(
    node: &JcNode,
    id: CanvasElementId,
    title: &str,
    owner_note: Option<&str>,
    resolve: &BlockResolver<'_>,
) -> Option<CanvasElement> {
    let ext = node.mnemonic.as_ref();
    let kind = ext.and_then(|m| m.kind.as_deref());
    let rect = node.rect();
    let pos = [rect[0], rect[1]];
    let size = [rect[2] - rect[0], rect[3] - rect[1]];
    let node_color = node.color.as_deref().and_then(parse_canvas_color);

    let sticky_or_shape = |text: String, binding: Option<BlockBinding>| -> CanvasElement {
        if kind == Some(KIND_SHAPE) {
            let stroke_color = ext
                .and_then(|m| m.stroke_color.as_deref())
                .and_then(parse_hex_color)
                .or(node_color)
                .unwrap_or(DEFAULT_EDGE_COLOR);
            CanvasElement::Shape {
                id,
                kind: shape_kind_from(ext.and_then(|m| m.shape.as_deref())),
                rect,
                stroke_color,
                stroke_width: ext.and_then(|m| m.stroke_width).unwrap_or(2.0),
                fill_color: ext.and_then(|m| m.fill_color.as_deref()).and_then(parse_hex_color),
                text,
                text_color: ext.and_then(|m| m.text_color.as_deref()).and_then(parse_hex_color),
                binding,
            }
        } else {
            let color = ext
                .and_then(|m| m.color.as_deref())
                .and_then(parse_hex_color)
                .or(node_color)
                .unwrap_or(tools::PALETTE_STICKY_YELLOW);
            CanvasElement::StickyNote { id, pos, size, text, color, binding }
        }
    };

    let is_segment = ext.and_then(|m| m.scope.as_deref()) == Some(SCOPE_SEGMENT);
    let ext_block_id = ext.and_then(|m| m.block_id.as_deref()).filter(|id| !id.is_empty());
    match node.kind() {
        JcNodeKind::File { file, subpath } => {
            if let Some(block_id) = ext_block_id.or_else(|| node.block_ref()) {
                // Local when it names the owning note (or the title-derived fallback
                // `bound_file` writes when no owner is known).
                let is_local = file.is_empty()
                    || Some(file) == owner_note
                    || (owner_note.is_none() && file == format!("{title}.md"));
                let binding = BlockBinding {
                    file: (!is_local).then(|| file.to_string()),
                    block_id: block_id.to_string(),
                    scope: if is_segment { BindingScope::Segment } else { BindingScope::Block },
                };
                let text = resolve(&binding).unwrap_or_default();
                return Some(sticky_or_shape(text, Some(binding)));
            }
            // An embedded file (note, PDF, image, ...) without a block reference.
            let name = file.rsplit('/').next().unwrap_or(file);
            let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
            let is_pdf = name.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("pdf"));
            Some(CanvasElement::DocCard {
                id,
                pos,
                size,
                note_id: None,
                title: stem.to_string(),
                snippet: match subpath {
                    Some(sp) => format!("{file}{sp}"),
                    None => file.to_string(),
                },
                doc_type: if is_pdf { "pdf" } else { "note" }.to_string(),
            })
        }
        JcNodeKind::Text { text } => {
            let color = node_color.unwrap_or(tools::PALETTE_STROKE_LIGHT);
            if kind == Some(KIND_ENTITY) {
                let (text_name, text_attrs) = crate::canvas::diagram_kinds::entity_from_text(text);
                let m = ext?;
                return Some(CanvasElement::Entity {
                    id,
                    rect,
                    name: m.title.clone().unwrap_or(text_name),
                    attributes: if m.attributes.is_empty() { text_attrs } else { m.attributes.clone() },
                    color,
                });
            }
            if kind == Some(KIND_CLASS) {
                let m = ext?;
                let (text_name, ..) = crate::canvas::diagram_kinds::class_from_text(text);
                return Some(CanvasElement::ClassBox {
                    id,
                    rect,
                    name: m.title.clone().unwrap_or(text_name),
                    annotation: m.annotation.clone().unwrap_or_default(),
                    attributes: m.members.clone(),
                    methods: m.methods.clone(),
                    color,
                });
            }
            if kind == Some(KIND_DOCCARD) {
                let m = ext?;
                return Some(CanvasElement::DocCard {
                    id,
                    pos,
                    size,
                    note_id: m.note_id.as_deref().and_then(|s| Uuid::parse_str(s).ok()),
                    title: m.title.clone().unwrap_or_else(|| text.to_string()),
                    snippet: m.snippet.clone().unwrap_or_default(),
                    doc_type: m.doc_type.clone().unwrap_or_else(|| "note".to_string()),
                });
            }
            Some(sticky_or_shape(text.to_string(), None))
        }
        JcNodeKind::Link { url } => Some(sticky_or_shape(url.to_string(), None)),
        JcNodeKind::Group { label } => Some(CanvasElement::Frame {
            id,
            rect,
            title: label.unwrap_or_default().to_string(),
            color: node_color.unwrap_or(tools::PALETTE_PRIMARY_ACCENT),
        }),
        JcNodeKind::Unknown(t) => {
            log::warn!("JSON Canvas import: skipping node '{}' of unknown type '{t}'", node.id);
            None
        }
    }
}
