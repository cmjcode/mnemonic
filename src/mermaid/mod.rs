//! Native Mermaid support (§3.7): ```` ```mermaid ```` fences in notes are
//! parsed, laid out and drawn entirely in Rust — no JavaScript, WebView or
//! Node. Pipeline: `source::preprocess` (frontmatter, directives,
//! comments) → per-type parser (typed AST + `Diagnostic`s with line/col)
//! → layout (`layout::layered` for graph diagrams, bespoke linear/tree/
//! chart layouts for the rest) → `scene::Scene` (backend-neutral display
//! list) → `paint` (egui) or `svg`.
//!
//! Callers: `markdown::renderer` (inline rendering, cached per source),
//! `api` (`diagram validate/render` for agents).

pub mod class;
pub mod diag;
pub mod er;
pub mod flowchart;
pub mod layout;
pub mod mindmap;
pub mod paint;
pub mod pie;
pub mod route;
pub mod scene;
pub mod sequence;
pub mod source;
pub mod state;
pub mod svg;
pub mod text;
pub mod theme;

pub use diag::{Diagnostic, Severity};
pub use scene::Scene;

use text::TextMeasure;
use theme::Theme;

/// Every diagram type Mermaid knows (v11), by header keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagramKind {
    Flowchart,
    Sequence,
    Class,
    State,
    Er,
    Gantt,
    Pie,
    Journey,
    GitGraph,
    Mindmap,
    Timeline,
    Quadrant,
    Requirement,
    C4,
    Sankey,
    XyChart,
    Block,
    Packet,
    Kanban,
    Architecture,
    Radar,
    Treemap,
    ZenUml,
    Unknown,
}

impl DiagramKind {
    /// From the first word of the diagram's first line.
    pub fn detect(header: &str) -> DiagramKind {
        let word = header.split(|c: char| c.is_whitespace() || c == ';' || c == ':').next().unwrap_or("");
        match word {
            "flowchart" | "graph" | "flowchart-elk" | "flowchart-v2" => DiagramKind::Flowchart,
            "sequenceDiagram" => DiagramKind::Sequence,
            "classDiagram" | "classDiagram-v2" => DiagramKind::Class,
            "stateDiagram" | "stateDiagram-v2" => DiagramKind::State,
            "erDiagram" => DiagramKind::Er,
            "gantt" => DiagramKind::Gantt,
            "pie" => DiagramKind::Pie,
            "journey" => DiagramKind::Journey,
            "gitGraph" => DiagramKind::GitGraph,
            "mindmap" => DiagramKind::Mindmap,
            "timeline" => DiagramKind::Timeline,
            "quadrantChart" => DiagramKind::Quadrant,
            "requirementDiagram" | "requirement" => DiagramKind::Requirement,
            "C4Context" | "C4Container" | "C4Component" | "C4Dynamic" | "C4Deployment" => DiagramKind::C4,
            "sankey-beta" | "sankey" => DiagramKind::Sankey,
            "xychart-beta" | "xychart" => DiagramKind::XyChart,
            "block-beta" | "block" => DiagramKind::Block,
            "packet-beta" | "packet" => DiagramKind::Packet,
            "kanban" => DiagramKind::Kanban,
            "architecture-beta" | "architecture" => DiagramKind::Architecture,
            "radar-beta" | "radar" => DiagramKind::Radar,
            "treemap-beta" | "treemap" => DiagramKind::Treemap,
            "zenuml" => DiagramKind::ZenUml,
            _ => DiagramKind::Unknown,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            DiagramKind::Flowchart => "flowchart",
            DiagramKind::Sequence => "sequence",
            DiagramKind::Class => "class",
            DiagramKind::State => "state",
            DiagramKind::Er => "er",
            DiagramKind::Gantt => "gantt",
            DiagramKind::Pie => "pie",
            DiagramKind::Journey => "journey",
            DiagramKind::GitGraph => "git_graph",
            DiagramKind::Mindmap => "mindmap",
            DiagramKind::Timeline => "timeline",
            DiagramKind::Quadrant => "quadrant",
            DiagramKind::Requirement => "requirement",
            DiagramKind::C4 => "c4",
            DiagramKind::Sankey => "sankey",
            DiagramKind::XyChart => "xy_chart",
            DiagramKind::Block => "block",
            DiagramKind::Packet => "packet",
            DiagramKind::Kanban => "kanban",
            DiagramKind::Architecture => "architecture",
            DiagramKind::Radar => "radar",
            DiagramKind::Treemap => "treemap",
            DiagramKind::ZenUml => "zenuml",
            DiagramKind::Unknown => "unknown",
        }
    }

    /// Whether this build can lay out and draw the type.
    pub fn is_supported(self) -> bool {
        matches!(
            self,
            DiagramKind::Flowchart
                | DiagramKind::Sequence
                | DiagramKind::Pie
                | DiagramKind::State
                | DiagramKind::Class
                | DiagramKind::Er
                | DiagramKind::Mindmap
        )
    }
}

/// Result of rendering one diagram source.
#[derive(Debug, Clone)]
pub struct Rendered {
    pub kind: DiagramKind,
    /// `None` when nothing could be drawn (unsupported type, fatal error).
    pub scene: Option<Scene>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Rendered {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

pub struct RenderOptions<'a> {
    /// App is in dark mode (picks the `dark` theme unless the diagram sets one).
    pub dark: bool,
    pub measure: &'a dyn TextMeasure,
}

/// Parse and (when `layout` is given) build one supported diagram type.
/// `None` scene with no diagnostics means the type isn't implemented.
fn dispatch(
    kind: DiagramKind,
    src: &source::Source<'_>,
    layout: Option<(&Theme, &dyn TextMeasure)>,
) -> Option<(Option<Scene>, Vec<Diagnostic>)> {
    macro_rules! diagram {
        ($module:ident, |$ast:ident, $theme:ident, $m:ident| $build:expr) => {{
            let ($ast, d) = $module::parse(src);
            let scene = layout.map(|($theme, $m)| $build);
            Some((scene, d))
        }};
    }
    match kind {
        DiagramKind::Flowchart => {
            diagram!(flowchart, |fc, t, m| flowchart::build(&fc, &src.config, src.title.as_deref(), t, m))
        }
        DiagramKind::Sequence => diagram!(sequence, |seq, t, m| sequence::build(&seq, &src.config, t, m)),
        DiagramKind::Pie => diagram!(pie, |p, t, m| pie::build(&p, &src.config, t, m)),
        DiagramKind::Er => diagram!(er, |d, t, m| er::build(&d, &src.config, src.title.as_deref(), t, m)),
        DiagramKind::Class => {
            diagram!(class, |d, t, m| class::build(&d, &src.config, src.title.as_deref(), t, m))
        }
        DiagramKind::State => {
            diagram!(state, |d, t, m| state::build(&d, &src.config, src.title.as_deref(), t, m))
        }
        DiagramKind::Mindmap => diagram!(mindmap, |d, t, m| mindmap::build(&d, &src.config, t, m)),
        _ => None,
    }
}

/// Parse, lay out and build the scene for a diagram source (the text
/// inside a ```` ```mermaid ```` fence).
pub fn render(source: &str, opts: &RenderOptions<'_>) -> Rendered {
    let src = source::preprocess(source);
    let mut diagnostics = src.diagnostics.clone();
    let Some(header) = src.header() else {
        diagnostics.push(Diagnostic::error(1, 1, "empty diagram: start with a type such as `flowchart TD`"));
        return Rendered { kind: DiagramKind::Unknown, scene: None, diagnostics };
    };
    let kind = DiagramKind::detect(header.text);
    let theme = Theme::resolve(&src.config, opts.dark);
    let scene = match dispatch(kind, &src, Some((&theme, opts.measure))) {
        Some((scene, d)) => {
            diagnostics.extend(d);
            scene
        }
        None => {
            diagnostics.push(unsupported(kind, header, true));
            None
        }
    };
    Rendered { kind, scene, diagnostics }
}

fn unsupported(kind: DiagramKind, header: &source::Line<'_>, rendering: bool) -> Diagnostic {
    let col = header.indent + 1;
    if kind == DiagramKind::Unknown {
        let word = header.text.split_whitespace().next().unwrap_or("");
        return Diagnostic::error(header.no, col, format!("unknown diagram type `{word}`"));
    }
    if rendering {
        Diagnostic::error(header.no, col, format!("`{}` diagrams are not supported yet", kind.name()))
    } else {
        Diagnostic::warning(header.no, col, format!("`{}` diagrams are not validated yet", kind.name()))
    }
}

/// Parse only (no layout): the diagram type and every diagnostic.
pub fn validate(source: &str) -> (DiagramKind, Vec<Diagnostic>) {
    let src = source::preprocess(source);
    let mut diagnostics = src.diagnostics.clone();
    let Some(header) = src.header() else {
        diagnostics.push(Diagnostic::error(1, 1, "empty diagram: start with a type such as `flowchart TD`"));
        return (DiagramKind::Unknown, diagnostics);
    };
    let kind = DiagramKind::detect(header.text);
    match dispatch(kind, &src, None) {
        Some((_, d)) => diagnostics.extend(d),
        None => diagnostics.push(unsupported(kind, header, false)),
    }
    (kind, diagnostics)
}

/// A ```` ```mermaid ```` fence inside a markdown body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FencedBlock {
    /// 0-based line of the opening fence.
    pub fence_line: usize,
    /// The diagram source (lines between the fences).
    pub source: String,
    /// The closing fence was found.
    pub closed: bool,
}

impl FencedBlock {
    /// Line in the note (0-based) of diagram-source line `line` (1-based).
    pub fn note_line(&self, line: usize) -> usize {
        self.fence_line + line
    }
}

/// Is `line` the opening of a mermaid fence? Returns the fence marker.
pub fn fence_open(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let marker = if t.starts_with("```") {
        "```"
    } else if t.starts_with("~~~") {
        "~~~"
    } else {
        return None;
    };
    let info = t.trim_start_matches(marker.chars().next().unwrap_or('`')).trim();
    (info.split_whitespace().next() == Some("mermaid")).then_some(marker)
}

/// Does `line` close a fence opened with `marker` (```` ``` ```` / `~~~`)?
pub fn fence_close(line: &str, marker: &str) -> bool {
    let t = line.trim();
    t.starts_with(marker) && t.chars().all(|c| c == '`' || c == '~')
}

/// Every mermaid fence in a markdown body, in order.
pub fn fenced_blocks(markdown: &str) -> Vec<FencedBlock> {
    let mut out = Vec::new();
    let mut other_fence: Option<&str> = None;
    let mut current: Option<(usize, &str, Vec<&str>)> = None;
    for (idx, line) in markdown.lines().enumerate() {
        let t = line.trim_start();
        if let Some((start, marker, body)) = current.as_mut() {
            if fence_close(line, marker) {
                out.push(FencedBlock { fence_line: *start, source: body.join("\n"), closed: true });
                current = None;
            } else {
                body.push(line);
            }
            continue;
        }
        if let Some(marker) = other_fence {
            if t.starts_with(marker) {
                other_fence = None;
            }
            continue;
        }
        if let Some(marker) = fence_open(line) {
            current = Some((idx, marker, Vec::new()));
        } else if t.starts_with("```") {
            other_fence = Some("```");
        } else if t.starts_with("~~~") {
            other_fence = Some("~~~");
        }
    }
    if let Some((start, _, body)) = current {
        out.push(FencedBlock { fence_line: start, source: body.join("\n"), closed: false });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mermaid::text::ApproxMeasure;

    #[test]
    fn detects_every_diagram_type() {
        let cases = [
            ("flowchart LR", DiagramKind::Flowchart),
            ("graph TD;", DiagramKind::Flowchart),
            ("sequenceDiagram", DiagramKind::Sequence),
            ("classDiagram-v2", DiagramKind::Class),
            ("stateDiagram-v2", DiagramKind::State),
            ("erDiagram", DiagramKind::Er),
            ("gantt", DiagramKind::Gantt),
            ("pie showData", DiagramKind::Pie),
            ("journey", DiagramKind::Journey),
            ("gitGraph:", DiagramKind::GitGraph),
            ("mindmap", DiagramKind::Mindmap),
            ("timeline", DiagramKind::Timeline),
            ("quadrantChart", DiagramKind::Quadrant),
            ("requirementDiagram", DiagramKind::Requirement),
            ("C4Context", DiagramKind::C4),
            ("sankey-beta", DiagramKind::Sankey),
            ("xychart-beta", DiagramKind::XyChart),
            ("block-beta", DiagramKind::Block),
            ("packet-beta", DiagramKind::Packet),
            ("kanban", DiagramKind::Kanban),
            ("architecture-beta", DiagramKind::Architecture),
            ("radar-beta", DiagramKind::Radar),
            ("treemap-beta", DiagramKind::Treemap),
            ("zenuml", DiagramKind::ZenUml),
            ("nonsense", DiagramKind::Unknown),
        ];
        for (h, k) in cases {
            assert_eq!(DiagramKind::detect(h), k, "{h}");
        }
    }

    #[test]
    fn finds_mermaid_fences_only() {
        let md = "# T\n```rust\nlet x = 1;\n```\n```mermaid\nflowchart LR\nA-->B\n```\ntext\n~~~ mermaid\npie\n\"a\": 1\n~~~\n```mermaid\nunclosed";
        let blocks = fenced_blocks(md);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].fence_line, 4);
        assert_eq!(blocks[0].source, "flowchart LR\nA-->B");
        assert_eq!(blocks[0].note_line(2), 6);
        assert_eq!(blocks[1].source, "pie\n\"a\": 1");
        assert!(!blocks[2].closed);
    }

    #[test]
    fn render_reports_unsupported_and_unknown_types() {
        let opts = RenderOptions { dark: false, measure: &ApproxMeasure };
        let r = render("flowchart LR\nA-->B", &opts);
        assert!(r.scene.is_some() && !r.has_errors());
        let u = render("nonsense\n", &opts);
        assert!(u.scene.is_none() && u.has_errors());
        assert!(render("", &opts).has_errors());
    }
}
