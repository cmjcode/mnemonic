//! Two-way Draw.io (diagrams.net) XML Importer & Exporter for MNEMONIC Canvas.
//!
//! Allows importing `.drawio` files and `mxGraphModel` XML into native `CanvasDocument`
//! elements, and exporting `CanvasDocument` whiteboards back to standard Draw.io XML
//! format fully compatible with https://app.diagrams.net and Draw.io Desktop.

use std::collections::HashMap;
use anyhow::Result;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::element::{CanvasElement, CanvasElementId, ConnectorRouting, ShapeKind};
use super::tools;
use super::CanvasDocument;

/// Helper to parse hex colors like `#d5e8d4`, `#D5E8D4`, or `d5e8d4` into RGB `[f32; 3]`.
pub fn parse_hex_color(hex: &str) -> Option<[f32; 3]> {
    let s = hex.trim().trim_start_matches('#');
    if s.len() == 6 {
        let r = u8::from_str_radix(&s[0..2], 16).ok()? as f32 / 255.0;
        let g = u8::from_str_radix(&s[2..4], 16).ok()? as f32 / 255.0;
        let b = u8::from_str_radix(&s[4..6], 16).ok()? as f32 / 255.0;
        Some([r, g, b])
    } else if s.len() == 3 {
        let r_ch = s.chars().nth(0)?;
        let g_ch = s.chars().nth(1)?;
        let b_ch = s.chars().nth(2)?;
        let r = u8::from_str_radix(&format!("{}{}", r_ch, r_ch), 16).ok()? as f32 / 255.0;
        let g = u8::from_str_radix(&format!("{}{}", g_ch, g_ch), 16).ok()? as f32 / 255.0;
        let b = u8::from_str_radix(&format!("{}{}", b_ch, b_ch), 16).ok()? as f32 / 255.0;
        Some([r, g, b])
    } else {
        None
    }
}

/// Convert RGB `[f32; 3]` into a hex string `#RRGGBB`.
pub fn to_hex_color(rgb: [f32; 3]) -> String {
    let r = (rgb[0].clamp(0.0, 1.0) * 255.0).round() as u8;
    let g = (rgb[1].clamp(0.0, 1.0) * 255.0).round() as u8;
    let b = (rgb[2].clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02X}{:02X}{:02X}", r, g, b)
}

/// Strip basic HTML tags and unescape XML/HTML entities from Draw.io label text.
pub fn clean_drawio_label(raw: &str) -> String {
    let mut text = raw.to_string();

    // Replace common line breaks and paragraphs
    text = text.replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("</div><div>", "\n")
        .replace("</p><p>", "\n");

    // Remove remaining XML / HTML tags
    let mut cleaned = String::with_capacity(text.len());
    let mut inside_tag = false;
    for ch in text.chars() {
        if ch == '<' {
            inside_tag = true;
        } else if ch == '>' {
            inside_tag = false;
        } else if !inside_tag {
            cleaned.push(ch);
        }
    }

    // Unescape XML entities
    let unescaped = cleaned
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&#10;", "\n")
        .replace("&#13;", "");

    unescaped.trim().to_string()
}

/// Escape text for XML attribute / content.
pub fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
        .replace('\n', "&#10;")
}

/// Raw parsed cell from Draw.io XML
#[derive(Debug, Clone, Default)]
struct RawCell {
    id: String,
    parent: Option<String>,
    value: String,
    style: String,
    vertex: bool,
    edge: bool,
    source: Option<String>,
    target: Option<String>,
    // Geometry
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    has_geometry: bool,
    source_point: Option<[f32; 2]>,
    target_point: Option<[f32; 2]>,
}

/// Style map parsed from Draw.io style string (e.g. `rounded=1;fillColor=#d5e8d4;shape=rhombus;`)
#[derive(Debug, Clone, Default)]
struct ParsedStyle {
    shape_type: Option<String>,
    is_rounded: bool,
    fill_color: Option<[f32; 3]>,
    has_no_fill: bool,
    stroke_color: Option<[f32; 3]>,
    stroke_width: f32,
    edge_style: Option<String>,
    end_arrow: Option<String>,
    start_arrow: Option<String>,
    is_curved: bool,
    is_swimlane: bool,
    is_note: bool,
}

fn parse_style_string(style_str: &str) -> ParsedStyle {
    let mut style = ParsedStyle {
        stroke_width: 1.5,
        ..Default::default()
    };

    for part in style_str.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        let mut kv = part.splitn(2, '=');
        let key = kv.next().unwrap_or("").trim();
        let val = kv.next().unwrap_or("").trim();

        match key {
            "rounded" => {
                if val == "1" || val == "true" {
                    style.is_rounded = true;
                }
            }
            "curved" => {
                if val == "1" || val == "true" {
                    style.is_curved = true;
                }
            }
            "shape" => {
                style.shape_type = Some(val.to_lowercase());
            }
            "fillColor" => {
                if val.eq_ignore_ascii_case("none") {
                    style.has_no_fill = true;
                } else if let Some(rgb) = parse_hex_color(val) {
                    style.fill_color = Some(rgb);
                }
            }
            "strokeColor" => {
                if val.eq_ignore_ascii_case("none") {
                    style.stroke_color = None;
                } else if let Some(rgb) = parse_hex_color(val) {
                    style.stroke_color = Some(rgb);
                }
            }
            "strokeWidth" => {
                if let Ok(w) = val.parse::<f32>() {
                    style.stroke_width = w.clamp(0.5, 20.0);
                }
            }
            "edgeStyle" => {
                style.edge_style = Some(val.to_string());
            }
            "endArrow" => {
                style.end_arrow = Some(val.to_string());
            }
            "startArrow" => {
                style.start_arrow = Some(val.to_string());
            }
            "swimlane" => {
                style.is_swimlane = true;
            }
            "note" => {
                style.is_note = true;
            }
            "ellipse" => {
                style.shape_type = Some("ellipse".to_string());
            }
            "rhombus" | "diamond" => {
                style.shape_type = Some("rhombus".to_string());
            }
            _ => {}
        }
    }

    style
}

/// Importer to load Draw.io XML into a `CanvasDocument`.
pub struct DrawioImporter;

impl DrawioImporter {
    /// Parse a Draw.io XML string (or ` ```drawio ... ``` ` markdown block) into a `CanvasDocument`.
    pub fn from_xml(title: &str, xml_content: &str) -> Result<CanvasDocument> {
        let content = xml_content.trim();
        // Unwrap markdown code block if present
        let xml_str = if let Some(start) = content.find("```drawio") {
            let rest = &content[start + 9..];
            if let Some(end) = rest.find("```") {
                rest[..end].trim()
            } else {
                rest.trim()
            }
        } else if let Some(start) = content.find("```xml") {
            let rest = &content[start + 6..];
            if let Some(end) = rest.find("```") {
                rest[..end].trim()
            } else {
                rest.trim()
            }
        } else {
            content
        };

        let mut reader = Reader::from_str(xml_str);
        reader.config_mut().trim_text(true);

        let mut raw_cells: Vec<RawCell> = Vec::new();
        let mut current_cell: Option<RawCell> = None;

        let mut buf = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                    let name_bytes = e.name();
                    let tag_name = String::from_utf8_lossy(name_bytes.as_ref()).to_string();

                    match tag_name.as_str() {
                        "mxCell" | "object" => {
                            let mut cell = RawCell::default();
                            Self::parse_cell_attributes(e, &mut cell);

                            // If this is a nested or existing cell, push previous if finished
                            if let Some(prev) = current_cell.take() {
                                raw_cells.push(prev);
                            }
                            current_cell = Some(cell);
                        }
                        "mxGeometry" => {
                            if let Some(ref mut cell) = current_cell {
                                cell.has_geometry = true;
                                Self::parse_geometry_attributes(e, cell);
                            }
                        }
                        "mxPoint" => {
                            if let Some(ref mut cell) = current_cell {
                                Self::parse_point_attributes(e, cell);
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::End(ref e)) => {
                    let name_bytes = e.name();
                    let tag_name = String::from_utf8_lossy(name_bytes.as_ref());
                    if tag_name == "mxCell" || tag_name == "object" {
                        if let Some(cell) = current_cell.take() {
                            raw_cells.push(cell);
                        }
                    }
                }
                Ok(Event::Eof) => {
                    if let Some(cell) = current_cell.take() {
                        raw_cells.push(cell);
                    }
                    break;
                }
                Err(err) => {
                    return Err(anyhow::anyhow!("Error parsing Draw.io XML: {:?}", err));
                }
                _ => {}
            }
            buf.clear();
        }

        // Convert raw cells to CanvasDocument
        Self::build_canvas_document(title, raw_cells)
    }

    fn parse_cell_attributes(e: &BytesStart, cell: &mut RawCell) {
        for attr in e.attributes().flatten() {
            let key = String::from_utf8_lossy(attr.key.as_ref());
            let val = String::from_utf8_lossy(&attr.value);

            match key.as_ref() {
                "id" => cell.id = val.to_string(),
                "parent" => cell.parent = Some(val.to_string()),
                "value" | "label" => cell.value = clean_drawio_label(&val),
                "style" => cell.style = val.to_string(),
                "vertex" => cell.vertex = val == "1" || val == "true",
                "edge" => cell.edge = val == "1" || val == "true",
                "source" => cell.source = Some(val.to_string()),
                "target" => cell.target = Some(val.to_string()),
                _ => {}
            }
        }
    }

    fn parse_geometry_attributes(e: &BytesStart, cell: &mut RawCell) {
        for attr in e.attributes().flatten() {
            let key = String::from_utf8_lossy(attr.key.as_ref());
            let val = String::from_utf8_lossy(&attr.value);

            match key.as_ref() {
                "x" => cell.x = val.parse::<f32>().unwrap_or(0.0),
                "y" => cell.y = val.parse::<f32>().unwrap_or(0.0),
                "width" => cell.width = val.parse::<f32>().unwrap_or(120.0),
                "height" => cell.height = val.parse::<f32>().unwrap_or(60.0),
                _ => {}
            }
        }
    }

    fn parse_point_attributes(e: &BytesStart, cell: &mut RawCell) {
        let mut point_type = String::new();
        let mut px = 0.0f32;
        let mut py = 0.0f32;

        for attr in e.attributes().flatten() {
            let key = String::from_utf8_lossy(attr.key.as_ref());
            let val = String::from_utf8_lossy(&attr.value);

            match key.as_ref() {
                "as" => point_type = val.to_string(),
                "x" => px = val.parse::<f32>().unwrap_or(0.0),
                "y" => py = val.parse::<f32>().unwrap_or(0.0),
                _ => {}
            }
        }

        if point_type == "sourcePoint" {
            cell.source_point = Some([px, py]);
        } else if point_type == "targetPoint" {
            cell.target_point = Some([px, py]);
        }
    }

    fn build_canvas_document(title: &str, raw_cells: Vec<RawCell>) -> Result<CanvasDocument> {
        let mut doc = CanvasDocument::new(title);

        // Map from Draw.io cell ID -> CanvasElementId and center position
        let mut id_map: HashMap<String, (CanvasElementId, [f32; 2])> = HashMap::new();

        // 1. Process vertices first (Shapes, StickyNotes, Frames)
        for cell in raw_cells.iter().filter(|c| c.vertex || (!c.edge && c.has_geometry)) {
            // Skip root/layer cells with id "0" or "1" if they have no geometry
            if (cell.id == "0" || cell.id == "1") && !c_has_valid_size(cell) {
                continue;
            }

            let style = parse_style_string(&cell.style);
            let elem_id = CanvasElementId::new();
            let width = if cell.width > 0.0 { cell.width } else { 120.0 };
            let height = if cell.height > 0.0 { cell.height } else { 60.0 };
            let rect = [cell.x, cell.y, cell.x + width, cell.y + height];
            let center = [cell.x + width * 0.5, cell.y + height * 0.5];

            // Determine element type
            let element = if style.is_swimlane || style.shape_type.as_deref() == Some("swimlane") {
                CanvasElement::Frame {
                    id: elem_id,
                    rect,
                    title: if cell.value.is_empty() { "Frame".to_string() } else { cell.value.clone() },
                    color: style.stroke_color.unwrap_or(tools::PALETTE_PRIMARY_ACCENT),
                }
            } else if style.is_note || style.shape_type.as_deref() == Some("note") {
                CanvasElement::StickyNote {
                    id: elem_id,
                    pos: [cell.x, cell.y],
                    size: [width, height],
                    text: cell.value.clone(),
                    color: style.fill_color.unwrap_or(tools::PALETTE_STICKY_YELLOW),
                }
            } else {
                // Geometric shape
                let kind = match style.shape_type.as_deref() {
                    Some("ellipse") => ShapeKind::Ellipse,
                    Some("rhombus") | Some("diamond") => ShapeKind::Diamond,
                    Some("cloud") | Some("callout") | Some("speech") => ShapeKind::CalloutBubble,
                    _ => {
                        if style.is_rounded {
                            ShapeKind::RoundedRect
                        } else {
                            ShapeKind::Rectangle
                        }
                    }
                };

                let stroke_color = style.stroke_color.unwrap_or(tools::PALETTE_STROKE_LIGHT);
                let fill_color = if style.has_no_fill {
                    None
                } else {
                    style.fill_color.or(Some([0.18, 0.22, 0.32]))
                };

                CanvasElement::Shape {
                    id: elem_id,
                    kind,
                    rect,
                    stroke_color,
                    stroke_width: style.stroke_width,
                    fill_color,
                    text: cell.value.clone(),
                }
            };

            if !cell.id.is_empty() {
                id_map.insert(cell.id.clone(), (elem_id, center));
            }
            doc.add_element(element);
        }

        // 2. Process edges (Connectors)
        for cell in raw_cells.iter().filter(|c| c.edge) {
            let style = parse_style_string(&cell.style);
            let elem_id = CanvasElementId::new();

            let from_elem_entry = cell.source.as_ref().and_then(|s| id_map.get(s));
            let to_elem_entry = cell.target.as_ref().and_then(|t| id_map.get(t));

            let from_elem = from_elem_entry.map(|e| e.0);
            let to_elem = to_elem_entry.map(|e| e.0);

            let from_pos = cell.source_point
                .or_else(|| from_elem_entry.map(|e| e.1))
                .unwrap_or([cell.x, cell.y]);

            let to_pos = cell.target_point
                .or_else(|| to_elem_entry.map(|e| e.1))
                .unwrap_or([cell.x + cell.width, cell.y + cell.height]);

            let routing = match style.edge_style.as_deref() {
                Some("orthogonalEdgeStyle") | Some("elbowEdgeStyle") => ConnectorRouting::Orthogonal,
                _ => {
                    if style.is_curved {
                        ConnectorRouting::Curved
                    } else {
                        ConnectorRouting::Straight
                    }
                }
            };

            let arrow_end = match style.end_arrow.as_deref() {
                Some("none") => false,
                _ => true,
            };

            let connector = CanvasElement::Connector {
                id: elem_id,
                from_elem,
                to_elem,
                from_pos,
                to_pos,
                routing,
                stroke_color: style.stroke_color.unwrap_or(tools::PALETTE_PRIMARY_ACCENT),
                stroke_width: style.stroke_width,
                label: cell.value.clone(),
                arrow_end,
            };

            doc.add_element(connector);
        }

        Ok(doc)
    }
}

fn c_has_valid_size(cell: &RawCell) -> bool {
    cell.width > 0.0 || cell.height > 0.0 || !cell.value.is_empty()
}

/// Exporter to generate standard Draw.io XML from a `CanvasDocument`.
pub struct DrawioExporter;

impl DrawioExporter {
    /// Export a `CanvasDocument` to a standard `.drawio` XML string.
    pub fn to_xml(doc: &CanvasDocument) -> String {
        let mut xml = String::with_capacity(4096);

        let safe_title = escape_xml(&doc.title);
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str("<mxfile host=\"mnemonic\" modified=\"2026-09-02T00:00:00Z\" agent=\"Mnemonic Native Edgeless Canvas\" version=\"1.0\" type=\"device\">\n");
        xml.push_str(&format!("  <diagram id=\"{}\" name=\"{}\">\n", doc.id, safe_title));
        xml.push_str("    <mxGraphModel dx=\"1200\" dy=\"800\" grid=\"1\" gridSize=\"10\" guides=\"1\" tooltips=\"1\" connect=\"1\" arrows=\"1\" fold=\"1\" page=\"1\" pageScale=\"1\" pageWidth=\"1169\" pageHeight=\"827\" math=\"0\" shadow=\"0\">\n");
        xml.push_str("      <root>\n");
        xml.push_str("        <mxCell id=\"0\"/>\n");
        xml.push_str("        <mxCell id=\"1\" parent=\"0\"/>\n");

        // Map CanvasElementId -> string ID for edge references
        let mut id_map: HashMap<CanvasElementId, String> = HashMap::new();
        let mut cell_counter = 2usize;

        // 1. Export Shapes, StickyNotes, Frames, DocCards
        for elem in &doc.elements {
            let elem_id = elem.id();
            let cell_id = format!("cell_{}", cell_counter);
            cell_counter += 1;
            id_map.insert(elem_id, cell_id.clone());

            match elem {
                CanvasElement::Shape {
                    kind,
                    rect,
                    stroke_color,
                    stroke_width,
                    fill_color,
                    text,
                    ..
                } => {
                    let w = (rect[2] - rect[0]).max(10.0);
                    let h = (rect[3] - rect[1]).max(10.0);
                    let x = rect[0];
                    let y = rect[1];

                    let shape_name = match kind {
                        ShapeKind::Rectangle => "rounded=0",
                        ShapeKind::RoundedRect => "rounded=1",
                        ShapeKind::Ellipse => "ellipse;whiteSpace=wrap;html=1",
                        ShapeKind::Diamond => "rhombus;whiteSpace=wrap;html=1",
                        ShapeKind::CalloutBubble => "shape=callout;whiteSpace=wrap;html=1",
                    };

                    let stroke_hex = to_hex_color(*stroke_color);
                    let fill_hex = fill_color.map(to_hex_color).unwrap_or_else(|| "none".to_string());

                    let style = format!(
                        "{};whiteSpace=wrap;html=1;strokeColor={};fillColor={};strokeWidth={};",
                        shape_name, stroke_hex, fill_hex, stroke_width
                    );

                    xml.push_str(&format!(
                        "        <mxCell id=\"{}\" value=\"{}\" style=\"{}\" vertex=\"1\" parent=\"1\">\n",
                        cell_id, escape_xml(text), style
                    ));
                    xml.push_str(&format!(
                        "          <mxGeometry x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" as=\"geometry\"/>\n",
                        x, y, w, h
                    ));
                    xml.push_str("        </mxCell>\n");
                }
                CanvasElement::StickyNote {
                    pos,
                    size,
                    text,
                    color,
                    ..
                } => {
                    let fill_hex = to_hex_color(*color);
                    let style = format!(
                        "shape=note;whiteSpace=wrap;html=1;size=14;fillColor={};strokeColor=#b8860b;",
                        fill_hex
                    );

                    xml.push_str(&format!(
                        "        <mxCell id=\"{}\" value=\"{}\" style=\"{}\" vertex=\"1\" parent=\"1\">\n",
                        cell_id, escape_xml(text), style
                    ));
                    xml.push_str(&format!(
                        "          <mxGeometry x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" as=\"geometry\"/>\n",
                        pos[0], pos[1], size[0], size[1]
                    ));
                    xml.push_str("        </mxCell>\n");
                }
                CanvasElement::Frame {
                    rect,
                    title,
                    color,
                    ..
                } => {
                    let w = (rect[2] - rect[0]).max(20.0);
                    let h = (rect[3] - rect[1]).max(20.0);
                    let color_hex = to_hex_color(*color);
                    let style = format!(
                        "swimlane;startSize=24;whiteSpace=wrap;html=1;strokeColor={};fillColor=none;",
                        color_hex
                    );

                    xml.push_str(&format!(
                        "        <mxCell id=\"{}\" value=\"{}\" style=\"{}\" vertex=\"1\" parent=\"1\">\n",
                        cell_id, escape_xml(title), style
                    ));
                    xml.push_str(&format!(
                        "          <mxGeometry x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" as=\"geometry\"/>\n",
                        rect[0], rect[1], w, h
                    ));
                    xml.push_str("        </mxCell>\n");
                }
                CanvasElement::DocCard {
                    pos,
                    size,
                    title,
                    snippet,
                    ..
                } => {
                    let text = format!("📄 {}\n\n{}", title, snippet);
                    let style = "rounded=1;whiteSpace=wrap;html=1;fillColor=#1e2430;strokeColor=#3b82f6;";

                    xml.push_str(&format!(
                        "        <mxCell id=\"{}\" value=\"{}\" style=\"{}\" vertex=\"1\" parent=\"1\">\n",
                        cell_id, escape_xml(&text), style
                    ));
                    xml.push_str(&format!(
                        "          <mxGeometry x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" as=\"geometry\"/>\n",
                        pos[0], pos[1], size[0], size[1]
                    ));
                    xml.push_str("        </mxCell>\n");
                }
                CanvasElement::Connector { .. } | CanvasElement::FreehandStroke { .. } => {}
            }
        }

        // 2. Export Connectors (Edges)
        for elem in &doc.elements {
            if let CanvasElement::Connector {
                id: _,
                from_elem,
                to_elem,
                from_pos,
                to_pos,
                routing,
                stroke_color,
                stroke_width,
                label,
                arrow_end,
            } = elem
            {
                let edge_id = format!("edge_{}", cell_counter);
                cell_counter += 1;

                let routing_style = match routing {
                    ConnectorRouting::Orthogonal => "edgeStyle=orthogonalEdgeStyle;rounded=0;",
                    ConnectorRouting::Curved => "curved=1;",
                    ConnectorRouting::Straight => "edgeStyle=none;rounded=0;",
                };

                let arrow_style = if *arrow_end {
                    "endArrow=classic;"
                } else {
                    "endArrow=none;"
                };

                let stroke_hex = to_hex_color(*stroke_color);
                let style = format!(
                    "{}html=1;{}strokeColor={};strokeWidth={};",
                    routing_style, arrow_style, stroke_hex, stroke_width
                );

                let source_attr = from_elem
                    .and_then(|sid| id_map.get(&sid))
                    .map(|sc| format!(" source=\"{}\"", sc))
                    .unwrap_or_default();

                let target_attr = to_elem
                    .and_then(|tid| id_map.get(&tid))
                    .map(|tc| format!(" target=\"{}\"", tc))
                    .unwrap_or_default();

                xml.push_str(&format!(
                    "        <mxCell id=\"{}\" value=\"{}\" style=\"{}\" edge=\"1\" parent=\"1\"{}>\n",
                    edge_id, escape_xml(label), style, format!("{}{}", source_attr, target_attr)
                ));
                xml.push_str("          <mxGeometry relative=\"1\" as=\"geometry\">\n");
                xml.push_str(&format!(
                    "            <mxPoint x=\"{:.1}\" y=\"{:.1}\" as=\"sourcePoint\"/>\n",
                    from_pos[0], from_pos[1]
                ));
                xml.push_str(&format!(
                    "            <mxPoint x=\"{:.1}\" y=\"{:.1}\" as=\"targetPoint\"/>\n",
                    to_pos[0], to_pos[1]
                ));
                xml.push_str("          </mxGeometry>\n");
                xml.push_str("        </mxCell>\n");
            }
        }

        xml.push_str("      </root>\n");
        xml.push_str("    </mxGraphModel>\n");
        xml.push_str("  </diagram>\n");
        xml.push_str("</mxfile>\n");

        xml
    }
}
