//! MCP server (`mnemonic-cli mcp`, §Fase 2): Model Context Protocol over
//! stdio, hand-rolled JSON-RPC 2.0 — one JSON object per line, newline
//! delimited, no framing headers, no extra dependency. Implements
//! `initialize`, `notifications/initialized`, `ping`, `tools/list` and
//! `tools/call`; every tool (notes, search, diagrams, sheets) maps 1:1
//! onto a `VaultService` method and
//! returns its JSON as a single `text` content block. stdout carries
//! protocol messages only — diagnostics go through `log` (stderr).
//! Callers: `src/bin/mnemonic-cli.rs`.

use std::io::{BufRead, Write};

use anyhow::Result;
use serde_json::{Value, json};

use super::service::VaultService;
use super::types::*;

pub const PROTOCOL_VERSION: &str = "2025-06-18";
pub const SERVER_NAME: &str = "mnemonic";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// Reads requests line by line from `input` until EOF, writing one reply
/// line per request (notifications get none) to `output`.
pub fn serve(service: &mut VaultService, input: impl BufRead, mut output: impl Write) -> Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle_message(service, &line) {
            serde_json::to_writer(&mut output, &reply)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

/// Handles one raw JSON-RPC message; `None` means nothing to send back
/// (a notification, or a reply-less error on a malformed notification).
pub fn handle_message(service: &mut VaultService, raw: &str) -> Option<Value> {
    let msg: Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(e) => return Some(error_reply(Value::Null, PARSE_ERROR, &format!("parse error: {e}"))),
    };
    let Some(obj) = msg.as_object() else {
        return Some(error_reply(Value::Null, INVALID_REQUEST, "request must be an object"));
    };
    let id = obj.get("id").cloned();
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        return Some(error_reply(
            id.unwrap_or(Value::Null),
            INVALID_REQUEST,
            "missing method",
        ));
    };
    let params = obj.get("params").cloned().unwrap_or(Value::Null);
    let is_notification = id.is_none();
    let result = dispatch(service, method, params);
    if is_notification {
        if let Err((code, msg)) = &result {
            log::warn!("mcp: notification {method} failed ({code}): {msg}");
        }
        return None;
    }
    let id = id.unwrap_or(Value::Null);
    Some(match result {
        Ok(v) => json!({ "jsonrpc": "2.0", "id": id, "result": v }),
        Err((code, msg)) => error_reply(id, code, &msg),
    })
}

fn error_reply(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn dispatch(service: &mut VaultService, method: &str, params: Value) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
            "instructions": format!(
                "MNEMONIC vault at {}. Notes are Markdown files with YAML frontmatter; \
                 refer to a note by title, alias, vault-relative path or id. Spreadsheets \
                 (CSV editable, XLSX read-only) have *_sheet tools; use query_sheet for \
                 totals and filters. Run `reindex` before `search_notes`/`ask_vault` if \
                 results look stale.",
                service.root().display()
            ),
        })),
        "notifications/initialized" | "notifications/cancelled" | "notifications/roots/list_changed" => {
            Ok(Value::Null)
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or((INVALID_PARAMS, "tools/call needs a `name`".to_string()))?;
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            if !TOOL_NAMES.contains(&name) {
                return Err((INVALID_PARAMS, format!("unknown tool: {name}")));
            }
            Ok(match call_tool(service, name, args) {
                Ok(v) => tool_result(&v, false),
                Err(e) => tool_result(&json!({ "error": format!("{e:#}") }), true),
            })
        }
        _ => Err((METHOD_NOT_FOUND, format!("method not found: {method}"))),
    }
}

fn tool_result(payload: &Value, is_error: bool) -> Value {
    let text = serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string());
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

const TOOL_NAMES: &[&str] = &[
    "list_notes",
    "read_note",
    "write_note",
    "create_note",
    "trash_note",
    "search_notes",
    "get_backlinks",
    "get_links",
    "get_graph",
    "reindex",
    "ask_vault",
    "list_diagrams",
    "validate_diagram",
    "render_diagram",
    "list_sheets",
    "read_sheet",
    "query_sheet",
    "set_sheet_cell",
    "append_sheet_rows",
    "create_sheet",
    "list_themes",
    "export_note",
];

fn parse_args<T: serde::de::DeserializeOwned>(args: Value) -> Result<T> {
    serde_json::from_value(args).map_err(|e| anyhow::anyhow!("invalid arguments: {e}"))
}

#[derive(serde::Deserialize)]
struct RefArgs {
    #[serde(alias = "reference", alias = "title", alias = "path")]
    r#ref: String,
}

fn call_tool(service: &mut VaultService, name: &str, args: Value) -> Result<Value> {
    let out = match name {
        "list_notes" => serde_json::to_value(service.list_notes(&parse_args::<NoteFilter>(args)?))?,
        "read_note" => serde_json::to_value(service.read_note(&parse_args::<RefArgs>(args)?.r#ref)?)?,
        "write_note" => serde_json::to_value(service.write_note(&parse_args::<WriteNoteRequest>(args)?)?)?,
        "create_note" => serde_json::to_value(service.create_note(&parse_args::<CreateNoteRequest>(args)?)?)?,
        "trash_note" => serde_json::to_value(service.trash_note(&parse_args::<RefArgs>(args)?.r#ref)?)?,
        "search_notes" => serde_json::to_value(service.search(&parse_args::<SearchRequest>(args)?)?)?,
        "get_backlinks" => serde_json::to_value(service.backlinks(&parse_args::<RefArgs>(args)?.r#ref)?)?,
        "get_links" => serde_json::to_value(service.outgoing_links(&parse_args::<RefArgs>(args)?.r#ref)?)?,
        "get_graph" => serde_json::to_value(service.graph()?)?,
        "reindex" => serde_json::to_value(service.reindex(parse_args::<ReindexOptions>(args)?)?)?,
        "ask_vault" => serde_json::to_value(service.ask(&parse_args::<AskRequest>(args)?)?)?,
        "list_diagrams" => serde_json::to_value(service.list_diagrams(&parse_args::<RefArgs>(args)?.r#ref)?)?,
        "validate_diagram" => serde_json::to_value(service.validate_diagram(&parse_args::<DiagramRequest>(args)?)?)?,
        "render_diagram" => serde_json::to_value(service.render_diagram(&parse_args::<DiagramRequest>(args)?)?)?,
        "list_sheets" => serde_json::to_value(service.list_sheets())?,
        "read_sheet" => serde_json::to_value(service.read_sheet(&parse_args::<SheetReadRequest>(args)?)?)?,
        "query_sheet" => serde_json::to_value(service.query_sheet(&parse_args::<SheetQueryRequest>(args)?)?)?,
        "set_sheet_cell" => serde_json::to_value(service.set_sheet_cell(&parse_args::<SheetSetCellRequest>(args)?)?)?,
        "append_sheet_rows" => serde_json::to_value(service.append_sheet_rows(&parse_args::<SheetAppendRequest>(args)?)?)?,
        "create_sheet" => serde_json::to_value(service.create_sheet(&parse_args::<SheetCreateRequest>(args)?)?)?,
        "list_themes" => serde_json::to_value(service.list_themes())?,
        "export_note" => serde_json::to_value(service.export_note(&parse_args::<ExportRequest>(args)?)?)?,
        other => anyhow::bail!("unknown tool: {other}"),
    };
    Ok(out)
}

fn ref_schema(what: &str) -> Value {
    json!({
        "type": "object",
        "properties": {
            "ref": {
                "type": "string",
                "description": format!("{what}: note title, alias, vault-relative path (with or without .md) or UUID")
            }
        },
        "required": ["ref"]
    })
}

/// The `tools/list` payload: name, description and JSON Schema input.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "list_notes",
            "description": "List notes in the vault (frontmatter only, no bodies), sorted by path. Optional filters by tag and folder.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tag": { "type": "string", "description": "Only notes carrying this tag (case-insensitive; matches nested tag/sub too)" },
                    "folder": { "type": "string", "description": "Vault-relative folder; includes subfolders. \".\" = vault root only" },
                    "include_trashed": { "type": "boolean", "default": false }
                }
            }
        }),
        json!({
            "name": "read_note",
            "description": "Read one note: frontmatter, Markdown body, preserved extra frontmatter keys and its outgoing [[wikilinks]].",
            "inputSchema": ref_schema("Note to read")
        }),
        json!({
            "name": "write_note",
            "description": "Replace the body and/or tags of an existing note (atomic write; unknown frontmatter keys are preserved). With create_if_missing, creates it using `ref` as the title.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ref": { "type": "string", "description": "Note title, alias, vault-relative path or UUID; the title when creating" },
                    "body": { "type": "string", "description": "New Markdown body (omit to keep)" },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Replaces the tag list" },
                    "folder": { "type": "string", "description": "Folder for a newly created note" },
                    "create_if_missing": { "type": "boolean", "default": false }
                },
                "required": ["ref"]
            }
        }),
        json!({
            "name": "create_note",
            "description": "Create a new Markdown note named after its title (`<folder>/<title>.md`, collision-safe) and index it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "body": { "type": "string", "default": "" },
                    "folder": { "type": "string", "description": "Vault-relative folder, created if missing" },
                    "tags": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["title"]
            }
        }),
        json!({
            "name": "trash_note",
            "description": "Soft-delete a note: move it into the vault's .trash/ folder (recoverable from the app).",
            "inputSchema": ref_schema("Note to trash")
        }),
        json!({
            "name": "search_notes",
            "description": "Hybrid search (BM25 keyword + vector similarity fused with RRF) over indexed note chunks. Falls back to keyword-only when the embedding model is unavailable; the result says which.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "k": { "type": "integer", "minimum": 1, "default": 10 },
                    "semantic": { "type": "boolean", "default": true, "description": "false = keyword-only, no model load" }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "get_backlinks",
            "description": "Notes that link to the given note (by title, file stem or alias), with the line and context of each link.",
            "inputSchema": ref_schema("Link target")
        }),
        json!({
            "name": "get_links",
            "description": "Outgoing [[wikilinks]] of a note and what each resolves to (null = unresolved ghost link).",
            "inputSchema": ref_schema("Source note")
        }),
        json!({
            "name": "get_graph",
            "description": "The whole link graph: nodes (notes, canvases, PDFs, ghost targets) and edges (indices into nodes).",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "reindex",
            "description": "Rescan the vault, refresh the note/link index and (re)chunk+embed changed notes. Loads the embedding model unless keyword_only.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "full": { "type": "boolean", "default": false, "description": "Re-chunk every note even if unchanged" },
                    "keyword_only": { "type": "boolean", "default": false, "description": "Skip embeddings (no model download)" }
                }
            }
        }),
        json!({
            "name": "ask_vault",
            "description": "Answer a question from the vault with the local LLM (RAG over indexed chunks). Slow: loads a ~1 GB model on first call. Returns the answer and the notes it was grounded on.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": { "type": "string" },
                    "max_tokens": { "type": "integer", "minimum": 1, "default": crate::llm::DEFAULT_MAX_TOKENS }
                },
                "required": ["question"]
            }
        }),
        json!({
            "name": "list_themes",
            "description": "List the reading themes (built-in and plugin *.toml files) with their light/dark/print colours and the plugin folders. A note picks one with `theme: <id>` in its frontmatter.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "export_note",
            "description": "Export a note as a standalone HTML page or PDF in a reading theme's print colours (headings, callouts, code, tables, Mermaid as SVG, images inlined). Without `out`, HTML is returned inline; PDF needs `out` and a Chromium-based browser.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ref": { "type": "string", "description": "Note title, alias, vault-relative path or UUID" },
                    "format": { "type": "string", "enum": ["html", "pdf"], "default": "html" },
                    "theme": { "type": "string", "description": "Theme id from list_themes (default: the note's frontmatter theme, else mnemonic)" },
                    "out": { "type": "string", "description": "File to write, absolute or vault-relative" }
                },
                "required": ["ref"]
            }
        }),
        json!({
            "name": "list_diagrams",
            "description": "List the ```mermaid diagrams in a note: index, fence line, type, source and validation diagnostics.",
            "inputSchema": ref_schema("Note to scan")
        }),
        json!({
            "name": "validate_diagram",
            "description": "Validate Mermaid diagram syntax without rendering. Returns the diagram type, whether it is supported, and diagnostics with 1-based line/column (plus note_line for diagrams in notes). Use before writing a diagram into a note.",
            "inputSchema": diagram_schema(false)
        }),
        json!({
            "name": "render_diagram",
            "description": "Render a Mermaid diagram to SVG with MNEMONIC's native renderer. Returns svg, width, height and diagnostics.",
            "inputSchema": diagram_schema(true)
        }),
        json!({
            "name": "list_sheets",
            "description": "List the CSV/TSV (editable) and XLSX/XLS/ODS (read-only) spreadsheet files in the vault.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "read_sheet",
            "description": "Read a sheet page by page: headers, rows (with 1-based row numbers) and per-column stats (count/sum/avg/min/max of numeric cells). Workbooks: pick a worksheet with `sheet`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ref": sheet_ref_description(),
                    "sheet": { "type": "string", "description": "Worksheet name or 0-based index (default: first)" },
                    "offset": { "type": "integer", "minimum": 0, "default": 0, "description": "0-based data row to start at" },
                    "limit": { "type": "integer", "minimum": 0, "default": 100, "description": "Rows to return; 0 = headers and stats only" }
                },
                "required": ["ref"]
            }
        }),
        json!({
            "name": "query_sheet",
            "description": "Filter, sort and aggregate a sheet deterministically — use this instead of doing arithmetic yourself. All filters must hold; `columns` aggregates (sum/avg/min/max) cover every matched row, not just the returned ones. Numbers like 1,234.5 / 1.234,5 / Rp 25.000 are understood.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ref": sheet_ref_description(),
                    "sheet": { "type": "string" },
                    "filters": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "column": { "type": "string", "description": "Column name (case-insensitive) or 0-based index" },
                                "op": { "type": "string", "enum": ["eq", "ne", "contains", "not_contains", "gt", "gte", "lt", "lte", "empty", "not_empty"], "default": "eq" },
                                "value": { "type": "string", "default": "" }
                            },
                            "required": ["column"]
                        }
                    },
                    "columns": { "type": "array", "items": { "type": "string" }, "description": "Columns to return (default all)" },
                    "sort_by": { "type": "string" },
                    "descending": { "type": "boolean", "default": false },
                    "limit": { "type": "integer", "minimum": 0, "default": 100 }
                },
                "required": ["ref"]
            }
        }),
        json!({
            "name": "set_sheet_cell",
            "description": "Set one cell of a CSV/TSV sheet (atomic write; delimiter, BOM and line endings are kept). Workbooks are read-only.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ref": sheet_ref_description(),
                    "row": { "type": "integer", "minimum": 1, "description": "1-based data row, as returned by read_sheet" },
                    "column": { "type": "string", "description": "Column name or 0-based index" },
                    "value": { "type": "string" }
                },
                "required": ["ref", "row", "column", "value"]
            }
        }),
        json!({
            "name": "append_sheet_rows",
            "description": "Append rows to a CSV/TSV sheet. Each row is an array (positional) or an object keyed by column name; unknown columns are rejected.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ref": sheet_ref_description(),
                    "rows": { "type": "array", "items": { "type": ["array", "object"] } }
                },
                "required": ["ref", "rows"]
            }
        }),
        json!({
            "name": "create_sheet",
            "description": "Create a new CSV/TSV sheet in the vault (fails if the file exists).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Vault-relative path ending in .csv or .tsv" },
                    "headers": { "type": "array", "items": { "type": "string" } },
                    "rows": { "type": "array", "items": { "type": "array", "items": { "type": "string" } } }
                },
                "required": ["path", "headers"]
            }
        }),
    ]
}

fn sheet_ref_description() -> Value {
    json!({ "type": "string", "description": "Sheet: vault-relative path (e.g. Data/Budget.csv) or an unambiguous file name/stem" })
}

fn diagram_schema(render: bool) -> Value {
    let mut props = json!({
        "source": { "type": "string", "description": "Diagram text (what goes inside a ```mermaid fence). Takes precedence over ref." },
        "ref": { "type": "string", "description": "Note containing the diagram: title, alias, path or UUID" },
        "index": { "type": "integer", "minimum": 0, "default": 0, "description": "Which mermaid fence of the note (0-based)" }
    });
    if render {
        props["dark"] = json!({ "type": "boolean", "default": false, "description": "Dark theme unless the diagram sets one" });
    }
    json!({ "type": "object", "properties": props })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::Note;
    use tempfile::tempdir;

    fn service() -> (tempfile::TempDir, VaultService) {
        let dir = tempdir().unwrap();
        Note::create(dir.path(), "Satu", "isi [[Dua]]").unwrap();
        Note::create(dir.path(), "Dua", "isi").unwrap();
        let svc = VaultService::open(dir.path()).unwrap();
        (dir, svc)
    }

    #[test]
    fn initialize_and_tools_list() {
        let (_d, mut svc) = service();
        let reply = handle_message(
            &mut svc,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
        )
        .unwrap();
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(reply["result"]["serverInfo"]["name"], "mnemonic");
        assert!(reply["result"]["capabilities"]["tools"].is_object());

        assert!(handle_message(&mut svc, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());

        let reply = handle_message(&mut svc, r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#).unwrap();
        let tools = reply["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), TOOL_NAMES.len());
        for t in tools {
            assert!(t["inputSchema"]["type"] == "object");
            assert!(TOOL_NAMES.contains(&t["name"].as_str().unwrap()));
        }
    }

    #[test]
    fn tools_call_round_trip_and_errors() {
        let (_d, mut svc) = service();
        let reply = handle_message(
            &mut svc,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_backlinks","arguments":{"ref":"Dua"}}}"#,
        )
        .unwrap();
        assert_eq!(reply["result"]["isError"], false);
        let text = reply["result"]["content"][0]["text"].as_str().unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        assert_eq!(payload["backlinks"][0]["source_title"], "Satu");

        let reply = handle_message(
            &mut svc,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read_note","arguments":{"ref":"Tiga"}}}"#,
        )
        .unwrap();
        assert_eq!(reply["result"]["isError"], true);
        assert!(reply["result"]["content"][0]["text"].as_str().unwrap().contains("not found"));

        let reply = handle_message(&mut svc, r#"{"jsonrpc":"2.0","id":4,"method":"nope"}"#).unwrap();
        assert_eq!(reply["error"]["code"], METHOD_NOT_FOUND);
        let reply = handle_message(&mut svc, "{not json").unwrap();
        assert_eq!(reply["error"]["code"], PARSE_ERROR);
        let reply = handle_message(
            &mut svc,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"bogus","arguments":{}}}"#,
        )
        .unwrap();
        assert_eq!(reply["error"]["code"], INVALID_PARAMS);
        let reply = handle_message(&mut svc, r#"{"jsonrpc":"2.0","id":6,"method":"ping"}"#).unwrap();
        assert!(reply["result"].is_object());
    }

    #[test]
    fn sheet_tools_round_trip() {
        let (d, mut svc) = service();
        std::fs::write(d.path().join("Kas.csv"), "Item,Jumlah\nKopi,12000\nTeh,8000\n").unwrap();
        let call = |svc: &mut VaultService, name: &str, args: Value| {
            let msg = json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":name,"arguments":args}});
            let reply = handle_message(svc, &msg.to_string()).unwrap();
            assert_eq!(reply["result"]["isError"], false, "{name}: {reply}");
            let text = reply["result"]["content"][0]["text"].as_str().unwrap().to_string();
            serde_json::from_str::<Value>(&text).unwrap()
        };
        assert_eq!(call(&mut svc, "list_sheets", json!({}))[0]["path"], "Kas.csv");
        let q = call(&mut svc, "query_sheet", json!({"ref":"Kas","filters":[{"column":"Jumlah","op":"gt","value":"10000"}]}));
        assert_eq!(q["matched"], 1);
        assert_eq!(q["columns"][1]["sum"], 12000.0);
        call(&mut svc, "append_sheet_rows", json!({"ref":"Kas","rows":[["Susu","5000"]]}));
        call(&mut svc, "set_sheet_cell", json!({"ref":"Kas","row":1,"column":"Jumlah","value":"13000"}));
        let r = call(&mut svc, "read_sheet", json!({"ref":"Kas.csv","limit":0}));
        assert_eq!(r["total_rows"], 3);
        assert_eq!(r["columns"][1]["sum"], 26000.0);
        call(&mut svc, "create_sheet", json!({"path":"Baru.csv","headers":["A"]}));
        assert_eq!(call(&mut svc, "list_sheets", json!({})).as_array().unwrap().len(), 2);
    }

    #[test]
    fn serve_writes_one_line_per_request() {
        let (_d, mut svc) = service();
        let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"list_notes\"}}\n";
        let mut out = Vec::new();
        serve(&mut svc, input.as_bytes(), &mut out).unwrap();
        let lines: Vec<&str> = std::str::from_utf8(&out).unwrap().lines().collect();
        assert_eq!(lines.len(), 2);
        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["id"], 2);
        let text = second["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(serde_json::from_str::<Value>(text).unwrap().as_array().unwrap().len(), 2);
    }
}
