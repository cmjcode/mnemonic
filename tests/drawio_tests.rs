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
    assert!(!cleaned.contains("&lt;"));
    assert!(!cleaned.contains("&gt;"));
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
    });

    let shape2_id = original_doc.add_element(CanvasElement::Shape {
        id: mnemonic::canvas::CanvasElementId::new(),
        kind: ShapeKind::Diamond,
        rect: [50.0, 200.0, 200.0, 300.0],
        stroke_color: [0.9, 0.6, 0.1],
        stroke_width: 2.0,
        fill_color: Some([0.3, 0.2, 0.1]),
        text: "Auth Gateway".to_string(),
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
