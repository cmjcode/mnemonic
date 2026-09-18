//! Serde model of the Obsidian **JSON Canvas 1.0** file format
//! (<https://jsoncanvas.org/spec/1.0/>), plus the `mnemonic` extension blocks.
//!
//! Every struct keeps unknown keys in a flattened `extra` map so files written by
//! other tools survive a load/save cycle untouched. MNEMONIC-only data (shape kind,
//! stroke widths, freehand strokes, viewport, ...) lives under `"mnemonic"` objects,
//! which Obsidian ignores.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

/// Top-level `.canvas` document: `{"nodes":[...],"edges":[...]}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JsonCanvas {
    #[serde(default)]
    pub nodes: Vec<JcNode>,
    #[serde(default)]
    pub edges: Vec<JcEdge>,
    /// Data JSON Canvas can't express: free connectors, strokes, viewport.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mnemonic: Option<JcExtension>,
    /// Unknown top-level keys, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

/// Node `type` values defined by the spec.
pub const NODE_TEXT: &str = "text";
pub const NODE_FILE: &str = "file";
pub const NODE_LINK: &str = "link";
pub const NODE_GROUP: &str = "group";

/// A node. The spec's per-type fields are all optional here and selected by
/// [`JcNode::node_type`]; [`JcNode::kind`] gives the typed view.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JcNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    #[serde(deserialize_with = "lenient_int")]
    pub x: i64,
    #[serde(deserialize_with = "lenient_int")]
    pub y: i64,
    #[serde(deserialize_with = "lenient_int")]
    pub width: i64,
    #[serde(deserialize_with = "lenient_int")]
    pub height: i64,
    /// Preset `"1"`..`"6"` or `"#rrggbb"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    // --- text ---
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    // --- file ---
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subpath: Option<String>,
    // --- link ---
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    // --- group ---
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(rename = "backgroundStyle", default, skip_serializing_if = "Option::is_none")]
    pub background_style: Option<String>,
    /// MNEMONIC-only presentation data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mnemonic: Option<JcNodeExt>,
    /// Unknown keys, preserved verbatim.
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

/// Typed view of a node's spec-defined payload.
#[derive(Debug, Clone, PartialEq)]
pub enum JcNodeKind<'a> {
    Text {
        text: &'a str,
    },
    File {
        file: &'a str,
        subpath: Option<&'a str>,
    },
    Link {
        url: &'a str,
    },
    Group {
        label: Option<&'a str>,
    },
    /// A `type` this crate doesn't know.
    Unknown(&'a str),
}

impl JcNode {
    /// Node with the common fields set and no payload.
    pub fn new(id: impl Into<String>, node_type: &str, rect: [i64; 4]) -> Self {
        JcNode {
            id: id.into(),
            node_type: node_type.to_string(),
            x: rect[0],
            y: rect[1],
            width: rect[2],
            height: rect[3],
            ..Default::default()
        }
    }

    pub fn kind(&self) -> JcNodeKind<'_> {
        match self.node_type.as_str() {
            NODE_TEXT => JcNodeKind::Text {
                text: self.text.as_deref().unwrap_or(""),
            },
            NODE_FILE => JcNodeKind::File {
                file: self.file.as_deref().unwrap_or(""),
                subpath: self.subpath.as_deref(),
            },
            NODE_LINK => JcNodeKind::Link {
                url: self.url.as_deref().unwrap_or(""),
            },
            NODE_GROUP => JcNodeKind::Group {
                label: self.label.as_deref(),
            },
            other => JcNodeKind::Unknown(other),
        }
    }

    /// `[min_x, min_y, max_x, max_y]` as floats.
    pub fn rect(&self) -> [f32; 4] {
        [
            self.x as f32,
            self.y as f32,
            (self.x + self.width) as f32,
            (self.y + self.height) as f32,
        ]
    }

    /// Block id when this is a `file` node whose `subpath` is a block reference (`#^id`).
    pub fn block_ref(&self) -> Option<&str> {
        if self.node_type != NODE_FILE {
            return None;
        }
        self.subpath
            .as_deref()
            .and_then(|s| s.strip_prefix("#^"))
            .filter(|id| !id.is_empty())
    }
}

/// Per-node MNEMONIC data.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JcNodeExt {
    /// `"sticky"` | `"shape"` | `"frame"` | `"doccard"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// [`ShapeKind`](crate::canvas::ShapeKind) variant name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_color: Option<String>,
    /// Sticky note pastel (also mirrored into the node `color`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    // --- doccard ---
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    /// Position in the document's z-order (elements are sorted by it on import).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z: Option<usize>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

/// Edge `fromEnd` / `toEnd` values.
pub const END_NONE: &str = "none";
pub const END_ARROW: &str = "arrow";

/// An edge between two nodes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JcEdge {
    pub id: String,
    #[serde(rename = "fromNode")]
    pub from_node: String,
    #[serde(rename = "fromSide", default, skip_serializing_if = "Option::is_none")]
    pub from_side: Option<String>,
    /// Default `"none"`.
    #[serde(rename = "fromEnd", default, skip_serializing_if = "Option::is_none")]
    pub from_end: Option<String>,
    #[serde(rename = "toNode")]
    pub to_node: String,
    #[serde(rename = "toSide", default, skip_serializing_if = "Option::is_none")]
    pub to_side: Option<String>,
    /// Default `"arrow"`.
    #[serde(rename = "toEnd", default, skip_serializing_if = "Option::is_none")]
    pub to_end: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mnemonic: Option<JcEdgeExt>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

impl JcEdge {
    /// Whether the edge ends in an arrow head (`toEnd` defaults to `"arrow"`).
    pub fn has_arrow_end(&self) -> bool {
        self.to_end.as_deref() != Some(END_NONE)
    }
}

/// Per-edge MNEMONIC data.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JcEdgeExt {
    /// [`ConnectorRouting`](crate::canvas::ConnectorRouting) variant name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<f32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waypoints: Vec<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_pos: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_pos: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z: Option<usize>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

/// Top-level `mnemonic` block.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JcExtension {
    /// The `CanvasDocument` id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Connectors with at least one end not attached to a node.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub free_connectors: Vec<JcFreeConnector>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strokes: Vec<JcStroke>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<JcViewport>,
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

impl JcExtension {
    pub fn is_empty(&self) -> bool {
        self.id.is_none()
            && self.free_connectors.is_empty()
            && self.strokes.is_empty()
            && self.viewport.is_none()
            && self.extra.is_empty()
    }
}

/// A connector that JSON Canvas can't express as an edge (a dangling end).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JcFreeConnector {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_node: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_node: Option<String>,
    pub from_pos: [f32; 2],
    pub to_pos: [f32; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default = "default_true")]
    pub arrow_end: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waypoints: Vec<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z: Option<usize>,
}

/// A freehand stroke.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JcStroke {
    pub id: String,
    pub points: Vec<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JcViewport {
    pub pan: [f32; 2],
    pub zoom: f32,
}

fn default_true() -> bool {
    true
}

/// The spec says coordinates are integers, but other writers emit `100.0`.
fn lenient_int<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    let v = f64::deserialize(d)?;
    if !v.is_finite() {
        return Err(serde::de::Error::custom("coordinate is not finite"));
    }
    Ok(v.round() as i64)
}
