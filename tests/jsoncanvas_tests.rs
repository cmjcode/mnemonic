//! JSON Canvas (Obsidian `.canvas`) sidecar: lossless round trip, block bindings,
//! and interoperability with files authored by Obsidian.

use std::collections::HashMap;

use mnemonic::canvas::element::{BlockBinding, CanvasElement, CanvasElementId, ConnectorRouting, ShapeKind};
use mnemonic::canvas::jsoncanvas::{parse_canvas_color, JsonCanvas};
use mnemonic::canvas::{CanvasDocument, Viewport};

const OWNER: &str = "Notes/Judul.md";

/// Markdown blocks the resolver knows about.
fn blocks() -> HashMap<BlockBinding, String> {
    HashMap::from([
        (BlockBinding::local("abc123"), "Paragraf lokal yang terikat".to_string()),
        (BlockBinding::to_file("Other/Ref.md", "zz9pla"), "Teks dari catatan lain".to_string()),
    ])
}

fn resolver(blocks: &HashMap<BlockBinding, String>) -> impl Fn(&BlockBinding) -> Option<String> + '_ {
    move |b| blocks.get(b).cloned()
}

/// Comparable form of an element: everything but the (regenerated) ids.
/// Node coordinates travel as integers, colors as `#RRGGBB`.
fn fingerprint(e: &CanvasElement) -> String {
    let c8 = |c: &[f32; 3]| c.map(|v| (v * 255.0).round() as u8);
    match e {
        CanvasElement::Shape { kind, rect, stroke_color, stroke_width, fill_color, text, text_color, binding, .. } => {
            format!(
                "shape {kind:?} {rect:?} {:?} {stroke_width} {:?} {text:?} {:?} {binding:?}",
                c8(stroke_color),
                fill_color.as_ref().map(c8),
                text_color.as_ref().map(c8)
            )
        }
        CanvasElement::StickyNote { pos, size, text, color, binding, .. } => {
            format!("sticky {pos:?} {size:?} {text:?} {:?} {binding:?}", c8(color))
        }
        CanvasElement::Frame { rect, title, color, .. } => format!("frame {rect:?} {title:?} {:?}", c8(color)),
        CanvasElement::DocCard { pos, size, note_id, title, snippet, doc_type, .. } => {
            format!("doccard {pos:?} {size:?} {note_id:?} {title:?} {snippet:?} {doc_type}")
        }
        CanvasElement::FreehandStroke { points, color, width, .. } => {
            format!("freehand {points:?} {:?} {width}", c8(color))
        }
        CanvasElement::Connector {
            from_elem, to_elem, from_pos, to_pos, routing, stroke_color, stroke_width, label, arrow_end, waypoints, ..
        } => format!(
            "connector attached={},{} {from_pos:?} {to_pos:?} {routing:?} {:?} {stroke_width} {label:?} {arrow_end} {waypoints:?}",
            from_elem.is_some(),
            to_elem.is_some(),
            c8(stroke_color),
        ),
    }
}

/// A document using every element kind (bound and unbound), in a mixed z-order.
fn full_document() -> CanvasDocument {
    let blocks = blocks();
    let mut doc = CanvasDocument::new("full");
    doc.viewport = Viewport { pan: [12.5, -40.0], zoom: 1.75 };
    doc.add_element(CanvasElement::FreehandStroke {
        id: CanvasElementId::new(),
        points: vec![[10.0, 10.0], [20.5, 30.25], [40.0, 12.0]],
        color: [1.0, 0.0, 0.0],
        width: 3.0,
    });
    let bound_sticky = BlockBinding::local("abc123");
    let a = doc.add_element(CanvasElement::StickyNote {
        id: CanvasElementId::new(),
        pos: [100.0, 100.0],
        size: [200.0, 120.0],
        text: blocks[&bound_sticky].clone(),
        color: [1.0, 0.94, 0.55],
        binding: Some(bound_sticky),
    });
    doc.add_element(CanvasElement::Connector {
        id: CanvasElementId::new(),
        from_elem: Some(a),
        to_elem: None,
        from_pos: [300.0, 160.0],
        to_pos: [420.0, 160.0],
        routing: ConnectorRouting::Straight,
        stroke_color: [0.0, 0.0, 0.0],
        stroke_width: 1.0,
        label: "dangling".to_string(),
        arrow_end: false,
        waypoints: Vec::new(),
    });
    doc.add_element(CanvasElement::Frame {
        id: CanvasElementId::new(),
        rect: [0.0, 0.0, 800.0, 600.0],
        title: "Frame".to_string(),
        color: [0.0, 1.0, 0.0],
    });
    let bound_shape = BlockBinding::to_file("Other/Ref.md", "zz9pla");
    let b = doc.add_element(CanvasElement::Shape {
        id: CanvasElementId::new(),
        kind: ShapeKind::Diamond,
        rect: [500.0, 100.0, 620.0, 220.0],
        stroke_color: [0.0, 0.0, 1.0],
        stroke_width: 2.0,
        fill_color: Some([1.0, 1.0, 0.0]),
        text: blocks[&bound_shape].clone(),
        text_color: Some([1.0, 1.0, 1.0]),
        binding: Some(bound_shape),
    });
    doc.add_element(CanvasElement::StickyNote {
        id: CanvasElementId::new(),
        pos: [100.0, 300.0],
        size: [160.0, 100.0],
        text: "diagram-only note".to_string(),
        color: [1.0, 0.0, 1.0],
        binding: None,
    });
    doc.add_element(CanvasElement::Shape {
        id: CanvasElementId::new(),
        kind: ShapeKind::Ellipse,
        rect: [400.0, 300.0, 520.0, 380.0],
        stroke_color: [0.5, 0.5, 0.5],
        stroke_width: 0.0,
        fill_color: None,
        text: "diagram-only shape".to_string(),
        text_color: None,
        binding: None,
    });
    doc.add_element(CanvasElement::DocCard {
        id: CanvasElementId::new(),
        pos: [50.0, 450.0],
        size: [200.0, 90.0],
        note_id: Some(uuid::Uuid::parse_str("6f1c0f6e-3a55-4f7a-9d55-1b2f7c7c1a11").unwrap()),
        title: "Linked note".to_string(),
        snippet: "first line\nsecond line".to_string(),
        doc_type: "pdf".to_string(),
    });
    doc.add_element(CanvasElement::Connector {
        id: CanvasElementId::new(),
        from_elem: Some(a),
        to_elem: Some(b),
        from_pos: [200.0, 100.0],
        to_pos: [560.0, 220.0],
        routing: ConnectorRouting::Orthogonal,
        stroke_color: [0.0, 0.0, 0.0],
        stroke_width: 2.0,
        label: "a→b".to_string(),
        arrow_end: true,
        waypoints: vec![[200.0, 20.0], [560.0, 20.0]],
    });
    doc
}

fn roundtrip(doc: &CanvasDocument) -> CanvasDocument {
    let blocks = blocks();
    let json = doc.to_json_canvas_string(Some(OWNER));
    CanvasDocument::from_json_canvas_str(&json, &doc.title, Some(OWNER), &resolver(&blocks)).expect("valid JSON Canvas")
}

#[test]
fn roundtrip_is_lossless_for_every_element_kind() {
    let doc = full_document();
    let back = roundtrip(&doc);

    let want: Vec<String> = doc.elements.iter().map(fingerprint).collect();
    let got: Vec<String> = back.elements.iter().map(fingerprint).collect();
    assert_eq!(got, want, "elements (and z-order) must survive save → load");

    // Element ids, document id and viewport are stable across saves.
    let ids: Vec<CanvasElementId> = doc.elements.iter().map(|e| e.id()).collect();
    let back_ids: Vec<CanvasElementId> = back.elements.iter().map(|e| e.id()).collect();
    assert_eq!(back_ids, ids);
    assert_eq!(back.id, doc.id);
    assert_eq!(back.viewport, doc.viewport);

    // Connector ends point at the re-imported elements.
    let ends: Vec<(bool, bool)> = back
        .elements
        .iter()
        .filter_map(|c| match c {
            CanvasElement::Connector { from_elem, to_elem, .. } => Some((
                from_elem.is_some_and(|id| back.get_element(id).is_some()),
                to_elem.is_some_and(|id| back.get_element(id).is_some()),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(ends, vec![(true, false), (true, true)]);

    // A second cycle doesn't drift.
    let again = roundtrip(&back);
    let third: Vec<String> = again.elements.iter().map(fingerprint).collect();
    assert_eq!(third, want);
}

#[test]
fn bound_node_never_serializes_its_text() {
    let doc = full_document();
    let json = doc.to_json_canvas_string(Some(OWNER));
    assert!(!json.contains("Paragraf lokal"), "bound sticky text leaked:\n{json}");
    assert!(!json.contains("Teks dari catatan lain"), "bound shape text leaked:\n{json}");
    assert!(json.contains("diagram-only note"), "unbound text must be stored");

    let jc: JsonCanvas = serde_json::from_str(&json).unwrap();
    let local = jc.nodes.iter().find(|n| n.subpath.as_deref() == Some("#^abc123")).expect("bound sticky node");
    assert_eq!(local.node_type, "file");
    assert_eq!(local.file.as_deref(), Some(OWNER), "local binding targets the owning note");
    assert!(local.text.is_none());
    let foreign = jc.nodes.iter().find(|n| n.subpath.as_deref() == Some("#^zz9pla")).expect("bound shape node");
    assert_eq!(foreign.file.as_deref(), Some("Other/Ref.md"));
    assert_eq!(foreign.mnemonic.as_ref().and_then(|m| m.kind.as_deref()), Some("shape"));
    assert!(json.ends_with('\n'));
}

#[test]
fn bound_block_ids_lists_each_binding_once() {
    let mut doc = full_document();
    let dup = BlockBinding::local("abc123");
    doc.add_element(CanvasElement::StickyNote {
        id: CanvasElementId::new(),
        pos: [0.0, 0.0],
        size: [10.0, 10.0],
        text: String::new(),
        color: [1.0, 1.0, 1.0],
        binding: Some(dup),
    });
    assert_eq!(
        doc.bound_block_ids(),
        vec![BlockBinding::local("abc123"), BlockBinding::to_file("Other/Ref.md", "zz9pla")]
    );
}

#[test]
fn refresh_bound_text_follows_markdown_edits() {
    let mut doc = full_document();
    let mut blocks = blocks();
    assert!(!doc.refresh_bound_text(&resolver(&blocks)), "unchanged markdown → no change");

    blocks.insert(BlockBinding::local("abc123"), "Diedit di markdown".to_string());
    assert!(doc.refresh_bound_text(&resolver(&blocks)));
    let sticky = doc
        .elements
        .iter()
        .find(|e| e.binding() == Some(&BlockBinding::local("abc123")))
        .unwrap();
    assert_eq!(sticky.text(), Some("Diedit di markdown"));
    assert!(!doc.refresh_bound_text(&resolver(&blocks)), "idempotent");

    // A block that vanished keeps the last known text.
    blocks.clear();
    assert!(!doc.refresh_bound_text(&resolver(&blocks)));
    assert_eq!(sticky_text(&doc), "Diedit di markdown");

    // Unbound elements are never touched.
    let unbound_before: Vec<String> = doc
        .elements
        .iter()
        .filter(|e| !e.is_bound())
        .filter_map(|e| e.text().map(str::to_string))
        .collect();
    blocks.insert(BlockBinding::local("abc123"), "x".to_string());
    doc.refresh_bound_text(&resolver(&blocks));
    let unbound_after: Vec<String> = doc
        .elements
        .iter()
        .filter(|e| !e.is_bound())
        .filter_map(|e| e.text().map(str::to_string))
        .collect();
    assert_eq!(unbound_after, unbound_before);
}

fn sticky_text(doc: &CanvasDocument) -> String {
    doc.elements
        .iter()
        .find(|e| e.binding() == Some(&BlockBinding::local("abc123")))
        .and_then(|e| e.text())
        .unwrap()
        .to_string()
}

#[test]
fn missing_block_imports_with_empty_text() {
    let doc = full_document();
    let json = doc.to_json_canvas_string(Some(OWNER));
    let back = CanvasDocument::from_json_canvas_str(&json, "full", Some(OWNER), &|_| None).unwrap();
    let bound: Vec<&str> = back.elements.iter().filter(|e| e.is_bound()).filter_map(|e| e.text()).collect();
    assert_eq!(bound, vec!["", ""]);
}

#[test]
fn output_is_valid_json_canvas() {
    let doc = full_document();
    let json = doc.to_json_canvas_string(Some(OWNER));
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let nodes = v["nodes"].as_array().expect("nodes array");
    let edges = v["edges"].as_array().expect("edges array");
    // 2 bound + 2 unbound + frame + doccard; 1 edge (the dangling one is an extension).
    assert_eq!(nodes.len(), 6);
    assert_eq!(edges.len(), 1);
    for n in nodes {
        assert!(n["id"].is_string());
        let ty = n["type"].as_str().expect("type");
        assert!(matches!(ty, "text" | "file" | "link" | "group"), "bad type {ty}");
        for k in ["x", "y", "width", "height"] {
            assert!(n[k].is_i64(), "{k} must be an integer: {}", n[k]);
        }
        if let Some(c) = n["color"].as_str() {
            assert!(c.starts_with('#') && c.len() == 7, "color must be #rrggbb: {c}");
        }
        match ty {
            "text" => assert!(n["text"].is_string()),
            "file" => assert!(n["file"].is_string()),
            "group" => assert!(n.get("text").is_none()),
            _ => {}
        }
    }
    let e = &edges[0];
    assert!(e["id"].is_string() && e["fromNode"].is_string() && e["toNode"].is_string());
    assert!(matches!(e["fromSide"].as_str(), Some("top" | "right" | "bottom" | "left")));
    assert!(matches!(e["toSide"].as_str(), Some("top" | "right" | "bottom" | "left")));
    assert_eq!(e["fromSide"], "top", "from_pos [200,100] is the top-center of the sticky");
    assert_eq!(e["toSide"], "bottom", "to_pos [560,220] is the bottom-center of the diamond");
    assert!(e.get("toEnd").is_none(), "arrow is the default");

    let ext = &v["mnemonic"];
    assert_eq!(ext["free_connectors"].as_array().map(Vec::len), Some(1));
    assert_eq!(ext["strokes"].as_array().map(Vec::len), Some(1));
    assert_eq!(ext["viewport"]["zoom"], 1.75);
    assert!(json.starts_with("{\n  \"nodes\""), "pretty-printed with 2-space indent");
}

#[test]
fn imports_obsidian_authored_canvas() {
    let json = r##"{
  "nodes":[
    {"id":"n1","type":"text","x":-100,"y":40,"width":250,"height":60,"color":"4","text":"Ide utama"},
    {"id":"n2","type":"file","x":300,"y":40,"width":400,"height":400,"file":"Riset/Paper.pdf"},
    {"id":"n3","type":"file","x":300,"y":500,"width":400,"height":100,"file":"Notes/Judul.md","subpath":"#^abc123"},
    {"id":"n4","type":"link","x":0,"y":500,"width":200,"height":80,"url":"https://jsoncanvas.org"},
    {"id":"g1","type":"group","x":-200,"y":-50,"width":1000,"height":800,"label":"Semua","color":"#ff0000"}
  ],
  "edges":[
    {"id":"e1","fromNode":"n1","fromSide":"right","toNode":"n2","toSide":"left","label":"lihat"},
    {"id":"e2","fromNode":"n1","toNode":"n3","toEnd":"none","color":"2"},
    {"id":"e3","fromNode":"n1","toNode":"missing"}
  ]
}"##;
    let doc = CanvasDocument::from_json_canvas_str(json, "Obsidian", Some("Notes/Judul.md"), &|b| {
        (b == &BlockBinding::local("abc123")).then(|| "Blok dari markdown".to_string())
    })
    .unwrap();

    assert_eq!(doc.title, "Obsidian");
    // Nodes keep file order, edges follow; the dangling edge is skipped.
    assert_eq!(doc.elements.len(), 5 + 2, "{:#?}", doc.elements);

    match &doc.elements[0] {
        CanvasElement::StickyNote { pos, size, text, color, binding, .. } => {
            assert_eq!((*pos, *size), ([-100.0, 40.0], [250.0, 60.0]));
            assert_eq!(text, "Ide utama");
            assert_eq!(*color, parse_canvas_color("4").unwrap(), "preset 4 = green");
            assert!(binding.is_none());
        }
        other => panic!("text node → sticky, got {other:?}"),
    }
    match &doc.elements[1] {
        CanvasElement::DocCard { title, doc_type, note_id, .. } => {
            assert_eq!(title, "Paper");
            assert_eq!(doc_type, "pdf");
            assert!(note_id.is_none());
        }
        other => panic!("file node → doc card, got {other:?}"),
    }
    match &doc.elements[2] {
        CanvasElement::StickyNote { text, binding, .. } => {
            assert_eq!(text, "Blok dari markdown");
            assert_eq!(*binding, Some(BlockBinding::local("abc123")), "owner note → local binding");
        }
        other => panic!("file node with block subpath → bound sticky, got {other:?}"),
    }
    assert_eq!(doc.elements[3].text(), Some("https://jsoncanvas.org"));
    match &doc.elements[4] {
        CanvasElement::Frame { rect, title, color, .. } => {
            assert_eq!(*rect, [-200.0, -50.0, 800.0, 750.0]);
            assert_eq!(title, "Semua");
            assert_eq!(*color, [1.0, 0.0, 0.0]);
        }
        other => panic!("group → frame, got {other:?}"),
    }
    match &doc.elements[5] {
        CanvasElement::Connector { from_elem, to_elem, from_pos, to_pos, label, arrow_end, routing, .. } => {
            assert_eq!(*from_elem, Some(doc.elements[0].id()));
            assert_eq!(*to_elem, Some(doc.elements[1].id()));
            assert_eq!(*from_pos, [150.0, 70.0], "right-center of n1");
            assert_eq!(*to_pos, [300.0, 240.0], "left-center of n2");
            assert_eq!(label, "lihat");
            assert!(*arrow_end);
            assert_eq!(*routing, ConnectorRouting::Curved, "Obsidian draws curves");
        }
        other => panic!("edge → connector, got {other:?}"),
    }
    match &doc.elements[6] {
        CanvasElement::Connector { from_pos, to_pos, arrow_end, stroke_color, .. } => {
            assert!(!*arrow_end, "toEnd none");
            assert_eq!(*stroke_color, parse_canvas_color("2").unwrap());
            // No sides given: n3 is below n1, so bottom → top.
            assert_eq!(*from_pos, [25.0, 100.0]);
            assert_eq!(*to_pos, [500.0, 500.0]);
        }
        other => panic!("edge → connector, got {other:?}"),
    }
}

#[test]
fn foreign_keys_and_float_coordinates_are_tolerated() {
    let json = r##"{"nodes":[{"id":"a","type":"text","x":10.0,"y":20.4,"width":100,"height":50,"text":"t","obsidianOnly":true}],"edges":[],"metadata":{"version":"1.0-0"}}"##;
    let jc: JsonCanvas = serde_json::from_str(json).unwrap();
    assert_eq!((jc.nodes[0].x, jc.nodes[0].y), (10, 20));
    assert_eq!(jc.nodes[0].extra["obsidianOnly"], true);
    assert_eq!(jc.extra["metadata"]["version"], "1.0-0");
    let again = serde_json::to_string(&jc).unwrap();
    assert!(again.contains("\"obsidianOnly\":true") && again.contains("\"metadata\""));

    let doc = CanvasDocument::from_json_canvas_str(json, "t", None, &|_| None).unwrap();
    assert_eq!(doc.elements.len(), 1);
    assert!(CanvasDocument::from_json_canvas_str("not json", "t", None, &|_| None).is_err());
}

#[test]
fn bound_file_without_owner_falls_back_to_title() {
    let doc = full_document();
    let json = doc.to_json_canvas_string(None);
    let jc: JsonCanvas = serde_json::from_str(&json).unwrap();
    let local = jc.nodes.iter().find(|n| n.subpath.as_deref() == Some("#^abc123")).unwrap();
    assert_eq!(local.file.as_deref(), Some("full.md"));

    // ...and reading it back without an owner still yields a local binding.
    let blocks = blocks();
    let back = CanvasDocument::from_json_canvas_str(&json, "full", None, &resolver(&blocks)).unwrap();
    assert_eq!(back.bound_block_ids(), doc.bound_block_ids());
}
