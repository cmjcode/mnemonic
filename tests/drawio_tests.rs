use mnemonic::canvas::drawio::{clean_drawio_label, parse_hex_color, to_hex_color, DrawioExporter, DrawioImporter};
use mnemonic::canvas::element::{CanvasElement, ConnectorRouting, ShapeKind};
use mnemonic::canvas::CanvasDocument;
use mnemonic::notes::Note;
use tempfile::tempdir;

#[test]
fn test_hex_color_conversion() {
    assert_eq!(parse_hex_color("#FFFFFF"), Some([1.0, 1.0, 1.0]));
    assert_eq!(parse_hex_color("#000000"), Some([0.0, 0.0, 0.0]));
    assert_eq!(parse_hex_color("d5e8d4").is_some(), true);
    assert_eq!(parse_hex_color("#fff"), Some([1.0, 1.0, 1.0]));
    assert_eq!(parse_hex_color("invalid"), None);

    let hex = to_hex_color([1.0, 0.0, 0.0]);
    assert_eq!(hex, "#FF0000");
}

#[test]
fn test_clean_drawio_label() {
    let raw = "&lt;b&gt;Hello World&lt;/b&gt;&lt;br&gt;&lt;div&gt;Line 2&lt;/div&gt;&amp;nbsp;&amp;amp;&amp;nbsp;More";
    let cleaned = clean_drawio_label(raw);
    assert!(cleaned.contains("Hello World"));
    assert!(cleaned.contains("Line 2"));
    assert!(cleaned.contains("&"));
    assert!(!cleaned.contains('<'), "tags must be stripped: {cleaned:?}");
    assert!(!cleaned.contains("&lt;"));
    assert!(!cleaned.contains("nbsp"));
}

#[test]
fn test_clean_drawio_label_unescaped_html() {
    let raw = r##"<font color="#ffffff">product_externals</font><br><p style="margin: 0px; font-variant-numeric: normal;">id&nbsp;:&nbsp;int</p>"##;
    assert_eq!(clean_drawio_label(raw), "product_externals\nid : int");
}

#[test]
fn test_parse_standard_drawio_xml() {
    let sample_xml = r#"
    <mxfile host="app.diagrams.net" version="20.0">
      <diagram id="diag1" name="Page-1">
        <mxGraphModel dx="1000" dy="600" grid="1" gridSize="10">
          <root>
            <mxCell id="0"/>
            <mxCell id="1" parent="0"/>
            <mxCell id="node_start" value="Start Node" style="rounded=1;whiteSpace=wrap;html=1;fillColor=#d5e8d4;strokeColor=#82b366;" vertex="1" parent="1">
              <mxGeometry x="100" y="50" width="140" height="60" as="geometry"/>
            </mxCell>
            <mxCell id="node_decision" value="Check Value?" style="shape=rhombus;whiteSpace=wrap;html=1;fillColor=#fff2cc;strokeColor=#d6b656;" vertex="1" parent="1">
              <mxGeometry x="100" y="180" width="140" height="80" as="geometry"/>
            </mxCell>
            <mxCell id="node_end" value="End Point" style="shape=ellipse;whiteSpace=wrap;html=1;fillColor=#f8cecc;strokeColor=#b85450;" vertex="1" parent="1">
              <mxGeometry x="320" y="190" width="100" height="60" as="geometry"/>
            </mxCell>
            <mxCell id="edge_1" value="Yes" style="edgeStyle=orthogonalEdgeStyle;rounded=0;html=1;" edge="1" parent="1" source="node_start" target="node_decision">
              <mxGeometry relative="1" as="geometry"/>
            </mxCell>
            <mxCell id="edge_2" value="Pass" style="curved=1;html=1;" edge="1" parent="1" source="node_decision" target="node_end">
              <mxGeometry relative="1" as="geometry"/>
            </mxCell>
          </root>
        </mxGraphModel>
      </diagram>
    </mxfile>
    "#;

    let doc = DrawioImporter::from_xml("Sample Diagram", sample_xml).expect("Should parse Draw.io XML successfully");
    assert_eq!(doc.title, "Sample Diagram");
    assert_eq!(doc.elements.len(), 5);

    // Verify Shapes
    let start_node = doc.elements.iter().find(|e| match e {
        CanvasElement::Shape { kind, text, .. } => *kind == ShapeKind::RoundedRect && text.contains("Start Node"),
        _ => false,
    });
    assert!(start_node.is_some(), "Start node should be parsed as RoundedRect shape");

    let decision_node = doc.elements.iter().find(|e| match e {
        CanvasElement::Shape { kind, text, .. } => *kind == ShapeKind::Diamond && text.contains("Check Value"),
        _ => false,
    });
    assert!(decision_node.is_some(), "Decision node should be parsed as Diamond shape");

    let end_node = doc.elements.iter().find(|e| match e {
        CanvasElement::Shape { kind, text, .. } => *kind == ShapeKind::Ellipse && text.contains("End Point"),
        _ => false,
    });
    assert!(end_node.is_some(), "End node should be parsed as Ellipse shape");

    // Verify Connectors
    let straight_edge = doc.elements.iter().find(|e| match e {
        CanvasElement::Connector { label, routing, .. } => label == "Yes" && *routing == ConnectorRouting::Orthogonal,
        _ => false,
    });
    assert!(straight_edge.is_some(), "Orthogonal connector should be preserved");

    let curved_edge = doc.elements.iter().find(|e| match e {
        CanvasElement::Connector { label, routing, .. } => label == "Pass" && *routing == ConnectorRouting::Curved,
        _ => false,
    });
    assert!(curved_edge.is_some(), "Curved connector should be preserved");
}

#[test]
fn test_drawio_export_and_roundtrip() {
    let mut original_doc = CanvasDocument::new("Architecture Flow");

    let shape1_id = original_doc.add_element(CanvasElement::Shape {
        id: mnemonic::canvas::CanvasElementId::new(),
        kind: ShapeKind::RoundedRect,
        rect: [50.0, 50.0, 200.0, 120.0],
        stroke_color: [0.2, 0.5, 0.9],
        stroke_width: 2.0,
        fill_color: Some([0.1, 0.2, 0.3]),
        text: "Frontend Client".to_string(),
        text_color: None,
    });

    let shape2_id = original_doc.add_element(CanvasElement::Shape {
        id: mnemonic::canvas::CanvasElementId::new(),
        kind: ShapeKind::Diamond,
        rect: [50.0, 200.0, 200.0, 300.0],
        stroke_color: [0.9, 0.6, 0.1],
        stroke_width: 2.0,
        fill_color: Some([0.3, 0.2, 0.1]),
        text: "Auth Gateway".to_string(),
        text_color: None,
    });

    original_doc.add_element(CanvasElement::Connector {
        id: mnemonic::canvas::CanvasElementId::new(),
        from_elem: Some(shape1_id),
        to_elem: Some(shape2_id),
        from_pos: [125.0, 120.0],
        to_pos: [125.0, 200.0],
        routing: ConnectorRouting::Orthogonal,
        stroke_color: [0.2, 0.5, 0.9],
        stroke_width: 2.0,
        label: "HTTPS/gRPC".to_string(),
        arrow_end: true,
        waypoints: Vec::new(),
    });

    original_doc.add_element(CanvasElement::StickyNote {
        id: mnemonic::canvas::CanvasElementId::new(),
        pos: [280.0, 60.0],
        size: [220.0, 140.0],
        text: "Note: Ensure TLS 1.3 encryption".to_string(),
        color: [1.0, 0.94, 0.55],
    });

    // 1. Export to Draw.io XML
    let exported_xml = DrawioExporter::to_xml(&original_doc);
    assert!(exported_xml.starts_with("<?xml"));
    assert!(exported_xml.contains("<mxfile"));
    assert!(exported_xml.contains("Frontend Client"));
    assert!(exported_xml.contains("Auth Gateway"));
    assert!(exported_xml.contains("HTTPS/gRPC"));
    assert!(exported_xml.contains("Ensure TLS 1.3"));

    // 2. Re-import from XML (Roundtrip)
    let reimported_doc = DrawioImporter::from_xml("Architecture Flow", &exported_xml)
        .expect("Re-imported XML should be valid");

    assert_eq!(reimported_doc.elements.len(), 4);

    let re_text = reimported_doc.extract_searchable_text();
    assert!(re_text.contains("Frontend Client"));
    assert!(re_text.contains("Auth Gateway"));
    assert!(re_text.contains("HTTPS/gRPC"));
    assert!(re_text.contains("Ensure TLS 1.3"));
}

#[test]
fn test_create_drawio_note_and_parse_body() {
    let dir = tempdir().unwrap();
    let note = Note::create_drawio(dir.path(), "System Design Diagram").unwrap();

    assert!(note.is_canvas(), "Draw.io note must be identified as canvas");
    assert!(note.body.contains("```drawio"));

    let doc = CanvasDocument::from_markdown_body("System Design Diagram", &note.body);
    assert!(doc.elements.len() >= 3, "Starter elements should be present in canvas");
}

fn shape_by_text<'a>(doc: &'a CanvasDocument, needle: &str) -> &'a CanvasElement {
    doc.elements
        .iter()
        .find(|e| match e {
            CanvasElement::Shape { text, .. } => text == needle,
            CanvasElement::Frame { title, .. } => title == needle,
            _ => false,
        })
        .unwrap_or_else(|| panic!("element {needle:?} not found in {:#?}", doc.elements))
}

fn connectors(doc: &CanvasDocument) -> Vec<&CanvasElement> {
    doc.elements
        .iter()
        .filter(|e| matches!(e, CanvasElement::Connector { .. }))
        .collect()
}

fn wrap_model(cells: &str) -> String {
    format!(
        r#"<mxfile><diagram name="Page-1"><mxGraphModel><root><mxCell id="0"/><mxCell id="1" parent="0"/>{cells}</root></mxGraphModel></diagram></mxfile>"#
    )
}

#[test]
fn test_import_html_label_attribute_is_clean() {
    let xml = wrap_model(
        r##"<mxCell id="t" value="&lt;font color=&quot;#ffffff&quot;&gt;sales_order&lt;/font&gt;&lt;br&gt;id&amp;nbsp;int" style="rounded=0;whiteSpace=wrap;html=1;fontColor=#FFFFFF;fillColor=#1ba1e2;" vertex="1" parent="1"><mxGeometry x="0" y="0" width="120" height="60" as="geometry"/></mxCell>"##,
    );
    let doc = DrawioImporter::from_xml("t", &xml).unwrap();
    match shape_by_text(&doc, "sales_order\nid int") {
        CanvasElement::Shape { text_color, .. } => assert_eq!(*text_color, Some([1.0, 1.0, 1.0])),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn test_import_non_html_label_keeps_angle_brackets() {
    let xml = wrap_model(
        r#"<mxCell id="t" value="a &lt; b" style="rounded=0;" vertex="1" parent="1"><mxGeometry width="80" height="40" as="geometry"/></mxCell>"#,
    );
    let doc = DrawioImporter::from_xml("t", &xml).unwrap();
    shape_by_text(&doc, "a < b");
}

#[test]
fn test_import_child_geometry_is_relative_to_parent() {
    let xml = wrap_model(
        r#"
        <mxCell id="lane" value="Orders" style="swimlane;html=1;" vertex="1" parent="1"><mxGeometry x="500" y="300" width="300" height="200" as="geometry"/></mxCell>
        <mxCell id="grp" style="group" vertex="1" connectable="0" parent="lane"><mxGeometry x="20" y="40" width="200" height="100" as="geometry"/></mxCell>
        <mxCell id="row" value="order_id" style="text;html=1;" vertex="1" parent="grp"><mxGeometry x="10" y="5" width="100" height="30" as="geometry"/></mxCell>
        "#,
    );
    let doc = DrawioImporter::from_xml("t", &xml).unwrap();

    match shape_by_text(&doc, "Orders") {
        CanvasElement::Frame { rect, .. } => assert_eq!(*rect, [500.0, 300.0, 800.0, 500.0]),
        other => panic!("swimlane should become a frame, got {other:?}"),
    }
    match shape_by_text(&doc, "order_id") {
        CanvasElement::Shape { rect, fill_color, stroke_width, .. } => {
            assert_eq!(*rect, [530.0, 345.0, 630.0, 375.0], "lane + group + own offset");
            assert_eq!(*fill_color, None, "text cells are transparent");
            assert_eq!(*stroke_width, 0.0, "text cells are borderless");
        }
        other => panic!("unexpected {other:?}"),
    }
    // The invisible group itself is not rendered.
    assert_eq!(doc.elements.len(), 2);
}

#[test]
fn test_import_edge_label_child_becomes_connector_label() {
    let xml = wrap_model(
        r#"
        <mxCell id="a" value="A" style="rounded=0;" vertex="1" parent="1"><mxGeometry x="0" y="0" width="100" height="50" as="geometry"/></mxCell>
        <mxCell id="b" value="B" style="rounded=0;" vertex="1" parent="1"><mxGeometry x="300" y="0" width="100" height="50" as="geometry"/></mxCell>
        <mxCell id="e" style="edgeStyle=orthogonalEdgeStyle;html=1;" edge="1" parent="1" source="a" target="b"><mxGeometry relative="1" as="geometry"/></mxCell>
        <mxCell id="lbl" value="1..*" style="edgeLabel;html=1;align=center;" vertex="1" connectable="0" parent="e"><mxGeometry x="-0.2" relative="1" as="geometry"><mxPoint as="offset"/></mxGeometry></mxCell>
        "#,
    );
    let doc = DrawioImporter::from_xml("t", &xml).unwrap();
    assert_eq!(doc.elements.len(), 3, "edge label must not become a shape: {:#?}", doc.elements);
    match connectors(&doc)[0] {
        CanvasElement::Connector { label, from_pos, to_pos, from_elem, to_elem, .. } => {
            assert_eq!(label, "1..*");
            assert_eq!(*from_pos, [100.0, 25.0], "leaves from A's right side");
            assert_eq!(*to_pos, [300.0, 25.0], "enters B's left side");
            assert!(from_elem.is_some() && to_elem.is_some());
        }
        _ => unreachable!(),
    }
}

#[test]
fn test_import_object_wrapper_keeps_id_label_and_geometry() {
    let xml = wrap_model(
        r#"
        <object label="Service" id="svc" owner="team"><mxCell style="rounded=1;html=1;" vertex="1" parent="1"><mxGeometry x="40" y="40" width="120" height="60" as="geometry"/></mxCell></object>
        <UserObject label="DB" id="db"><mxCell style="shape=cylinder3;" vertex="1" parent="1"><mxGeometry x="40" y="200" width="120" height="60" as="geometry"/></mxCell></UserObject>
        <mxCell id="e" edge="1" parent="1" source="svc" target="db" style="html=1;"><mxGeometry relative="1" as="geometry"/></mxCell>
        "#,
    );
    let doc = DrawioImporter::from_xml("t", &xml).unwrap();
    match shape_by_text(&doc, "Service") {
        CanvasElement::Shape { rect, kind, .. } => {
            assert_eq!(*rect, [40.0, 40.0, 160.0, 100.0]);
            assert_eq!(*kind, ShapeKind::RoundedRect);
        }
        _ => unreachable!(),
    }
    shape_by_text(&doc, "DB");
    match connectors(&doc)[0] {
        CanvasElement::Connector { from_pos, to_pos, .. } => {
            assert_eq!(*from_pos, [100.0, 100.0], "bottom of Service");
            assert_eq!(*to_pos, [100.0, 200.0], "top of DB");
        }
        _ => unreachable!(),
    }
}

#[test]
fn test_import_waypoints_are_absolute_and_edges_without_endpoints_skipped() {
    let xml = wrap_model(
        r#"
        <mxCell id="box" style="rounded=0;" vertex="1" parent="1"><mxGeometry x="100" y="100" width="400" height="300" as="geometry"/></mxCell>
        <mxCell id="a" value="A" vertex="1" parent="box"><mxGeometry x="10" y="10" width="50" height="50" as="geometry"/></mxCell>
        <mxCell id="b" value="B" vertex="1" parent="box"><mxGeometry x="200" y="200" width="50" height="50" as="geometry"/></mxCell>
        <mxCell id="e" style="edgeStyle=orthogonalEdgeStyle;" edge="1" parent="box" source="a" target="b"><mxGeometry relative="1" as="geometry"><Array as="points"><mxPoint x="135" y="35"/></Array></mxGeometry></mxCell>
        <mxCell id="dangling" edge="1" parent="1" source="missing"><mxGeometry relative="1" as="geometry"/></mxCell>
        "#,
    );
    let (doc, report) = DrawioImporter::from_xml_with_report("t", &xml).unwrap();
    assert_eq!(report.skipped, 1);
    let conns = connectors(&doc);
    assert_eq!(conns.len(), 1);
    match conns[0] {
        CanvasElement::Connector { waypoints, from_pos, to_pos, .. } => {
            assert_eq!(waypoints, &vec![[235.0, 135.0]]);
            assert_eq!(*from_pos, [160.0, 135.0], "A right side, level with the waypoint");
            assert_eq!(*to_pos, [325.0, 300.0], "B top side, under the waypoint");
        }
        _ => unreachable!(),
    }
}

#[test]
fn test_import_multi_page_does_not_overlap() {
    let cell = |name: &str| {
        format!(
            r#"<diagram name="{name}"><mxGraphModel><root><mxCell id="0"/><mxCell id="1" parent="0"/><mxCell id="n" value="{name} node" vertex="1" parent="1"><mxGeometry x="0" y="0" width="200" height="100" as="geometry"/></mxCell></root></mxGraphModel></diagram>"#
        )
    };
    let xml = format!("<mxfile>{}{}</mxfile>", cell("First"), cell("Second"));
    let (doc, report) = DrawioImporter::from_xml_with_report("t", &xml).unwrap();
    assert_eq!(report.pages, 2);

    let first = shape_by_text(&doc, "First node").bounding_rect();
    let second = shape_by_text(&doc, "Second node").bounding_rect();
    assert!(!first.intersects(second), "pages overlap: {first:?} vs {second:?}");
    let first_frame = shape_by_text(&doc, "First").bounding_rect();
    let second_frame = shape_by_text(&doc, "Second").bounding_rect();
    assert!(first_frame.contains_rect(first));
    assert!(second_frame.contains_rect(second));
    assert!(!first_frame.intersects(second_frame));
}

#[test]
fn test_import_compressed_diagram() {
    let xml = r#"<mxfile host="Electron"><diagram id="x" name="Page-1">jVHRDsIgDPyavjNI/ABR96T/QEIzlsBYoNPt78XRufiwxAeS6/V6uRZQOsxtMqO7R4se1BWUTjFSRWHW6D1I0VtQF5BSlAfydtBt1q4YTcKB/hkwdeBp/ISV0TGMCXNGW/hHyVQVmRbPihSnweLHoAF1dhQ8Q/bCRDgf5lkpDtNiDEhpKZJtoMYVSy05vXj1lhwrNs5h3zk2PTFncq27r/G+ewG8/lbuZ157P7/wBg==</diagram></mxfile>"#;
    let doc = DrawioImporter::from_xml("t", xml).unwrap();
    match shape_by_text(&doc, "Compressed Node") {
        CanvasElement::Shape { rect, .. } => assert_eq!(*rect, [10.0, 20.0, 130.0, 80.0]),
        _ => unreachable!(),
    }
}

#[test]
fn test_import_corrupt_compressed_diagram_is_an_error() {
    let xml = r#"<mxfile><diagram name="Broken">not-base64-@@@</diagram></mxfile>"#;
    let err = DrawioImporter::from_xml("t", xml).unwrap_err();
    assert!(format!("{err:#}").contains("Broken"));
}

#[test]
fn test_export_roundtrip_keeps_waypoints_text_color_and_special_chars() {
    let mut doc = CanvasDocument::new("rt");
    let a = doc.add_element(CanvasElement::Shape {
        id: mnemonic::canvas::CanvasElementId::new(),
        kind: ShapeKind::Rectangle,
        rect: [0.0, 0.0, 100.0, 50.0],
        stroke_color: [0.0, 0.0, 0.0],
        stroke_width: 1.0,
        fill_color: Some([0.0, 0.0, 0.0]),
        text: "x < y & z\nline 2".to_string(),
        text_color: Some([1.0, 0.0, 0.0]),
    });
    let b = doc.add_element(CanvasElement::Shape {
        id: mnemonic::canvas::CanvasElementId::new(),
        kind: ShapeKind::Rectangle,
        rect: [300.0, 300.0, 400.0, 350.0],
        stroke_color: [0.0, 0.0, 0.0],
        stroke_width: 1.0,
        fill_color: None,
        text: "<b>".to_string(),
        text_color: None,
    });
    doc.add_element(CanvasElement::Connector {
        id: mnemonic::canvas::CanvasElementId::new(),
        from_elem: Some(a),
        to_elem: Some(b),
        from_pos: [100.0, 25.0],
        to_pos: [350.0, 300.0],
        routing: ConnectorRouting::Orthogonal,
        stroke_color: [0.0, 0.0, 0.0],
        stroke_width: 1.0,
        label: String::new(),
        arrow_end: true,
        waypoints: vec![[350.0, 25.0]],
    });

    let back = DrawioImporter::from_xml("rt", &DrawioExporter::to_xml(&doc)).unwrap();
    match shape_by_text(&back, "x < y & z\nline 2") {
        CanvasElement::Shape { text_color, .. } => assert_eq!(*text_color, Some([1.0, 0.0, 0.0])),
        _ => unreachable!(),
    }
    shape_by_text(&back, "<b>");
    match connectors(&back)[0] {
        CanvasElement::Connector { waypoints, from_pos, to_pos, .. } => {
            assert_eq!(waypoints, &vec![[350.0, 25.0]]);
            assert_eq!(*from_pos, [100.0, 25.0]);
            assert_eq!(*to_pos, [350.0, 300.0]);
        }
        _ => unreachable!(),
    }
}

#[test]
fn test_legacy_canvas_json_without_new_fields_still_loads() {
    let json = r#"{"id":"6f1c0f6e-3a55-4f7a-9d55-1b2f7c7c1a11","title":"old","viewport":{"pan":[0.0,0.0],"zoom":1.0},"elements":[
        {"Shape":{"id":"0b7e7d8e-1111-4a4a-9b9b-222222222222","kind":"Rectangle","rect":[0.0,0.0,10.0,10.0],"stroke_color":[0.0,0.0,0.0],"stroke_width":1.0,"fill_color":null,"text":"s"}},
        {"Connector":{"id":"0b7e7d8e-3333-4a4a-9b9b-222222222222","from_elem":null,"to_elem":null,"from_pos":[0.0,0.0],"to_pos":[5.0,5.0],"routing":"Straight","stroke_color":[0.0,0.0,0.0],"stroke_width":1.0,"label":"","arrow_end":true}}
    ]}"#;
    let doc: CanvasDocument = serde_json::from_str(json).expect("legacy canvas JSON must deserialize");
    assert_eq!(doc.elements.len(), 2);
}

#[test]
fn test_import_erd_table_shapes() {
    let xml = wrap_model(
        r#"
        <mxCell id="t" value="users" style="shape=table;startSize=30;container=1;childLayout=tableLayout;html=1;" vertex="1" parent="1"><mxGeometry x="200" y="100" width="180" height="60" as="geometry"/></mxCell>
        <mxCell id="r1" value="" style="shape=tableRow;fillColor=none;top=0;left=0;right=0;bottom=1;html=1;" vertex="1" parent="t"><mxGeometry y="30" width="180" height="30" as="geometry"/></mxCell>
        <mxCell id="c1" value="PK" style="shape=partialRectangle;connectable=0;fillColor=none;top=0;left=0;bottom=0;right=0;html=1;" vertex="1" parent="r1"><mxGeometry width="30" height="30" as="geometry"><mxRectangle width="30" height="30" as="alternateBounds"/></mxGeometry></mxCell>
        <mxCell id="s" value="orders" style="swimlane;childLayout=stackLayout;startSize=26;fillColor=#dae8fc;html=1;" vertex="1" parent="1"><mxGeometry x="500" y="100" width="160" height="78" as="geometry"/></mxCell>
        <mxCell id="s1" value="+ id: int" style="text;strokeColor=none;fillColor=none;align=left;html=1;" vertex="1" parent="s"><mxGeometry y="26" width="160" height="26" as="geometry"/></mxCell>
        <mxCell id="e" style="edgeStyle=entityRelationEdgeStyle;html=1;endArrow=ERoneToMany;" edge="1" parent="1" source="r1" target="s1"><mxGeometry relative="1" as="geometry"/></mxCell>
        "#,
    );
    let doc = DrawioImporter::from_xml("erd", &xml).unwrap();

    assert!(matches!(shape_by_text(&doc, "users"), CanvasElement::Frame { .. }), "table header is a frame");
    match shape_by_text(&doc, "PK") {
        CanvasElement::Shape { rect, stroke_width, .. } => {
            assert_eq!(*rect, [200.0, 130.0, 230.0, 160.0], "table + row + cell offsets");
            assert_eq!(*stroke_width, 0.0, "partial rectangles are borderless");
        }
        _ => unreachable!(),
    }
    // Empty table row is layout-only: not rendered, but still an edge endpoint.
    assert_eq!(doc.elements.len(), 5, "{:#?}", doc.elements);
    match connectors(&doc)[0] {
        CanvasElement::Connector { from_pos, to_pos, routing, .. } => {
            assert_eq!(*from_pos, [380.0, 139.0]);
            assert_eq!(*to_pos, [500.0, 145.0]);
            assert_eq!(*routing, ConnectorRouting::Orthogonal);
        }
        _ => unreachable!(),
    }
}
