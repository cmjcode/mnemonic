//! Two-way Draw.io (diagrams.net) XML Importer & Exporter for MNEMONIC Canvas.
//!
//! Allows importing `.drawio` files and `mxGraphModel` XML into native `CanvasDocument`
//! elements, and exporting `CanvasDocument` whiteboards back to standard Draw.io XML
//! format fully compatible with https://app.diagrams.net and Draw.io Desktop.
//!
//! Import notes:
//! - Child geometry in Draw.io is relative to its parent (groups, containers, swimlanes,
//!   tables), so every cell is resolved to absolute world coordinates.
//! - Labels with `html=1` are XML-escaped HTML; they are unescaped, stripped of tags and
//!   HTML entities are decoded.
//! - Compressed diagrams (base64 + raw deflate + URI encoding) are inflated.
//! - Multi-page files are laid out side by side, each page wrapped in a frame.

use std::collections::HashMap;
use std::io::Read;

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use egui::{Pos2, Rect, Vec2};
use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use super::element::{CanvasElement, CanvasElementId, ConnectorRouting, ShapeKind};
use super::tools;
use super::CanvasDocument;

/// Draw.io's default fill for vertices without an explicit `fillColor`.
const DEFAULT_FILL: [f32; 3] = [1.0, 1.0, 1.0];
/// Neutral stroke that stays visible on both light and dark canvas backgrounds.
const DEFAULT_STROKE: [f32; 3] = [0.45, 0.47, 0.50];
/// Horizontal gap between pages of a multi-page diagram.
const PAGE_GAP: f32 = 200.0;
/// Padding between a page frame and the page content.
const PAGE_PADDING: f32 = 40.0;

/// Helper to parse hex colors like `#d5e8d4`, `#D5E8D4`, or `d5e8d4` into RGB `[f32; 3]`.
pub fn parse_hex_color(hex: &str) -> Option<[f32; 3]> {
    let s = hex.trim().trim_start_matches('#');
    if !s.is_ascii() {
        return None;
    }
    // `#RRGGBBAA`: alpha is ignored.
    let s = if s.len() == 8 { &s[0..6] } else { s };
    if s.len() == 6 {
        let r = u8::from_str_radix(&s[0..2], 16).ok()? as f32 / 255.0;
        let g = u8::from_str_radix(&s[2..4], 16).ok()? as f32 / 255.0;
        let b = u8::from_str_radix(&s[4..6], 16).ok()? as f32 / 255.0;
        Some([r, g, b])
    } else if s.len() == 3 {
        let mut rgb = [0.0f32; 3];
        for (i, ch) in s.chars().enumerate() {
            rgb[i] = u8::from_str_radix(&format!("{ch}{ch}"), 16).ok()? as f32 / 255.0;
        }
        Some(rgb)
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

/// Decode one level of XML/HTML entities (`&amp;`, `&nbsp;`, `&#10;`, `&#x41;`, ...).
/// Unknown entities are kept verbatim.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let decoded = after
            .char_indices()
            .take(12)
            .find(|&(_, c)| c == ';')
            .and_then(|(semi, _)| {
                let name = &after[..semi];
                let ch = match name {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some(' '),
                    _ => {
                        if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                            u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                        } else if let Some(dec) = name.strip_prefix('#') {
                            dec.parse::<u32>().ok().and_then(char::from_u32)
                        } else {
                            None
                        }
                    }
                };
                ch.map(|c| (c, semi))
            });
        match decoded {
            Some((ch, semi)) => {
                if ch != '\r' {
                    out.push(ch);
                }
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Remove HTML tags, turning block-level tags into line breaks.
fn strip_html_tags(html: &str) -> String {
    const BLOCK_TAGS: &[&str] = &[
        "div", "p", "li", "ul", "ol", "tr", "hr", "h1", "h2", "h3", "h4", "h5", "h6", "table",
    ];

    let mut out = String::with_capacity(html.len());
    let mut chars = html.char_indices();
    while let Some((i, ch)) = chars.next() {
        if ch != '<' {
            out.push(ch);
            continue;
        }
        let tail = &html[i + 1..];
        let name = tail
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        let looks_like_tag = tail.starts_with('/') || tail.starts_with('!') || !name.is_empty();
        if !looks_like_tag {
            // A literal '<' (e.g. "a < b").
            out.push(ch);
            continue;
        }
        // Skip to the closing '>' while respecting quoted attribute values.
        let mut quote: Option<char> = None;
        for (_, c) in chars.by_ref() {
            match quote {
                Some(q) if c == q => quote = None,
                Some(_) => {}
                None if c == '"' || c == '\'' => quote = Some(c),
                None if c == '>' => break,
                None => {}
            }
        }
        // `<br>` always breaks; other block tags only start a new line when needed,
        // so `<br><p>` or `</div><div>` don't create blank lines.
        if name == "br" || (BLOCK_TAGS.contains(&name.as_str()) && !out.is_empty() && !out.ends_with('\n')) {
            out.push('\n');
        }
    }
    out
}

/// Collapse runs of spaces, trim each line and drop redundant blank lines.
fn normalize_label_whitespace(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        let prev_blank = lines.last().is_none_or(|l| l.is_empty());
        if collapsed.is_empty() && prev_blank {
            continue;
        }
        lines.push(collapsed);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// Convert a Draw.io HTML label into plain text.
///
/// Accepts both the already-unescaped attribute value (`<b>Hi</b>`) and the raw
/// XML-escaped form (`&lt;b&gt;Hi&lt;/b&gt;`).
pub fn clean_drawio_label(raw: &str) -> String {
    if raw.contains("&lt;") {
        html_label_to_text(&decode_entities(raw))
    } else {
        html_label_to_text(raw)
    }
}

/// Convert an unescaped HTML label (`<b>a &amp; b</b>`) into plain text.
fn html_label_to_text(html: &str) -> String {
    normalize_label_whitespace(&decode_entities(&strip_html_tags(html)))
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

/// Escape plain text for an `html=1` label attribute (HTML-escaped, then XML-escaped).
fn escape_html_label(s: &str) -> String {
    let html = s
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\n', "<br>");
    escape_xml(&html)
}

/// Summary of what an import produced, for user feedback.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DrawioImportReport {
    /// Number of diagram pages found in the file.
    pub pages: usize,
    /// Number of canvas elements created.
    pub elements: usize,
    /// Cells that could not be represented (e.g. dangling edges, unsupported geometry).
    pub skipped: usize,
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
    // Geometry (relative to the parent cell)
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    has_geometry: bool,
    relative: bool,
    source_point: Option<[f32; 2]>,
    target_point: Option<[f32; 2]>,
    waypoints: Vec<[f32; 2]>,
}

impl RawCell {
    fn label(&self) -> String {
        if self.style.split(';').any(|p| p.trim() == "html=1") {
            html_label_to_text(&self.value)
        } else {
            normalize_label_whitespace(&self.value)
        }
    }
}

/// One `<diagram>` page.
#[derive(Debug, Default)]
struct RawPage {
    name: String,
    cells: Vec<RawCell>,
}

/// Style map parsed from Draw.io style string (e.g. `rounded=1;fillColor=#d5e8d4;shape=rhombus;`)
#[derive(Debug, Clone, Default)]
struct ParsedStyle {
    shape_type: Option<String>,
    is_rounded: bool,
    fill_color: Option<[f32; 3]>,
    has_no_fill: bool,
    stroke_color: Option<[f32; 3]>,
    has_no_stroke: bool,
    stroke_width: f32,
    font_color: Option<[f32; 3]>,
    edge_style: Option<String>,
    end_arrow: Option<String>,
    is_curved: bool,
    is_swimlane: bool,
    is_note: bool,
    is_text: bool,
    is_group: bool,
    is_edge_label: bool,
    exit: [Option<f32>; 2],
    entry: [Option<f32>; 2],
}

fn parse_style_string(style_str: &str) -> ParsedStyle {
    let mut style = ParsedStyle {
        stroke_width: 1.0,
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
        let is_true = val == "1" || val == "true";

        match key {
            "rounded" => style.is_rounded = is_true,
            "curved" => style.is_curved = is_true,
            "shape" => {
                let shape = val.to_lowercase();
                match shape.as_str() {
                    // Tables are containers with a header, like swimlanes.
                    "swimlane" | "table" => style.is_swimlane = true,
                    "note" => style.is_note = true,
                    _ => {}
                }
                style.shape_type = Some(shape);
            }
            "fillColor" => {
                if val.eq_ignore_ascii_case("none") {
                    style.has_no_fill = true;
                } else {
                    style.fill_color = parse_hex_color(val);
                }
            }
            "strokeColor" => {
                if val.eq_ignore_ascii_case("none") {
                    style.has_no_stroke = true;
                } else {
                    style.stroke_color = parse_hex_color(val);
                }
            }
            "fontColor" => style.font_color = parse_hex_color(val),
            "strokeWidth" => {
                if let Ok(w) = val.parse::<f32>() {
                    style.stroke_width = w.clamp(0.5, 20.0);
                }
            }
            "edgeStyle" => style.edge_style = Some(val.to_string()),
            "endArrow" => style.end_arrow = Some(val.to_string()),
            "exitX" => style.exit[0] = val.parse().ok(),
            "exitY" => style.exit[1] = val.parse().ok(),
            "entryX" => style.entry[0] = val.parse().ok(),
            "entryY" => style.entry[1] = val.parse().ok(),
            // Bare style names (no `=`)
            "swimlane" => style.is_swimlane = true,
            "note" => style.is_note = true,
            "text" => style.is_text = true,
            "group" => style.is_group = true,
            "edgeLabel" => style.is_edge_label = true,
            "ellipse" => style.shape_type = Some("ellipse".to_string()),
            "rhombus" | "diamond" => style.shape_type = Some("rhombus".to_string()),
            _ => {}
        }
    }

    style
}

/// Fill and stroke width a plain vertex renders with (`0.0` = no border).
fn vertex_appearance(style: &ParsedStyle) -> (Option<[f32; 3]>, f32) {
    let fill = if style.has_no_fill || (style.is_text && style.fill_color.is_none()) {
        None
    } else {
        Some(style.fill_color.unwrap_or(DEFAULT_FILL))
    };
    // Text cells are borderless unless a stroke is set explicitly. Table rows and cells
    // draw per-side borders that the canvas can't express, so they stay borderless too.
    let borderless_shape = matches!(style.shape_type.as_deref(), Some("tablerow" | "partialrectangle"));
    let invisible_stroke = style.has_no_stroke
        || borderless_shape
        || (style.is_text && style.stroke_color.is_none());
    (fill, if invisible_stroke { 0.0 } else { style.stroke_width })
}

fn shape_kind_for(style: &ParsedStyle) -> ShapeKind {
    let shape = style.shape_type.as_deref().unwrap_or("");
    if shape.contains("ellipse") || shape.ends_with("doublecircle") {
        ShapeKind::Ellipse
    } else if shape == "rhombus" || shape.contains("decision") || shape.contains("diamond") {
        ShapeKind::Diamond
    } else if matches!(shape, "cloud" | "callout" | "speech") {
        ShapeKind::CalloutBubble
    } else if style.is_rounded {
        ShapeKind::RoundedRect
    } else {
        ShapeKind::Rectangle
    }
}

/// Decode a compressed `<diagram>` payload: base64 → raw inflate → URI-decode.
fn decode_compressed_diagram(data: &str) -> Result<String> {
    let cleaned: String = data.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(cleaned)
        .context("diagram payload is not valid base64")?;
    let mut inflated = String::new();
    flate2::read::DeflateDecoder::new(&bytes[..])
        .read_to_string(&mut inflated)
        .context("failed to inflate compressed diagram")?;
    let decoded = percent_encoding::percent_decode_str(&inflated)
        .decode_utf8()
        .context("compressed diagram is not valid UTF-8")?;
    Ok(decoded.into_owned())
}

/// Attribute value with XML entities resolved (`&lt;` → `<`).
fn attr_value(attr: &Attribute) -> String {
    attr.normalized_value(XmlVersion::Implicit1_0)
        .map(|v| v.into_owned())
        .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value).into_owned())
}

fn get_attr(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name.as_bytes())
        .map(|a| attr_value(&a))
}

/// Unwrap a ```` ```drawio ```` / ```` ```xml ```` markdown fence if present.
fn unwrap_code_fence(content: &str) -> &str {
    for fence in ["```drawio", "```xml"] {
        if let Some(start) = content.find(fence) {
            let rest = &content[start + fence.len()..];
            return match rest.find("```") {
                Some(end) => rest[..end].trim(),
                None => rest.trim(),
            };
        }
    }
    content
}

fn push_cell(pages: &mut Vec<RawPage>, cell: RawCell) {
    if pages.is_empty() {
        pages.push(RawPage {
            name: "Page-1".to_string(),
            cells: Vec::new(),
        });
    }
    if let Some(page) = pages.last_mut() {
        page.cells.push(cell);
    }
}

/// Importer to load Draw.io XML into a `CanvasDocument`.
pub struct DrawioImporter;

impl DrawioImporter {
    /// Parse a Draw.io XML string (or ` ```drawio ... ``` ` markdown block) into a `CanvasDocument`.
    pub fn from_xml(title: &str, xml_content: &str) -> Result<CanvasDocument> {
        Self::from_xml_with_report(title, xml_content).map(|(doc, _)| doc)
    }

    /// Like [`Self::from_xml`], but also reports how many pages/elements were imported
    /// and how many cells had to be skipped.
    pub fn from_xml_with_report(
        title: &str,
        xml_content: &str,
    ) -> Result<(CanvasDocument, DrawioImportReport)> {
        let xml_str = unwrap_code_fence(xml_content.trim());
        let pages = Self::parse_pages(xml_str)?;

        let mut doc = CanvasDocument::new(title);
        let mut report = DrawioImportReport {
            pages: pages.len(),
            ..Default::default()
        };

        let multi_page = pages.len() > 1;
        let mut cursor_x: Option<f32> = None;
        let mut top_y = 0.0f32;

        for page in &pages {
            let mut elements = Self::build_page(&page.cells, &mut report.skipped);
            if elements.is_empty() {
                continue;
            }
            if !multi_page {
                doc.elements.extend(elements);
                continue;
            }

            let bounds = elements
                .iter()
                .map(|e| e.bounding_rect())
                .fold(Rect::NOTHING, |acc, r| acc.union(r));
            let frame_min = match cursor_x {
                None => {
                    top_y = bounds.min.y - PAGE_PADDING;
                    Pos2::new(bounds.min.x - PAGE_PADDING, top_y)
                }
                Some(x) => Pos2::new(x, top_y),
            };
            let delta = frame_min + Vec2::splat(PAGE_PADDING) - bounds.min;
            for elem in &mut elements {
                elem.translate(delta);
            }
            let frame_max = bounds.max + delta + Vec2::splat(PAGE_PADDING);
            cursor_x = Some(frame_max.x + PAGE_GAP);

            doc.add_element(CanvasElement::Frame {
                id: CanvasElementId::new(),
                rect: [frame_min.x, frame_min.y, frame_max.x, frame_max.y],
                title: page.name.clone(),
                color: tools::PALETTE_PRIMARY_ACCENT,
            });
            doc.elements.extend(elements);
        }

        report.elements = doc.elements.len();
        Ok((doc, report))
    }

    /// Parse every `<diagram>` page (or a bare `<mxGraphModel>`) into raw cells.
    fn parse_pages(xml_str: &str) -> Result<Vec<RawPage>> {
        let mut reader = Reader::from_str(xml_str);
        reader.config_mut().trim_text(true);

        let mut pages: Vec<RawPage> = Vec::new();
        let mut in_diagram = false;
        let mut current_cell: Option<RawCell> = None;
        let mut in_object = false;
        let mut in_points = false;

        loop {
            let event = reader
                .read_event()
                .map_err(|err| anyhow!("Error parsing Draw.io XML: {err}"))?;
            let is_empty = matches!(event, Event::Empty(_));
            match event {
                Event::Start(ref e) | Event::Empty(ref e) => match e.name().as_ref() {
                    b"diagram" => {
                        let index = pages.len() + 1;
                        pages.push(RawPage {
                            name: get_attr(e, "name").unwrap_or_else(|| format!("Page-{index}")),
                            cells: Vec::new(),
                        });
                        in_diagram = !is_empty;
                    }
                    b"object" | b"UserObject" => {
                        if let Some(prev) = current_cell.take() {
                            push_cell(&mut pages, prev);
                        }
                        let mut cell = RawCell::default();
                        Self::parse_cell_attributes(e, &mut cell);
                        if is_empty {
                            push_cell(&mut pages, cell);
                        } else {
                            current_cell = Some(cell);
                            in_object = true;
                        }
                    }
                    b"mxCell" => {
                        if in_object {
                            // The mxCell inside an <object> carries style, flags and geometry.
                            if let Some(cell) = current_cell.as_mut() {
                                Self::parse_cell_attributes(e, cell);
                            }
                        } else {
                            if let Some(prev) = current_cell.take() {
                                push_cell(&mut pages, prev);
                            }
                            let mut cell = RawCell::default();
                            Self::parse_cell_attributes(e, &mut cell);
                            if is_empty {
                                push_cell(&mut pages, cell);
                            } else {
                                current_cell = Some(cell);
                            }
                        }
                    }
                    b"mxGeometry" => {
                        let is_geometry = get_attr(e, "as").is_none_or(|a| a == "geometry");
                        if let (true, Some(cell)) = (is_geometry, current_cell.as_mut()) {
                            cell.has_geometry = true;
                            Self::parse_geometry_attributes(e, cell);
                        }
                    }
                    b"Array" => {
                        in_points = !is_empty && get_attr(e, "as").as_deref() == Some("points");
                    }
                    b"mxPoint" => {
                        if let Some(cell) = current_cell.as_mut() {
                            Self::parse_point_attributes(e, cell, in_points);
                        }
                    }
                    _ => {}
                },
                Event::End(ref e) => match e.name().as_ref() {
                    b"mxCell" if !in_object => {
                        if let Some(cell) = current_cell.take() {
                            push_cell(&mut pages, cell);
                        }
                    }
                    b"object" | b"UserObject" => {
                        if let Some(cell) = current_cell.take() {
                            push_cell(&mut pages, cell);
                        }
                        in_object = false;
                    }
                    b"Array" => in_points = false,
                    b"diagram" => in_diagram = false,
                    _ => {}
                },
                Event::Text(ref t) if in_diagram => {
                    let text = t
                        .decode()
                        .map_err(|err| anyhow!("Error decoding diagram text: {err}"))?;
                    let text = text.trim();
                    let Some(page) = pages.last_mut() else { continue };
                    if !text.is_empty() && !text.starts_with('<') && page.cells.is_empty() {
                        let inner = decode_compressed_diagram(text)
                            .with_context(|| format!("Couldn't decode compressed page \"{}\"", page.name))?;
                        for inner_page in Self::parse_pages(&inner)? {
                            page.cells.extend(inner_page.cells);
                        }
                    }
                }
                Event::Eof => {
                    if let Some(cell) = current_cell.take() {
                        push_cell(&mut pages, cell);
                    }
                    break;
                }
                _ => {}
            }
        }

        Ok(pages)
    }

    fn parse_cell_attributes(e: &BytesStart, cell: &mut RawCell) {
        for attr in e.attributes().flatten() {
            let val = attr_value(&attr);
            match attr.key.as_ref() {
                b"id" if cell.id.is_empty() => cell.id = val,
                b"parent" => cell.parent = Some(val),
                b"value" | b"label" if cell.value.is_empty() => cell.value = val,
                b"style" => cell.style = val,
                b"vertex" => cell.vertex = val == "1" || val == "true",
                b"edge" => cell.edge = val == "1" || val == "true",
                b"source" => cell.source = Some(val),
                b"target" => cell.target = Some(val),
                _ => {}
            }
        }
    }

    fn parse_geometry_attributes(e: &BytesStart, cell: &mut RawCell) {
        for attr in e.attributes().flatten() {
            let val = attr_value(&attr);
            let num = val.parse::<f32>().unwrap_or(0.0);
            match attr.key.as_ref() {
                b"x" => cell.x = num,
                b"y" => cell.y = num,
                b"width" => cell.width = num,
                b"height" => cell.height = num,
                b"relative" => cell.relative = val == "1",
                _ => {}
            }
        }
    }

    fn parse_point_attributes(e: &BytesStart, cell: &mut RawCell, in_points: bool) {
        let mut point_type = String::new();
        let mut px = 0.0f32;
        let mut py = 0.0f32;

        for attr in e.attributes().flatten() {
            let val = attr_value(&attr);
            match attr.key.as_ref() {
                b"as" => point_type = val,
                b"x" => px = val.parse::<f32>().unwrap_or(0.0),
                b"y" => py = val.parse::<f32>().unwrap_or(0.0),
                _ => {}
            }
        }

        match point_type.as_str() {
            "sourcePoint" => cell.source_point = Some([px, py]),
            "targetPoint" => cell.target_point = Some([px, py]),
            "" if in_points => cell.waypoints.push([px, py]),
            _ => {}
        }
    }

    /// Resolve cells of one page into canvas elements in document (z-)order.
    fn build_page(cells: &[RawCell], skipped: &mut usize) -> Vec<CanvasElement> {
        let by_id: HashMap<&str, usize> = cells
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.id.is_empty())
            .map(|(i, c)| (c.id.as_str(), i))
            .collect();
        let styles: Vec<ParsedStyle> = cells.iter().map(|c| parse_style_string(&c.style)).collect();
        let parent_idx = |idx: usize| cells[idx].parent.as_deref().and_then(|p| by_id.get(p).copied());

        // Absolute origin of a cell's coordinate space = sum of ancestor vertex offsets.
        let origin_of = |idx: usize| -> Vec2 {
            let mut offset = Vec2::ZERO;
            let mut current = parent_idx(idx);
            let mut depth = 0;
            while let Some(p) = current {
                depth += 1;
                if depth > cells.len() {
                    log::warn!("Draw.io import: parent cycle at cell '{}'", cells[idx].id);
                    break;
                }
                let pc = &cells[p];
                if pc.vertex && pc.has_geometry && !pc.relative {
                    offset += Vec2::new(pc.x, pc.y);
                }
                current = parent_idx(p);
            }
            offset
        };

        // Edge labels: vertices attached to an edge become that edge's label.
        let mut edge_labels: HashMap<usize, Vec<String>> = HashMap::new();
        let mut is_edge_label = vec![false; cells.len()];
        for (idx, cell) in cells.iter().enumerate() {
            if !cell.vertex {
                continue;
            }
            match parent_idx(idx) {
                Some(p) if cells[p].edge => {
                    is_edge_label[idx] = true;
                    let label = cell.label();
                    if !label.is_empty() {
                        edge_labels.entry(p).or_default().push(label);
                    }
                }
                _ if styles[idx].is_edge_label => {
                    is_edge_label[idx] = true;
                    *skipped += 1;
                }
                _ => {}
            }
        }

        // Pass A: absolute rects for all vertices, element ids for the visible ones.
        let mut rects: HashMap<usize, Rect> = HashMap::new();
        let mut elem_ids: HashMap<usize, CanvasElementId> = HashMap::new();
        for (idx, cell) in cells.iter().enumerate() {
            let is_vertex = cell.vertex || (!cell.edge && cell.has_geometry);
            if !is_vertex || is_edge_label[idx] {
                continue;
            }
            if cell.relative || (cell.width <= 0.0 && cell.height <= 0.0) {
                if cell.vertex {
                    log::warn!("Draw.io import: skipping vertex '{}' without usable geometry", cell.id);
                    *skipped += 1;
                }
                continue;
            }
            let min = Pos2::new(cell.x, cell.y) + origin_of(idx);
            rects.insert(
                idx,
                Rect::from_min_size(min, Vec2::new(cell.width.max(1.0), cell.height.max(1.0))),
            );
            // Groups and empty layout cells (e.g. table rows) are invisible but still
            // act as coordinate parents and edge endpoints.
            let style = &styles[idx];
            let (fill, stroke_width) = vertex_appearance(style);
            let invisible = style.is_group
                || (!style.is_swimlane
                    && !style.is_note
                    && fill.is_none()
                    && stroke_width <= 0.0
                    && cell.label().is_empty());
            if !invisible {
                elem_ids.insert(idx, CanvasElementId::new());
            }
        }

        // Pass B: emit elements in document order.
        let mut elements = Vec::new();
        for (idx, cell) in cells.iter().enumerate() {
            let style = &styles[idx];

            if cell.edge {
                let endpoint = |id: &Option<String>| id.as_deref().and_then(|s| by_id.get(s)).copied();
                let source = endpoint(&cell.source);
                let target = endpoint(&cell.target);
                let connector = Self::build_connector(
                    cell,
                    style,
                    source.and_then(|i| rects.get(&i)).copied(),
                    target.and_then(|i| rects.get(&i)).copied(),
                    edge_labels.get(&idx).map(Vec::as_slice).unwrap_or_default(),
                    origin_of(idx),
                );
                match connector {
                    Some(mut connector) => {
                        if let CanvasElement::Connector { from_elem, to_elem, .. } = &mut connector {
                            *from_elem = source.and_then(|i| elem_ids.get(&i)).copied();
                            *to_elem = target.and_then(|i| elem_ids.get(&i)).copied();
                        }
                        elements.push(connector);
                    }
                    None => {
                        log::warn!("Draw.io import: skipping edge '{}' without resolvable endpoints", cell.id);
                        *skipped += 1;
                    }
                }
                continue;
            }

            let (Some(&rect), Some(&elem_id)) = (rects.get(&idx), elem_ids.get(&idx)) else {
                continue;
            };
            let label = cell.label();

            let element = if style.is_swimlane {
                CanvasElement::Frame {
                    id: elem_id,
                    rect: [rect.min.x, rect.min.y, rect.max.x, rect.max.y],
                    title: label,
                    color: style
                        .stroke_color
                        .or(style.fill_color)
                        .unwrap_or(tools::PALETTE_PRIMARY_ACCENT),
                }
            } else if style.is_note {
                CanvasElement::StickyNote {
                    id: elem_id,
                    pos: [rect.min.x, rect.min.y],
                    size: [rect.width(), rect.height()],
                    text: label,
                    color: style.fill_color.unwrap_or(tools::PALETTE_STICKY_YELLOW),
                }
            } else {
                let (fill_color, stroke_width) = vertex_appearance(style);
                CanvasElement::Shape {
                    id: elem_id,
                    kind: shape_kind_for(style),
                    rect: [rect.min.x, rect.min.y, rect.max.x, rect.max.y],
                    stroke_color: style.stroke_color.unwrap_or(DEFAULT_STROKE),
                    stroke_width,
                    fill_color,
                    text: label,
                    text_color: style.font_color,
                }
            };
            elements.push(element);
        }

        elements
    }

    /// Build a connector with absolute waypoints and endpoints attached to shape borders.
    /// Returns `None` when an end is neither connected to a shape nor has an explicit point.
    fn build_connector(
        cell: &RawCell,
        style: &ParsedStyle,
        source_rect: Option<Rect>,
        target_rect: Option<Rect>,
        extra_labels: &[String],
        origin: Vec2,
    ) -> Option<CanvasElement> {
        let abs = |p: [f32; 2]| Pos2::new(p[0], p[1]) + origin;
        let waypoints: Vec<Pos2> = cell.waypoints.iter().map(|&p| abs(p)).collect();

        // Rough positions of each end, used to aim the other end before clipping.
        let source_guess = source_rect.map(|r| r.center()).or(cell.source_point.map(abs))?;
        let target_guess = target_rect.map(|r| r.center()).or(cell.target_point.map(abs))?;

        let routing = match style.edge_style.as_deref() {
            Some(
                "orthogonalEdgeStyle" | "elbowEdgeStyle" | "entityRelationEdgeStyle" | "isometricEdgeStyle",
            ) => ConnectorRouting::Orthogonal,
            _ if style.is_curved => ConnectorRouting::Curved,
            _ => ConnectorRouting::Straight,
        };
        let orthogonal = routing == ConnectorRouting::Orthogonal;

        let from_pos = match source_rect {
            Some(r) => {
                let toward = waypoints.first().copied().unwrap_or(target_guess);
                anchor_on_rect(r, style.exit, toward, orthogonal)
            }
            None => source_guess,
        };
        let to_pos = match target_rect {
            Some(r) => {
                let toward = waypoints.last().copied().unwrap_or(source_guess);
                anchor_on_rect(r, style.entry, toward, orthogonal)
            }
            None => target_guess,
        };

        let label = std::iter::once(cell.label())
            .chain(extra_labels.iter().cloned())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        Some(CanvasElement::Connector {
            id: CanvasElementId::new(),
            from_elem: None,
            to_elem: None,
            from_pos: [from_pos.x, from_pos.y],
            to_pos: [to_pos.x, to_pos.y],
            routing,
            stroke_color: style.stroke_color.unwrap_or(DEFAULT_STROKE),
            stroke_width: style.stroke_width,
            label,
            arrow_end: style.end_arrow.as_deref() != Some("none"),
            waypoints: waypoints.iter().map(|p| [p.x, p.y]).collect(),
        })
    }
}

/// Point where a connector attaches to a shape.
///
/// Uses the explicit `exitX/exitY` (or `entryX/entryY`) constraint when present; otherwise
/// orthogonal edges leave from the side facing `toward`, and other edges leave where the
/// line from the shape center to `toward` crosses the border.
fn anchor_on_rect(rect: Rect, constraint: [Option<f32>; 2], toward: Pos2, orthogonal: bool) -> Pos2 {
    if let [Some(fx), Some(fy)] = constraint {
        return rect.min
            + Vec2::new(fx.clamp(0.0, 1.0) * rect.width(), fy.clamp(0.0, 1.0) * rect.height());
    }

    let center = rect.center();
    let d = toward - center;
    if d.length_sq() < 1e-6 || rect.contains(toward) {
        return center;
    }

    if orthogonal {
        if (rect.min.x..=rect.max.x).contains(&toward.x) {
            let y = if d.y < 0.0 { rect.min.y } else { rect.max.y };
            return Pos2::new(toward.x, y);
        }
        if (rect.min.y..=rect.max.y).contains(&toward.y) {
            let x = if d.x < 0.0 { rect.min.x } else { rect.max.x };
            return Pos2::new(x, toward.y);
        }
        return if d.x.abs() >= d.y.abs() {
            Pos2::new(if d.x < 0.0 { rect.min.x } else { rect.max.x }, center.y)
        } else {
            Pos2::new(center.x, if d.y < 0.0 { rect.min.y } else { rect.max.y })
        };
    }

    let half = rect.size() * 0.5;
    let tx = if d.x.abs() > 1e-6 { half.x / d.x.abs() } else { f32::INFINITY };
    let ty = if d.y.abs() > 1e-6 { half.y / d.y.abs() } else { f32::INFINITY };
    center + d * tx.min(ty)
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
                    text_color,
                    ..
                } => {
                    let w = (rect[2] - rect[0]).max(10.0);
                    let h = (rect[3] - rect[1]).max(10.0);
                    let x = rect[0];
                    let y = rect[1];

                    let shape_name = match kind {
                        ShapeKind::Rectangle => "rounded=0",
                        ShapeKind::RoundedRect => "rounded=1",
                        ShapeKind::Ellipse => "ellipse",
                        ShapeKind::Diamond => "rhombus",
                        ShapeKind::CalloutBubble => "shape=callout",
                    };

                    let stroke_hex = if *stroke_width <= 0.0 {
                        "none".to_string()
                    } else {
                        to_hex_color(*stroke_color)
                    };
                    let fill_hex = fill_color.map(to_hex_color).unwrap_or_else(|| "none".to_string());
                    let font_style = text_color
                        .map(|c| format!("fontColor={};", to_hex_color(c)))
                        .unwrap_or_default();

                    let style = format!(
                        "{};whiteSpace=wrap;html=1;strokeColor={};fillColor={};strokeWidth={};{}",
                        shape_name,
                        stroke_hex,
                        fill_hex,
                        stroke_width.max(0.5),
                        font_style
                    );

                    xml.push_str(&format!(
                        "        <mxCell id=\"{}\" value=\"{}\" style=\"{}\" vertex=\"1\" parent=\"1\">\n",
                        cell_id, escape_html_label(text), style
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
                        cell_id, escape_html_label(text), style
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
                        cell_id, escape_html_label(title), style
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
                        cell_id, escape_html_label(&text), style
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
                waypoints,
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
                    "        <mxCell id=\"{}\" value=\"{}\" style=\"{}\" edge=\"1\" parent=\"1\"{}{}>\n",
                    edge_id, escape_html_label(label), style, source_attr, target_attr
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
                if !waypoints.is_empty() {
                    xml.push_str("            <Array as=\"points\">\n");
                    for pt in waypoints {
                        xml.push_str(&format!(
                            "              <mxPoint x=\"{:.1}\" y=\"{:.1}\"/>\n",
                            pt[0], pt[1]
                        ));
                    }
                    xml.push_str("            </Array>\n");
                }
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
