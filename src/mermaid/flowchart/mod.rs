//! Mermaid flowchart (`flowchart` / `graph`) — AST, parser and scene
//! builder (§3.7.6). Covers node shapes (classic bracket syntax and the
//! v11 `A@{ shape: … }` form), every link type (`-->`, `---`, `-.->`,
//! `==>`, `~~~`, `<-->`, `--o`, `--x`, extra-length dashes, `|label|` and
//! `-- label -->` forms, `&` fan-out, chains, edge ids), nested
//! `subgraph … end` with `direction`, edges to subgraphs, `classDef` /
//! `class` / `:::` / `style` / `linkStyle`, and `click` links.
//! Callers: `mermaid::render`/`mermaid::validate`.

mod build;
mod parse;
mod shapes;

pub use build::build;
pub use parse::parse;

use super::layout::Dir;
use super::scene::Marker;
use super::theme::StyleSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shape {
    #[default]
    Rect,
    Round,
    Stadium,
    Subroutine,
    Cylinder,
    Circle,
    DoubleCircle,
    Asymmetric,
    Diamond,
    Hexagon,
    LeanRight,
    LeanLeft,
    Trapezoid,
    InvTrapezoid,
    // v11 named shapes.
    Document,
    NotchRect,
    SmallCircle,
    FramedCircle,
    FilledCircle,
    ForkBar,
    Hourglass,
    Brace,
    Bolt,
    Triangle,
    FlippedTriangle,
    Delay,
    HCylinder,
    LinedRect,
    Stacked,
    Flag,
    Text,
    Cloud,
}

impl Shape {
    /// v11 `@{ shape: name }` names (and their aliases).
    pub fn from_name(name: &str) -> Option<Shape> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "rect" | "proc" | "process" | "rectangle" | "square" => Shape::Rect,
            "rounded" | "event" => Shape::Round,
            "stadium" | "pill" | "terminal" => Shape::Stadium,
            "fr-rect" | "subprocess" | "subproc" | "framed-rectangle" | "subroutine" => Shape::Subroutine,
            "cyl" | "db" | "database" | "cylinder" | "lin-cyl" | "disk" | "lined-cylinder" => Shape::Cylinder,
            "circle" | "circ" => Shape::Circle,
            "dbl-circ" | "double-circle" => Shape::DoubleCircle,
            "odd" => Shape::Asymmetric,
            "diam" | "diamond" | "decision" | "question" => Shape::Diamond,
            "hex" | "hexagon" | "prepare" => Shape::Hexagon,
            "lean-r" | "lean-right" | "in-out" => Shape::LeanRight,
            "lean-l" | "lean-left" | "out-in" => Shape::LeanLeft,
            "trap-b" | "trapezoid" | "trapezoid-bottom" | "priority" => Shape::Trapezoid,
            "trap-t" | "inv-trapezoid" | "trapezoid-top" | "manual" => Shape::InvTrapezoid,
            "doc" | "document" | "lin-doc" | "lined-document" | "tag-doc" | "tagged-document" => Shape::Document,
            "docs" | "documents" | "st-doc" | "stacked-document" | "procs" | "processes" | "st-rect"
            | "stacked-rectangle" => Shape::Stacked,
            "notch-rect" | "card" | "notched-rectangle" | "notch-pent" | "loop-limit" | "notched-pentagon" => {
                Shape::NotchRect
            }
            "sm-circ" | "start" | "small-circle" => Shape::SmallCircle,
            "fr-circ" | "stop" | "framed-circle" => Shape::FramedCircle,
            "f-circ" | "junction" | "filled-circle" => Shape::FilledCircle,
            "fork" | "join" => Shape::ForkBar,
            "hourglass" | "collate" => Shape::Hourglass,
            "brace" | "brace-l" | "comment" | "brace-r" | "braces" => Shape::Brace,
            "bolt" | "com-link" | "lightning-bolt" => Shape::Bolt,
            "tri" | "triangle" | "extract" => Shape::Triangle,
            "flip-tri" | "flipped-triangle" | "manual-file" => Shape::FlippedTriangle,
            "delay" | "half-rounded-rectangle" => Shape::Delay,
            "das" | "h-cyl" | "horizontal-cylinder" => Shape::HCylinder,
            "lin-rect" | "lined-rectangle" | "lined-process" | "lin-proc" | "shaded-process" | "div-rect"
            | "div-proc" | "divided-rectangle" | "divided-process" | "win-pane" | "internal-storage"
            | "window-pane" | "tag-rect" | "tagged-rectangle" | "tag-proc" | "tagged-process" => Shape::LinedRect,
            "flag" | "paper-tape" => Shape::Flag,
            "sl-rect" | "manual-input" | "sloped-rectangle" | "curv-trap" | "display" | "curved-trapezoid"
            | "bow-rect" | "stored-data" | "bow-tie-rectangle" | "cross-circ" | "summary" | "crossed-circle" => {
                Shape::Round
            }
            "text" => Shape::Text,
            "cloud" => Shape::Cloud,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Node {
    pub id: String,
    /// Display text (already cleaned); `None` shows the id.
    pub label: Option<String>,
    /// Label came from a markdown string (auto-wraps).
    pub markdown: bool,
    pub shape: Shape,
    pub classes: Vec<String>,
    pub style: StyleSpec,
    pub subgraph: Option<usize>,
    /// 1-based source line of the first mention.
    pub line: usize,
    pub link: Option<String>,
    pub tooltip: Option<String>,
}

impl Node {
    pub fn text(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkStroke {
    #[default]
    Normal,
    Thick,
    Dotted,
    Invisible,
}

/// A link endpoint as written: node or subgraph id, resolved in `build`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub label: Option<String>,
    pub stroke: LinkStroke,
    pub start: Option<Marker>,
    pub end: Option<Marker>,
    pub minlen: u32,
    pub id: Option<String>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Subgraph {
    pub id: String,
    pub title: String,
    pub parent: Option<usize>,
    pub dir: Option<Dir>,
    pub line: usize,
    pub classes: Vec<String>,
    pub style: StyleSpec,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Flowchart {
    pub dir: Dir,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub subgraphs: Vec<Subgraph>,
    pub class_defs: Vec<(String, StyleSpec)>,
    /// `(None, style)` = `linkStyle default`.
    pub link_styles: Vec<(Option<usize>, StyleSpec)>,
}

impl Flowchart {
    pub fn node_index(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    pub fn subgraph_index(&self, id: &str) -> Option<usize> {
        self.subgraphs.iter().position(|s| s.id == id)
    }

    pub fn class_def(&self, name: &str) -> Option<&StyleSpec> {
        self.class_defs.iter().rev().find(|(n, _)| n == name).map(|(_, s)| s)
    }
}
