//! `mnemonic-cli`: command-line and MCP front end to a vault (§Fase 2
//! "agent-harness friendliness"). Every subcommand is a thin wrapper over
//! `mnemonic::api::VaultService`; `--json` prints the service's result
//! verbatim (stable contract, see `docs/agent-interface.md`), otherwise a
//! human-readable rendering. Errors go to stderr with a non-zero exit;
//! logs go to stderr via `env_logger` (`RUST_LOG=info` for model loading
//! progress) so stdout stays parseable.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use serde::Serialize;

use mnemonic::api::types::*;
use mnemonic::api::{VaultService, diagram, mcp};

#[derive(Parser)]
#[command(name = "mnemonic-cli", version, about = "Agent interface to a MNEMONIC vault")]
struct Cli {
    /// Vault folder (default: the vault last opened in the desktop app).
    #[arg(long, global = true, env = "MNEMONIC_VAULT")]
    vault: Option<PathBuf>,
    /// Print results as JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List, read, write, create or trash notes.
    Notes {
        #[command(subcommand)]
        action: NotesAction,
    },
    /// Hybrid keyword + semantic search over indexed notes.
    Search {
        query: String,
        /// Number of hits.
        #[arg(short, long, default_value_t = 10)]
        k: usize,
        /// Skip the vector route (no embedding model needed).
        #[arg(long)]
        keyword_only: bool,
    },
    /// Notes linking to REF.
    Backlinks { r#ref: String },
    /// Outgoing [[wikilinks]] of REF.
    Links { r#ref: String },
    /// The whole link graph.
    Graph,
    /// Rescan the vault and (re)chunk + embed changed notes.
    Index {
        /// Re-chunk every note even if unchanged.
        #[arg(long)]
        full: bool,
        /// Keyword (FTS) index only; never loads the embedding model.
        #[arg(long)]
        keyword_only: bool,
    },
    /// Ask the local LLM a question grounded in the vault (RAG).
    Ask {
        question: String,
        #[arg(long, default_value_t = mnemonic::llm::DEFAULT_MAX_TOKENS)]
        max_tokens: usize,
    },
    /// Validate or render Mermaid diagrams (```mermaid fences or .mmd files).
    Diagram {
        #[command(subcommand)]
        action: DiagramAction,
    },
    /// List, read, query or edit CSV/XLSX sheets (§3.8; XLSX is read-only).
    Sheets {
        #[command(subcommand)]
        action: SheetsAction,
    },
    /// Reading themes (§3.2.5): built-ins and plugin *.toml files.
    Themes {
        #[command(subcommand)]
        action: ThemesAction,
    },
    /// Run as an MCP server over stdio (JSON-RPC 2.0, newline-delimited).
    Mcp,
}

#[derive(Subcommand)]
enum ThemesAction {
    /// Every theme with its id, name and source, plus the plugin folders.
    List,
}

#[derive(Subcommand)]
enum SheetsAction {
    /// Every CSV/TSV/XLSX/… file in the vault.
    List,
    /// Headers, a page of rows and per-column stats.
    Read {
        /// Vault-relative path or unambiguous file name/stem.
        r#ref: String,
        /// Worksheet name or 0-based index.
        #[arg(long)]
        sheet: Option<String>,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Filter + sort rows; stats cover every matched row.
    Query {
        r#ref: String,
        #[arg(long)]
        sheet: Option<String>,
        /// Condition COLUMN:OP:VALUE (repeatable, all must hold). OP: eq, ne,
        /// contains, not_contains, gt, gte, lt, lte, empty, not_empty.
        #[arg(long = "where", value_name = "COLUMN:OP:VALUE")]
        filters: Vec<String>,
        /// Columns to show (repeatable).
        #[arg(long = "column")]
        columns: Vec<String>,
        #[arg(long)]
        sort: Option<String>,
        #[arg(long)]
        desc: bool,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Set one cell of a CSV (ROW is 1-based, COLUMN a name or index).
    Set {
        r#ref: String,
        row: usize,
        column: String,
        value: String,
    },
    /// Append rows given as JSON arrays or objects (repeatable).
    Append {
        r#ref: String,
        #[arg(long = "row", value_name = "JSON", required = true)]
        rows: Vec<String>,
    },
    /// Create a new CSV/TSV with the given headers.
    Create {
        path: String,
        #[arg(long = "header", required = true)]
        headers: Vec<String>,
    },
}

#[derive(Subcommand)]
enum DiagramAction {
    /// List the mermaid diagrams of a note, each validated.
    List { r#ref: String },
    /// Check diagram syntax; exits non-zero when it has errors.
    Validate {
        #[command(flatten)]
        input: DiagramInput,
    },
    /// Render a diagram to SVG (stdout, or --out FILE).
    Render {
        #[command(flatten)]
        input: DiagramInput,
        /// Write the SVG here instead of stdout.
        #[arg(long, short, value_name = "FILE")]
        out: Option<PathBuf>,
        /// Use the dark theme (unless the diagram sets its own).
        #[arg(long)]
        dark: bool,
    },
}

#[derive(Args)]
struct DiagramInput {
    /// Diagram source file (e.g. flow.mmd); no vault needed.
    #[arg(long, value_name = "FILE", conflicts_with_all = ["stdin", "note"])]
    file: Option<PathBuf>,
    /// Read the diagram source from standard input; no vault needed.
    #[arg(long, conflicts_with = "note")]
    stdin: bool,
    /// Note holding the diagram (title, alias, path or UUID).
    #[arg(long)]
    note: Option<String>,
    /// Which mermaid fence of the note (0-based).
    #[arg(long, default_value_t = 0)]
    index: usize,
}

impl DiagramInput {
    /// Raw source for --file/--stdin; `None` means "read it from --note".
    fn raw_source(&self) -> Result<Option<String>> {
        if let Some(path) = &self.file {
            return std::fs::read_to_string(path)
                .with_context(|| format!("reading diagram file {}", path.display()))
                .map(Some);
        }
        if self.stdin {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s).context("reading diagram from stdin")?;
            return Ok(Some(s));
        }
        if self.note.is_none() {
            bail!("give the diagram with --file, --stdin or --note");
        }
        Ok(None)
    }
}

impl DiagramAction {
    fn needs_vault(&self) -> bool {
        match self {
            DiagramAction::List { .. } => true,
            DiagramAction::Validate { input } | DiagramAction::Render { input, .. } => input.note.is_some(),
        }
    }
}

#[derive(Subcommand)]
enum NotesAction {
    List {
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        folder: Option<String>,
        #[arg(long)]
        include_trashed: bool,
    },
    Read {
        r#ref: String,
    },
    /// Replace the body and/or tags of REF (unknown frontmatter keys survive).
    Write {
        r#ref: String,
        #[command(flatten)]
        body: BodySource,
        /// Replace the tag list (repeatable).
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Create the note (titled REF) when it doesn't exist.
        #[arg(long)]
        create: bool,
        /// Folder for a created note.
        #[arg(long)]
        folder: Option<String>,
    },
    Create {
        title: String,
        #[arg(long)]
        folder: Option<String>,
        #[command(flatten)]
        body: BodySource,
        #[arg(long = "tag")]
        tags: Vec<String>,
    },
    /// Move REF into .trash/ (soft delete).
    Trash {
        r#ref: String,
    },
    /// Export REF as HTML or PDF in a reading theme's colours (§3.2.5).
    Export {
        r#ref: String,
        /// html or pdf (PDF needs Chrome/Edge/Chromium/Brave).
        #[arg(long, default_value = "html")]
        format: String,
        /// Theme id (see `themes list`); default: the note's `theme:`.
        #[arg(long)]
        theme: Option<String>,
        /// Output file (absolute or vault-relative). HTML without --out
        /// is printed to stdout.
        #[arg(long)]
        out: Option<String>,
    },
}

#[derive(Args)]
#[group(multiple = false)]
struct BodySource {
    /// Read the body from this file.
    #[arg(long, value_name = "FILE")]
    body_file: Option<PathBuf>,
    /// Read the body from standard input.
    #[arg(long)]
    stdin: bool,
}

impl BodySource {
    fn read(&self) -> Result<Option<String>> {
        if let Some(path) = &self.body_file {
            return std::fs::read_to_string(path)
                .with_context(|| format!("reading body file {}", path.display()))
                .map(Some);
        }
        if self.stdin {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .context("reading body from stdin")?;
            return Ok(Some(s));
        }
        Ok(None)
    }
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .target(env_logger::Target::Stderr)
        .init();
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn resolve_vault(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(v) = explicit {
        return Ok(v);
    }
    match mnemonic::settings::load().vault_path {
        Some(p) if !p.trim().is_empty() => Ok(PathBuf::from(p)),
        _ => bail!("no vault given: pass --vault <PATH> (or set MNEMONIC_VAULT), or open one in the app first"),
    }
}

fn run(cli: Cli) -> Result<()> {
    let json = cli.json;
    if let Command::Diagram { action } = &cli.command
        && !action.needs_vault()
    {
        return run_diagram(json, action, None);
    }
    let root = resolve_vault(cli.vault)?;
    let mut svc = VaultService::open(&root)?;

    match cli.command {
        Command::Notes { action } => match action {
            NotesAction::List {
                tag,
                folder,
                include_trashed,
            } => {
                let notes = svc.list_notes(&NoteFilter {
                    tag,
                    folder,
                    include_trashed,
                });
                emit(json, &notes, |out| {
                    for n in &notes {
                        let tags = if n.tags.is_empty() {
                            String::new()
                        } else {
                            format!("  [{}]", n.tags.join(", "))
                        };
                        out.push(format!("{}\t{}{}", n.path, n.title, tags));
                    }
                    if notes.is_empty() {
                        out.push("(no notes)".into());
                    }
                })
            }
            NotesAction::Read { r#ref } => {
                let note = svc.read_note(&r#ref)?;
                emit(json, &note, |out| {
                    out.push(format!("title: {}", note.summary.title));
                    out.push(format!("path: {}", note.summary.path));
                    out.push(format!("id: {}", note.summary.id));
                    out.push(format!("tags: {}", note.summary.tags.join(", ")));
                    if !note.summary.aliases.is_empty() {
                        out.push(format!("aliases: {}", note.summary.aliases.join(", ")));
                    }
                    out.push(format!("modified: {}", note.summary.modified.to_rfc3339()));
                    for (k, v) in &note.extra {
                        out.push(format!("{k}: {v}"));
                    }
                    out.push(String::new());
                    out.push(note.body.clone());
                })
            }
            NotesAction::Write {
                r#ref,
                body,
                tags,
                create,
                folder,
            } => {
                let body = body.read()?;
                if body.is_none() && tags.is_empty() && !create {
                    bail!("nothing to write: pass --body-file, --stdin and/or --tag");
                }
                let res = svc.write_note(&WriteNoteRequest {
                    reference: r#ref,
                    body,
                    folder,
                    tags: if tags.is_empty() { None } else { Some(tags) },
                    create_if_missing: create,
                })?;
                emit(json, &res, |out| {
                    out.push(format!(
                        "{} {} ({})",
                        if res.created { "created" } else { "updated" },
                        res.note.path,
                        res.note.id
                    ));
                    out.extend(res.warnings.iter().map(|w| format!("warning: {w}")));
                })
            }
            NotesAction::Create {
                title,
                folder,
                body,
                tags,
            } => {
                let res = svc.create_note(&CreateNoteRequest {
                    title,
                    body: body.read()?.unwrap_or_default(),
                    folder,
                    tags,
                })?;
                emit(json, &res, |out| out.push(format!("created {} ({})", res.path, res.id)))
            }
            NotesAction::Export { r#ref, format, theme, out } => {
                let res = svc.export_note(&ExportRequest { r#ref, format, theme, out })?;
                match (&res.html, json) {
                    (Some(html), false) => {
                        print!("{html}");
                        Ok(())
                    }
                    _ => emit(json, &res, |out| {
                        out.push(format!(
                            "exported {} as {} ({} theme) -> {} ({} bytes)",
                            res.note,
                            res.format,
                            res.theme,
                            res.path.as_deref().unwrap_or("-"),
                            res.bytes
                        ))
                    }),
                }
            }
            NotesAction::Trash { r#ref } => {
                let res = svc.trash_note(&r#ref)?;
                emit(json, &res, |out| {
                    out.push(format!("trashed {} -> {}", res.previous_path, res.path))
                })
            }
        },
        Command::Search {
            query,
            k,
            keyword_only,
        } => {
            let res = svc.search(&SearchRequest {
                query,
                k,
                semantic: !keyword_only,
            })?;
            emit(json, &res, |out| {
                out.extend(res.warnings.iter().map(|w| format!("warning: {w}")));
                if res.hits.is_empty() {
                    out.push("(no hits)".into());
                }
                for (i, h) in res.hits.iter().enumerate() {
                    let score = h.score.map(|s| format!(" score={s:.3}")).unwrap_or_default();
                    let page = h
                        .page
                        .map(|p| format!(" p.{p}"))
                        .or_else(|| h.row.map(|r| format!(" row {r}")))
                        .unwrap_or_default();
                    out.push(format!("{}. {} — {}{page} [{}]{score}", i + 1, h.title, h.path, h.kind));
                    let preview = h.snippet.clone().unwrap_or_else(|| first_line(&h.text));
                    out.push(format!("   {}", preview.replace('\n', " ")));
                }
            })
        }
        Command::Backlinks { r#ref } => {
            let res = svc.backlinks(&r#ref)?;
            emit(json, &res, |out| {
                out.push(format!("backlinks to {} ({}):", res.target.title, res.target.path));
                for b in &res.backlinks {
                    out.push(format!("  {}:{}: {}", b.source_path, b.line, b.context.trim()));
                }
                if res.backlinks.is_empty() {
                    out.push("  (none)".into());
                }
            })
        }
        Command::Links { r#ref } => {
            let res = svc.outgoing_links(&r#ref)?;
            emit(json, &res, |out| {
                out.push(format!("links from {} ({}):", res.source.title, res.source.path));
                for l in &res.links {
                    let target = match &l.heading {
                        Some(h) => format!("{}#{h}", l.target),
                        None => l.target.clone(),
                    };
                    let resolved = l.resolved_path.as_deref().unwrap_or("(unresolved)");
                    out.push(format!("  line {}: [[{target}]] -> {resolved}", l.line));
                }
                if res.links.is_empty() {
                    out.push("  (none)".into());
                }
            })
        }
        Command::Graph => {
            let g = svc.graph()?;
            emit(json, &g, |out| {
                out.push(format!("{} nodes, {} edges", g.nodes.len(), g.edges.len()));
                for n in &g.nodes {
                    out.push(format!("  [{}] {} (degree {})", n.kind, n.label, n.degree));
                }
                for e in &g.edges {
                    let a = g.nodes.get(e.a).map(|n| n.label.as_str()).unwrap_or("?");
                    let b = g.nodes.get(e.b).map(|n| n.label.as_str()).unwrap_or("?");
                    out.push(format!("  {a} --{}--> {b} (x{})", e.kind, e.weight));
                }
            })
        }
        Command::Index { full, keyword_only } => {
            let report = svc.reindex(ReindexOptions { full, keyword_only })?;
            emit(json, &report, |out| {
                out.push(format!(
                    "indexed {} notes: {} chunked, {} up to date, {} pruned; {} sheets, {} chunked ({})",
                    report.notes_indexed,
                    report.chunked,
                    report.skipped,
                    report.pruned,
                    report.sheets_indexed,
                    report.sheets_chunked,
                    if report.semantic { "with embeddings" } else { "keyword-only" }
                ));
                out.extend(report.warnings.iter().map(|w| format!("warning: {w}")));
                out.extend(report.failed.iter().map(|f| format!("failed: {}: {}", f.path, f.error)));
            })
        }
        Command::Ask { question, max_tokens } => {
            let res = svc.ask(&AskRequest { question, max_tokens })?;
            emit(json, &res, |out| {
                out.extend(res.warnings.iter().map(|w| format!("warning: {w}")));
                out.push(res.answer.clone());
                if !res.citations.is_empty() {
                    out.push(String::new());
                    out.push("sources:".into());
                    for c in &res.citations {
                        let page = c
                            .page
                            .map(|p| format!(" (page {p})"))
                            .or_else(|| c.row.map(|r| format!(" (row {r})")))
                            .unwrap_or_default();
                        out.push(format!("  - {} — {}{page}", c.title, c.path));
                    }
                }
            })
        }
        Command::Diagram { action } => run_diagram(json, &action, Some(&svc)),
        Command::Sheets { action } => run_sheets(json, action, &mut svc),
        Command::Themes { action: ThemesAction::List } => {
            let list = svc.list_themes();
            emit(json, &list, |out| {
                for t in &list.themes {
                    let by = t.author.as_deref().map(|a| format!(" by {a}")).unwrap_or_default();
                    out.push(format!("{:<16} {}{by} [{}]", t.id, t.name, t.source));
                }
                out.push(format!("plugin folders: {}", list.plugin_dirs.join(", ")));
                out.extend(list.problems.iter().map(|p| format!("warning: {p}")));
            })
        }
        Command::Mcp => {
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            mcp::serve(&mut svc, stdin.lock(), stdout.lock())
        }
    }
}

fn diagnostic_lines(out: &mut Vec<String>, diags: &[DiagramDiagnostic]) {
    for d in diags {
        let at = match d.note_line {
            Some(n) => format!("note line {n}, col {}", d.col),
            None => format!("{}:{}", d.line, d.col),
        };
        out.push(format!("  {at}: {}: {}", d.severity, d.message));
    }
}

/// `diagram …`; `svc` is `None` for raw --file/--stdin sources.
fn run_diagram(json: bool, action: &DiagramAction, svc: Option<&VaultService>) -> Result<()> {
    let request = |input: &DiagramInput, dark: bool| -> Result<DiagramRequest> {
        Ok(DiagramRequest { source: input.raw_source()?, r#ref: input.note.clone(), index: input.index, dark })
    };
    match action {
        DiagramAction::List { r#ref } => {
            let svc = svc.context("`diagram list` needs a vault")?;
            let list = svc.list_diagrams(r#ref)?;
            emit(json, &list, |out| {
                out.push(format!("{}: {} diagram(s)", list.path, list.diagrams.len()));
                for d in &list.diagrams {
                    let state = if !d.check.supported {
                        "unsupported"
                    } else if d.check.valid {
                        "ok"
                    } else {
                        "errors"
                    };
                    out.push(format!("[{}] line {} {} — {state}", d.index, d.line, d.check.kind));
                    diagnostic_lines(out, &d.check.diagnostics);
                }
            })
        }
        DiagramAction::Validate { input } => {
            let req = request(input, false)?;
            let check = match svc {
                Some(s) => s.validate_diagram(&req)?,
                None => diagram::check_source(req.source.as_deref().unwrap_or(""), None),
            };
            emit(json, &check, |out| {
                out.push(format!("{}: {}", check.kind, if check.valid { "ok" } else { "invalid" }));
                diagnostic_lines(out, &check.diagnostics);
            })?;
            if !check.valid {
                bail!("diagram has errors");
            }
            Ok(())
        }
        DiagramAction::Render { input, out, dark } => {
            let req = request(input, *dark)?;
            let rendered = match svc {
                Some(s) => s.render_diagram(&req)?,
                None => diagram::render_source(req.source.as_deref().unwrap_or(""), *dark, None)?,
            };
            if let Some(path) = out {
                std::fs::write(path, &rendered.svg).with_context(|| format!("writing {}", path.display()))?;
            }
            if json {
                emit(true, &rendered, |_| {})
            } else if out.is_none() {
                print!("{}", rendered.svg);
                Ok(())
            } else {
                let mut lines = vec![format!("{} {:.0}×{:.0}", rendered.kind, rendered.width, rendered.height)];
                diagnostic_lines(&mut lines, &rendered.diagnostics);
                println!("{}", lines.join("\n"));
                Ok(())
            }
        }
    }
}

/// `sheets …`.
fn run_sheets(json: bool, action: SheetsAction, svc: &mut VaultService) -> Result<()> {
    match action {
        SheetsAction::List => {
            let sheets = svc.list_sheets();
            emit(json, &sheets, |out| {
                for s in &sheets {
                    let ro = if s.editable { "" } else { "  (read-only)" };
                    out.push(format!("{}\t{}\t{} B{ro}", s.path, s.kind, s.size_bytes));
                }
                if sheets.is_empty() {
                    out.push("(no sheets)".into());
                }
            })
        }
        SheetsAction::Read { r#ref, sheet, offset, limit } => {
            let data = svc.read_sheet(&SheetReadRequest { r#ref, sheet, offset, limit })?;
            emit(json, &data, |out| {
                out.push(format!("{} [{}] — {} rows", data.path, data.sheet, data.total_rows));
                table_lines(out, &data.headers, &data.rows, &data.columns);
            })
        }
        SheetsAction::Query { r#ref, sheet, filters, columns, sort, desc, limit } => {
            let filters = filters
                .iter()
                .map(|f| {
                    let mut parts = f.splitn(3, ':');
                    let column = parts.next().unwrap_or_default().to_string();
                    let op = parts.next().context("--where needs COLUMN:OP[:VALUE]")?.to_string();
                    let value = parts.next().unwrap_or_default().to_string();
                    Ok(SheetFilter { column, op, value })
                })
                .collect::<Result<Vec<_>>>()?;
            let res = svc.query_sheet(&SheetQueryRequest {
                r#ref,
                sheet,
                filters,
                columns: (!columns.is_empty()).then_some(columns),
                sort_by: sort,
                descending: desc,
                limit,
            })?;
            emit(json, &res, |out| {
                out.push(format!("{} [{}] — {} matched", res.path, res.sheet, res.matched));
                table_lines(out, &res.headers, &res.rows, &res.columns);
            })
        }
        SheetsAction::Set { r#ref, row, column, value } => {
            let res = svc.set_sheet_cell(&SheetSetCellRequest { r#ref, row, column, value })?;
            emit(json, &res, |out| out.push(format!("{}: {} cell(s) changed", res.path, res.changed)))
        }
        SheetsAction::Append { r#ref, rows } => {
            let rows = rows
                .iter()
                .map(|r| serde_json::from_str(r).with_context(|| format!("--row is not JSON: {r}")))
                .collect::<Result<Vec<_>>>()?;
            let res = svc.append_sheet_rows(&SheetAppendRequest { r#ref, rows })?;
            emit(json, &res, |out| out.push(format!("{}: now {} rows", res.path, res.rows)))
        }
        SheetsAction::Create { path, headers } => {
            let res = svc.create_sheet(&SheetCreateRequest { path, headers, rows: Vec::new() })?;
            emit(json, &res, |out| out.push(format!("created {}", res.path)))
        }
    }
}

/// `row | a | b` lines plus a stats line per numeric column.
fn table_lines(out: &mut Vec<String>, headers: &[String], rows: &[SheetRowOut], cols: &[SheetColumnOut]) {
    out.push(format!("#\t{}", headers.join("\t")));
    for r in rows {
        out.push(format!("{}\t{}", r.row, r.cells.join("\t")));
    }
    for c in cols.iter().filter(|c| c.numeric) {
        let n = |v: Option<f64>| v.map(mnemonic::sheet::model::format_number).unwrap_or_default();
        out.push(format!(
            "{}: count {} · sum {} · avg {} · min {} · max {}",
            c.name,
            c.count,
            n(c.sum),
            n(c.avg),
            n(c.min),
            n(c.max)
        ));
    }
}

/// Prints `value` as JSON, or the lines produced by `text`.
fn emit<T: Serialize>(json: bool, value: &T, text: impl FnOnce(&mut Vec<String>)) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        let mut lines = Vec::new();
        text(&mut lines);
        println!("{}", lines.join("\n"));
    }
    Ok(())
}

fn first_line(s: &str) -> String {
    let line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let mut out: String = line.chars().take(160).collect();
    if out.len() < line.len() {
        out.push('…');
    }
    out
}
