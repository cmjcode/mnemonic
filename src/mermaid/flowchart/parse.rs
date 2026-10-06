//! Hand-written recursive-descent parser for flowchart statements. Works
//! on the preprocessed lines of `mermaid::source`, splits them into
//! `;`-separated statements (outside quotes/brackets), and never aborts:
//! a bad statement yields a `Diagnostic` with line/column and parsing
//! continues with the next one. Callers: `mermaid::render`,
//! `mermaid::validate` (via `flowchart::parse`).

use super::{Edge, Flowchart, LinkStroke, Node, Shape, Subgraph};
use crate::mermaid::diag::Diagnostic;
use crate::mermaid::layout::Dir;
use crate::mermaid::scene::Marker;
use crate::mermaid::source::Source;
use crate::mermaid::text::clean_label;
use crate::mermaid::theme::StyleSpec;

pub fn parse(src: &Source<'_>) -> (Flowchart, Vec<Diagnostic>) {
    let mut p = Parser::default();
    let Some(header) = src.header() else {
        p.diags.push(Diagnostic::error(1, 1, "empty diagram"));
        return (p.fc, p.diags);
    };

    // `graph TD;A-->B` may put statements on the header line.
    let mut statements: Vec<(usize, usize, String)> = Vec::new();
    let header_parts = split_statements(header.text);
    if let Some((_, first)) = header_parts.first() {
        let mut words = first.split_whitespace();
        let keyword = words.next().unwrap_or("");
        if !matches!(keyword, "flowchart" | "graph" | "flowchart-elk" | "flowchart-v2") {
            p.diags.push(Diagnostic::error(
                header.no,
                header.indent + 1,
                format!("expected `flowchart` or `graph`, found `{keyword}`"),
            ));
        }
        if let Some(d) = words.next() {
            match Dir::parse(d) {
                Some(dir) => p.fc.dir = dir,
                None => p.diags.push(Diagnostic::error(
                    header.no,
                    header.indent + 1,
                    format!("unknown direction `{d}` (use TB, TD, BT, LR or RL)"),
                )),
            }
        }
    }
    for (off, text) in header_parts.into_iter().skip(1) {
        statements.push((header.no, header.indent + off, text));
    }

    // Join lines while a quote is left open (multi-line markdown strings).
    let body = src.body();
    let mut i = 0;
    while i < body.len() {
        let line = &body[i];
        let mut text = line.text.to_string();
        while text.matches('"').count() % 2 == 1 && i + 1 < body.len() {
            i += 1;
            text.push('\n');
            text.push_str(body[i].text);
        }
        for (off, stmt) in split_statements(&text) {
            statements.push((line.no, line.indent + off, stmt));
        }
        i += 1;
    }

    for (line, col, stmt) in statements {
        p.statement(&stmt, line, col);
    }
    for &open in &p.stack {
        let sg = &p.fc.subgraphs[open];
        p.diags
            .push(Diagnostic::error(sg.line, 1, format!("subgraph `{}` is never closed with `end`", sg.id)));
    }
    for (idx, _) in &p.fc.link_styles {
        if let Some(i) = idx
            && *i >= p.fc.edges.len()
        {
            p.diags.push(Diagnostic::warning(
                1,
                1,
                format!("linkStyle index {i} is out of range ({} links)", p.fc.edges.len()),
            ));
        }
    }
    (p.fc, p.diags)
}

/// Split on `;` outside quotes and brackets; returns (char offset, text).
fn split_statements(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quoted = false;
    let mut start_char = 0;
    let mut cur = String::new();
    for (ci, c) in text.chars().enumerate() {
        match c {
            '"' => quoted = !quoted,
            '[' | '(' | '{' if !quoted => depth += 1,
            ']' | ')' | '}' if !quoted => depth -= 1,
            _ => {}
        }
        if c == ';' && !quoted && depth <= 0 {
            push_trimmed(&mut out, start_char, &cur);
            cur.clear();
            start_char = ci + 1;
            continue;
        }
        cur.push(c);
    }
    push_trimmed(&mut out, start_char, &cur);
    out
}

fn push_trimmed(out: &mut Vec<(usize, String)>, start: usize, s: &str) {
    let lead = s.chars().take_while(|c| c.is_whitespace()).count();
    let t = s.trim();
    if !t.is_empty() {
        out.push((start + lead, t.to_string()));
    }
}

#[derive(Default)]
struct Parser {
    fc: Flowchart,
    diags: Vec<Diagnostic>,
    stack: Vec<usize>,
    auto_subgraphs: usize,
}

/// Byte cursor over one statement.
struct Cur<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Cur<'a> {
    fn rest(&self) -> &'a str {
        &self.s[self.i..]
    }
    fn eof(&self) -> bool {
        self.i >= self.s.len()
    }
    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }
    fn peek_at(&self, n: usize) -> Option<char> {
        self.rest().chars().nth(n)
    }
    fn bump(&mut self) {
        if let Some(c) = self.peek() {
            self.i += c.len_utf8();
        }
    }
    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }
    fn eat(&mut self, token: &str) -> bool {
        if self.rest().starts_with(token) {
            self.i += token.len();
            true
        } else {
            false
        }
    }
    /// 0-based char column within the statement.
    fn col(&self) -> usize {
        self.s[..self.i].chars().count()
    }
}

struct LinkOp {
    stroke: LinkStroke,
    start: Option<Marker>,
    end: Option<Marker>,
    minlen: u32,
    label: Option<String>,
}

fn is_id_char(c: char) -> bool {
    !c.is_whitespace() && !"[](){}<>|&;:\"@,`=~".contains(c)
}

/// A link token starts here (so an id must end).
fn link_starts(rest: &str) -> bool {
    rest.starts_with("--") || rest.starts_with("==") || rest.starts_with("-.") || rest.starts_with("~~~")
}

type Closers = &'static [(&'static str, Shape)];

/// Shape openers → (closer, shape) candidates, longest opener first.
const OPENERS: &[(&str, Closers)] = &[
    ("(((", &[(")))", Shape::DoubleCircle)]),
    ("((", &[("))", Shape::Circle)]),
    ("([", &[("])", Shape::Stadium)]),
    ("(", &[(")", Shape::Round)]),
    ("[[", &[("]]", Shape::Subroutine)]),
    ("[(", &[(")]", Shape::Cylinder)]),
    ("[/", &[("/]", Shape::LeanRight), ("\\]", Shape::Trapezoid)]),
    ("[\\", &[("\\]", Shape::LeanLeft), ("/]", Shape::InvTrapezoid)]),
    ("[", &[("]", Shape::Rect)]),
    ("{{", &[("}}", Shape::Hexagon)]),
    ("{", &[("}", Shape::Diamond)]),
    (">", &[("]", Shape::Asymmetric)]),
];

impl Parser {
    fn err(&mut self, line: usize, col: usize, msg: impl Into<String>) {
        self.diags.push(Diagnostic::error(line, col + 1, msg));
    }

    fn warn(&mut self, line: usize, col: usize, msg: impl Into<String>) {
        self.diags.push(Diagnostic::warning(line, col + 1, msg));
    }

    fn statement(&mut self, stmt: &str, line: usize, col: usize) {
        let (keyword, rest) = match stmt.split_once(char::is_whitespace) {
            Some((k, r)) => (k, r.trim()),
            None => (stmt, ""),
        };
        match keyword {
            "subgraph" => self.subgraph(rest, line),
            "end" if rest.is_empty() => {
                if self.stack.pop().is_none() {
                    self.err(line, col, "`end` without an open `subgraph`");
                }
            }
            "direction" => match Dir::parse(rest) {
                Some(d) => match self.stack.last() {
                    Some(&top) => self.fc.subgraphs[top].dir = Some(d),
                    None => self.fc.dir = d,
                },
                None => self.err(line, col, format!("unknown direction `{rest}`")),
            },
            "classDef" => {
                let (names, style) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                let spec = StyleSpec::parse(style);
                for name in names.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                    self.fc.class_defs.push((name.to_string(), spec.clone()));
                }
            }
            "class" => {
                let (ids, class) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                let class = class.trim();
                if class.is_empty() {
                    self.err(line, col, "`class` needs node ids and a class name");
                    return;
                }
                for id in ids.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                    if let Some(i) = self.fc.node_index(id) {
                        self.fc.nodes[i].classes.push(class.to_string());
                    } else if let Some(s) = self.fc.subgraph_index(id) {
                        self.fc.subgraphs[s].classes.push(class.to_string());
                    } else {
                        self.warn(line, col, format!("`class` refers to unknown node `{id}`"));
                    }
                }
            }
            "style" => {
                let (id, css) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                let spec = StyleSpec::parse(css);
                if let Some(s) = self.fc.subgraph_index(id) {
                    self.fc.subgraphs[s].style.apply(&spec);
                } else if !id.is_empty() {
                    let i = self.mention(id, line);
                    self.fc.nodes[i].style.apply(&spec);
                }
            }
            "linkStyle" => {
                let (which, css) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                let spec = StyleSpec::parse(css);
                if which == "default" {
                    self.fc.link_styles.push((None, spec));
                } else {
                    for idx in which.split(',').map(str::trim) {
                        match idx.parse::<usize>() {
                            Ok(i) => self.fc.link_styles.push((Some(i), spec.clone())),
                            Err(_) => self.err(line, col, format!("`linkStyle` index `{idx}` is not a number")),
                        }
                    }
                }
            }
            "click" => self.click(rest, line, col),
            _ => self.node_statement(stmt, line, col),
        }
    }

    /// Find or create the node `id`, assigning it to the open subgraph.
    fn mention(&mut self, id: &str, line: usize) -> usize {
        let top = self.stack.last().copied();
        if let Some(i) = self.fc.node_index(id) {
            if self.fc.nodes[i].subgraph.is_none() && top.is_some() {
                self.fc.nodes[i].subgraph = top;
            }
            return i;
        }
        self.fc.nodes.push(Node { id: id.to_string(), subgraph: top, line, ..Node::default() });
        self.fc.nodes.len() - 1
    }

    fn subgraph(&mut self, rest: &str, line: usize) {
        let (id, title) = if rest.is_empty() {
            self.auto_subgraphs += 1;
            (format!("subGraph{}", self.auto_subgraphs - 1), String::new())
        } else if let Some(open) = rest.find('[') {
            let close = rest.rfind(']').unwrap_or(rest.len());
            let id = rest[..open].trim().to_string();
            let title = clean_label(rest.get(open + 1..close).unwrap_or(""));
            let id = if id.is_empty() { title.clone() } else { id };
            (id, title)
        } else {
            let t = clean_label(rest);
            (t.clone(), t)
        };
        // A bare mention of the id before the subgraph existed was a
        // forward reference to the subgraph, not a node.
        if let Some(i) = self.fc.node_index(&id) {
            let n = &self.fc.nodes[i];
            if n.label.is_none() && n.shape == Shape::Rect {
                self.fc.nodes.remove(i);
            } else {
                self.warn(line, 0, format!("`{id}` is used as both a node and a subgraph"));
            }
        }
        self.fc.subgraphs.push(Subgraph { id, title, parent: self.stack.last().copied(), line, ..Subgraph::default() });
        self.stack.push(self.fc.subgraphs.len() - 1);
    }

    fn click(&mut self, rest: &str, line: usize, col: usize) {
        let tokens = tokenize(rest);
        let Some(id) = tokens.first() else {
            self.err(line, col, "`click` needs a node id");
            return;
        };
        let Some(i) = self.fc.node_index(id) else {
            let msg = format!("`click` refers to unknown node `{id}`");
            self.warn(line, col, msg);
            return;
        };
        let (link, k) = match tokens.get(1).map(String::as_str) {
            Some("href") => (tokens.get(2).cloned(), 3),
            Some("call") => (None, 3),
            Some(t) if looks_like_url(t) => (Some(t.to_string()), 2),
            Some(_) => (None, 2),
            None => (None, 1),
        };
        let tooltip = tokens.get(k).filter(|t| !t.starts_with('_')).cloned();
        let node = &mut self.fc.nodes[i];
        if link.is_some() {
            node.link = link;
        }
        if tooltip.is_some() {
            node.tooltip = tooltip;
        }
    }

    fn node_statement(&mut self, stmt: &str, line: usize, col: usize) {
        let mut c = Cur { s: stmt, i: 0 };
        let Some(mut group) = self.node_group(&mut c, line, col) else { return };
        loop {
            c.skip_ws();
            if c.eof() {
                break;
            }
            let edge_id = edge_id_prefix(&mut c);
            let at = c.col();
            let op = match link_op(&mut c) {
                Ok(op) => op,
                Err(msg) => {
                    self.err(line, col + at, msg);
                    return;
                }
            };
            c.skip_ws();
            let Some(next) = self.node_group(&mut c, line, col) else { return };
            for a in &group {
                for b in &next {
                    self.fc.edges.push(Edge {
                        from: a.clone(),
                        to: b.clone(),
                        label: op.label.clone(),
                        stroke: op.stroke,
                        start: op.start,
                        end: op.end,
                        minlen: op.minlen,
                        id: edge_id.clone(),
                        line,
                    });
                }
            }
            group = next;
        }
    }

    fn node_group(&mut self, c: &mut Cur<'_>, line: usize, col: usize) -> Option<Vec<String>> {
        let mut ids = vec![self.node(c, line, col)?];
        loop {
            let save = c.i;
            c.skip_ws();
            if c.eat("&") {
                c.skip_ws();
                ids.push(self.node(c, line, col)?);
            } else {
                c.i = save;
                return Some(ids);
            }
        }
    }

    fn node(&mut self, c: &mut Cur<'_>, line: usize, col: usize) -> Option<String> {
        let start = c.i;
        while let Some(ch) = c.peek() {
            if !is_id_char(ch) || (c.i > start && link_starts(c.rest())) {
                break;
            }
            c.bump();
        }
        let id = c.s[start..c.i].to_string();
        if id.is_empty() {
            let found = c.peek().map_or("end of line".to_string(), |ch| format!("`{ch}`"));
            self.err(line, col + c.col(), format!("expected a node id, found {found}"));
            return None;
        }
        if self.fc.subgraph_index(&id).is_some() {
            return Some(id);
        }
        let idx = self.mention(&id, line);
        let at = c.col();
        if c.rest().starts_with("@{") {
            let Some((inner, used)) = braced(c.rest()) else {
                self.err(line, col + at, "unclosed `@{`");
                return None;
            };
            c.i += used;
            self.apply_attrs(idx, inner, line, col + at);
        } else {
            match shape_text(c) {
                Ok(Some((shape, raw))) => {
                    let node = &mut self.fc.nodes[idx];
                    node.shape = shape;
                    node.markdown = raw.trim().starts_with("\"`");
                    node.label = Some(clean_label(&raw));
                }
                Ok(None) => {}
                Err(msg) => {
                    self.err(line, col + at, msg);
                    return None;
                }
            }
        }
        while c.eat(":::") {
            let s = c.i;
            while c.peek().is_some_and(|ch| ch.is_alphanumeric() || ch == '_' || ch == '-') {
                c.bump();
            }
            let class = c.s[s..c.i].to_string();
            if !class.is_empty() {
                self.fc.nodes[idx].classes.push(class);
            }
        }
        Some(id)
    }

    fn apply_attrs(&mut self, idx: usize, inner: &str, line: usize, col: usize) {
        let yaml = format!("{{{inner}}}");
        let map = match serde_yaml::from_str::<serde_json::Value>(&yaml) {
            Ok(serde_json::Value::Object(m)) => m,
            _ => {
                self.err(line, col, "invalid `@{…}` node data (expected `key: value, …`)");
                return;
            }
        };
        if let Some(shape) = map.get("shape").and_then(|v| v.as_str()) {
            match Shape::from_name(shape) {
                Some(s) => self.fc.nodes[idx].shape = s,
                None => self.warn(line, col, format!("unknown shape `{shape}`; drawn as a rectangle")),
            }
        }
        if let Some(label) = map.get("label").and_then(|v| v.as_str()) {
            let node = &mut self.fc.nodes[idx];
            node.markdown = label.trim().starts_with('`');
            node.label = Some(clean_label(label));
        }
    }
}

/// `e1@-->`: an edge id directly before the link token.
fn edge_id_prefix(c: &mut Cur<'_>) -> Option<String> {
    let rest = c.rest();
    let at = rest.find('@')?;
    let id = &rest[..at];
    if id.is_empty() || !id.chars().all(|ch| ch.is_alphanumeric() || ch == '_') {
        return None;
    }
    let after = &rest[at + 1..];
    if link_starts(after) || after.starts_with('<') {
        c.i += at + 1;
        Some(id.to_string())
    } else {
        None
    }
}

fn marker_char(ch: Option<char>) -> Option<Marker> {
    match ch {
        Some('>') => Some(Marker::Arrow),
        Some('o') => Some(Marker::Circle),
        Some('x') => Some(Marker::Cross),
        _ => None,
    }
}

/// After a run of `-`/`=`: is `>`/`o`/`x` a marker (not the next node's id)?
fn ox_is_marker(c: &Cur<'_>, run: usize) -> bool {
    match c.peek() {
        Some('>') => true,
        Some('o') | Some('x') => {
            run == 2 || c.peek_at(1).is_none_or(|n| n.is_whitespace() || n == '|' || n == ';')
        }
        _ => false,
    }
}

fn run_of(c: &mut Cur<'_>, ch: char) -> usize {
    let n = c.rest().chars().take_while(|&x| x == ch).count();
    c.i += n * ch.len_utf8();
    n
}

fn link_op(c: &mut Cur<'_>) -> Result<LinkOp, String> {
    let mut start = None;
    match c.peek() {
        Some('<') => {
            start = Some(Marker::Arrow);
            c.bump();
        }
        Some('o') | Some('x') if matches!(c.peek_at(1), Some('-') | Some('=')) => {
            start = marker_char(c.peek());
            c.bump();
        }
        _ => {}
    }
    if c.rest().starts_with("~~~") {
        let n = run_of(c, '~');
        return Ok(LinkOp { stroke: LinkStroke::Invisible, start: None, end: None, minlen: (n - 2) as u32, label: None });
    }
    let Some(ch) = c.peek().filter(|&ch| ch == '-' || ch == '=') else {
        return Err(format!(
            "expected a link such as `-->`, found `{}`",
            c.rest().chars().take(8).collect::<String>()
        ));
    };
    let mut op = if ch == '-' && c.peek_at(1) == Some('.') {
        dotted(c)?
    } else {
        let n = run_of(c, ch);
        let stroke = if ch == '=' { LinkStroke::Thick } else { LinkStroke::Normal };
        let (label, run) = if n == 2 && c.peek().is_some_and(char::is_whitespace) {
            // `-- text -->` / `== text ==>`
            let closer = if ch == '=' { "==" } else { "--" };
            let rest = c.rest();
            let Some(pos) = rest.find(closer) else {
                return Err(format!("link text after `{closer}` is never closed with `{closer}>`"));
            };
            let label = clean_label(rest[..pos].trim());
            c.i += pos;
            (Some(label), run_of(c, ch))
        } else {
            (None, n)
        };
        let end = if ox_is_marker(c, run) { marker_char(c.peek()) } else { None };
        if end.is_some() {
            c.bump();
        }
        if run < 2 || (end.is_none() && run < 3) {
            return Err("link needs an arrow head (`-->`) or a third dash (`---`)".into());
        }
        let minlen = if end.is_some() { run - 1 } else { run - 2 };
        LinkOp { stroke, start, end, minlen: minlen.max(1) as u32, label }
    };
    op.start = op.start.or(start);
    // `-->|label|`
    let save = c.i;
    c.skip_ws();
    if c.eat("|") {
        let rest = c.rest();
        // A quoted label may itself contain `|`.
        let from = match rest.trim_start().strip_prefix('"') {
            Some(q) => q.find('"').map_or(0, |e| rest.len() - q.len() + e + 1),
            None => 0,
        };
        let Some(end) = rest[from..].find('|').map(|e| e + from) else {
            return Err("link label `|…` is never closed with `|`".into());
        };
        op.label = Some(clean_label(&rest[..end]));
        c.i += end + 1;
    } else {
        c.i = save;
    }
    Ok(op)
}

/// `-.->`, `-..-`, `-. text .->`.
fn dotted(c: &mut Cur<'_>) -> Result<LinkOp, String> {
    c.bump(); // '-'
    let mut label = None;
    let mut dots = run_of(c, '.');
    if c.peek().is_some_and(char::is_whitespace) {
        let rest = c.rest();
        let Some(pos) = rest.find(".-") else {
            return Err("dotted link text is never closed with `.->`".into());
        };
        label = Some(clean_label(rest[..pos].trim()));
        c.i += pos;
        dots = run_of(c, '.');
    }
    if !c.eat("-") {
        return Err("dotted link must end with `-` (`-.->` or `-.-`)".into());
    }
    let end = if ox_is_marker(c, 2) { marker_char(c.peek()) } else { None };
    if end.is_some() {
        c.bump();
    }
    Ok(LinkOp { stroke: LinkStroke::Dotted, start: None, end, minlen: dots.max(1) as u32, label })
}

/// Parses a bracketed node text at the cursor: `Ok(None)` when no shape
/// follows, `Err` when it is malformed.
fn shape_text(c: &mut Cur<'_>) -> Result<Option<(Shape, String)>, String> {
    let rest = c.rest();
    let Some(&(open, closers)) = OPENERS.iter().find(|(o, _)| rest.starts_with(o)) else {
        return Ok(None);
    };
    let body = &rest[open.len()..];
    let lead = body.len() - body.trim_start().len();
    let (content, after) = if body[lead..].starts_with('"') {
        let q = &body[lead + 1..];
        let Some(end) = q.find('"') else {
            return Err("unclosed `\"` in node text".into());
        };
        let split = lead + 1 + end + 1;
        (&body[..split], &body[split..])
    } else {
        let first = closers.iter().filter_map(|(cl, _)| body.find(cl)).min();
        let Some(pos) = first else {
            return Err(format!("`{open}` is never closed with `{}`", closers[0].0));
        };
        (&body[..pos], &body[pos..])
    };
    let after_trim = after.trim_start();
    let Some(&(closer, shape)) = closers.iter().find(|(cl, _)| after_trim.starts_with(cl)) else {
        return Err(format!("expected `{}` to close the node text", closers[0].0));
    };
    c.i += open.len() + content.len() + (after.len() - after_trim.len()) + closer.len();
    Ok(Some((shape, content.to_string())))
}

/// `@{ … }` with nesting and quotes → (inner text, bytes consumed).
fn braced(s: &str) -> Option<(&str, usize)> {
    let body = s.strip_prefix("@{")?;
    let mut depth = 1;
    let mut quoted = false;
    for (i, ch) in body.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            '{' if !quoted => depth += 1,
            '}' if !quoted => {
                depth -= 1;
                if depth == 0 {
                    return Some((&body[..i], 2 + i + 1));
                }
            }
            _ => {}
        }
    }
    None
}

/// Whitespace-separated words, `"quoted strings"` kept whole (unquoted).
fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            out.push(chars.by_ref().take_while(|&x| x != '"').collect());
        } else {
            let mut t = String::new();
            while let Some(&x) = chars.peek() {
                if x.is_whitespace() {
                    break;
                }
                t.push(x);
                chars.next();
            }
            out.push(t);
        }
    }
    out
}

fn looks_like_url(t: &str) -> bool {
    t.contains("://") || t.starts_with('/') || t.starts_with('#') || t.starts_with("mailto:") || t.contains("[[")
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
