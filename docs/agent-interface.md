# Agent interface: `mnemonic-cli` and the MCP server

MNEMONIC is a desktop note app, but the vault it manages is just a folder of
Markdown files plus a rebuildable SQLite index. `mnemonic-cli` exposes that
vault to external agents (Claude Code, Codex, Cursor, shell scripts) in two
ways, both built on the same egui-free `mnemonic::api::VaultService`:

- a **CLI** with stable `--json` output, and
- an **MCP server** over stdio (`mnemonic-cli mcp`).

The desktop app and the CLI can be used on the same vault at the same time.

```
cargo build --release --bin mnemonic-cli
./target/release/mnemonic-cli --vault ~/Notes notes list --json
```

## Choosing the vault

`--vault <PATH>` (or `MNEMONIC_VAULT=<PATH>`) selects the vault folder; it
is created if missing. Without it the CLI falls back to the vault last
opened in the desktop app (`settings.vault_path`) and errors out if there is
none. Every path in output is **vault-relative** with `/` separators.

## CLI reference

Global flags: `--vault <PATH>`, `--json`. Errors print `error: ...` on
stderr and exit non-zero; stdout carries only results. Logs go to stderr
(`RUST_LOG=info` shows model loading).

| Command | What it does |
| --- | --- |
| `notes list [--tag T] [--folder F] [--include-trashed]` | Frontmatter summaries sorted by path. `--tag a` also matches nested `a/b`. `--folder F` includes subfolders; `--folder .` is the root only. |
| `notes read <REF>` | Frontmatter + body + `extra` (unmodelled frontmatter keys) + outgoing link targets. |
| `notes write <REF> [--body-file F \| --stdin] [--tag T]... [--create] [--folder F]` | Replace the body and/or the tag list of an existing note. `--create` creates it (titled `REF`, in `--folder`) when missing. |
| `notes create <TITLE> [--folder F] [--body-file F \| --stdin] [--tag T]...` | New `<folder>/<title>.md` (collision-safe name). |
| `notes trash <REF>` | Soft delete: move into `.trash/`, mark `trashed: true`, drop its search chunks. |
| `search <QUERY> [-k N] [--keyword-only]` | Hybrid BM25 + vector search over indexed chunks (RRF-fused, one hit per note). |
| `backlinks <REF>` | Notes linking to `REF` by title, file stem or alias. |
| `links <REF>` | Outgoing `[[wikilinks]]` and what each resolves to. |
| `graph` | Nodes (`note`/`canvas`/`pdf`/`ghost`) and edges (indices into `nodes`). |
| `index [--full] [--keyword-only]` | Rescan, refresh `notes_index`/`links`, chunk + embed changed notes, prune stale chunks. |
| `ask <QUESTION> [--max-tokens N]` | RAG answer from the local LLM with citations. |
| `diagram list <REF>` | Every ```` ```mermaid ```` fence of a note: index, fence line, type, source, diagnostics. |
| `diagram validate (--file F \| --stdin \| --note REF [--index N])` | Syntax check; exits non-zero on errors. `--file`/`--stdin` need no vault. |
| `diagram render (--file F \| --stdin \| --note REF [--index N]) [--out F] [--dark]` | Render to SVG (stdout, or `--out`; `--json` wraps it with size and diagnostics). |
| `sheets list` | Every CSV/TSV (editable) and XLSX/XLSM/XLSB/XLS/ODS (read-only) file in the vault. |
| `sheets read <SHEET> [--sheet W] [--offset N] [--limit N]` | Headers, a page of rows (1-based `row` numbers) and per-column stats. `--sheet` picks a worksheet (name or 0-based index). |
| `sheets query <SHEET> [--where COL:OP:VALUE]... [--column C]... [--sort C] [--desc] [--limit N]` | Filter (all conditions AND) + sort + project; `columns` stats cover every matched row. OP: `eq ne contains not_contains gt gte lt lte empty not_empty`. |
| `sheets set <SHEET> <ROW> <COLUMN> <VALUE>` | Set one cell of a CSV/TSV (row 1-based, column by name or 0-based index). |
| `sheets append <SHEET> --row JSON...` | Append rows: a JSON array (positional) or object keyed by column name. |
| `sheets create <PATH> --header H...` | New `.csv`/`.tsv` (fails if it exists). |
| `themes list` | Reading themes (built-in + plugin `*.toml`) with their source, plus the plugin folders and any load problems. `--json` includes every light/dark/print colour. |
| `notes export <REF> [--format html\|pdf] [--theme ID] [--out F]` | The note as a standalone HTML page (stdout without `--out`) or PDF (`--out` required; needs Chrome/Edge/Chromium/Brave, or `MNEMONIC_BROWSER`), in the theme's print colours. `--out` may be vault-relative. |
| `mcp` | Serve MCP over stdio until stdin closes. |

`<REF>` resolves, in order: UUID (`id` frontmatter), then wikilink rules
(title, file stem, alias; case-insensitive), then vault-relative path with
or without `.md`, then bare file name. `<SHEET>` is a vault-relative path
(`Data/Budget.csv`) or a file name/stem that matches exactly one sheet.

### Search, index and models

- `search` and `ask` only see notes that have been indexed: run `index`
  first (the desktop app also indexes in the background while it runs).
- Semantic search needs the FastEmbed `multilingual-e5-small` model
  (~120 MB, downloaded once into FastEmbed's cache). If it cannot be loaded
  the call **degrades to keyword-only** and says so: `"semantic": false`
  plus a `warnings` entry. `--keyword-only` skips the model on purpose.
- `index --keyword-only` chunks notes into the FTS index with placeholder
  zero vectors and records a `kw:`-prefixed content hash, so the next
  semantic `index` (CLI or app) still embeds them. A keyword-only run never
  overwrites current real embeddings, even with `--full`.
- `ask` loads the quantized Qwen2.5 model through Candle (~1 GB, first
  call only) and runs on CPU; expect seconds to minutes.

### JSON shapes

`--json` prints exactly the `serde` form of the `mnemonic::api::types`
structs. Field names are snake_case and stable. Highlights:

```jsonc
// notes list → [NoteSummary]
{ "id": "uuid", "title": "…", "path": "Rumah/Belanja.md", "folder": "Rumah",
  "note_type": "note", "tags": [], "aliases": [], "created": "RFC3339",
  "modified": "RFC3339", "pinned": false, "archived": false, "trashed": false,
  "canvas": false }
// notes read → NoteSummary fields + "body", "extra": {…}, "links": ["Target", …]
// search → { "query", "semantic": bool, "hits": [ { "doc_id", "path", "title",
//            "page": null|N (PDF), "row": null|N (sheet, first data row),
//            "char_offset", "score": null|f32,
//            "kind": "semantic"|"keyword"|"both", "snippet", "text" } ], "warnings": [] }
// backlinks → { "target": {id,title,path}, "backlinks": [ { "source_id",
//               "source_title", "source_path", "line" (0-based), "context" } ] }
// links → { "source": {…}, "links": [ { "target", "heading", "alias", "line",
//           "context", "resolved_path": null|"…" } ] }
// graph → { "nodes": [ { "key", "label", "kind", "doc_id", "path", "tag", "degree" } ],
//           "edges": [ { "a", "b", "kind": "link"|"semantic", "weight" } ] }
// index → { "notes_indexed", "chunked", "skipped", "pruned", "semantic",
//           "sheets_indexed", "sheets_chunked",
//           "failed": [ { "path", "error" } ], "warnings": [] }
// ask → { "question", "answer", "citations": [ { "doc_id", "path", "title",
//         "page": null|N, "row": null|N } ], "semantic", "warnings" }
// sheets list → [ { "path", "kind": "csv"|"workbook", "editable", "size_bytes",
//                   "modified": null|"RFC3339" } ]
// sheets read → { "path", "kind", "editable", "sheet", "sheets": [names],
//                 "headers", "total_rows", "offset", "rows": [ { "row", "cells" } ],
//                 "columns": [SheetColumn] }
// sheets query → { "path", "sheet", "headers", "matched", "rows", "columns": [SheetColumn] }
// SheetColumn = { "name", "numeric", "filled", "count", "sum", "avg", "min", "max" }
//               (sum/avg/min/max are null when the column has no numbers)
// sheets set/append/create → { "path", "rows", "columns", "changed" }
```

## MCP server

`mnemonic-cli --vault <PATH> mcp` speaks JSON-RPC 2.0 over stdio, one JSON
object per line (newline-delimited, no `Content-Length` headers). It
implements `initialize` (protocol `2025-06-18`, `capabilities.tools`),
`notifications/initialized`, `ping`, `tools/list` and `tools/call`.
Unknown methods return JSON-RPC error `-32601`; unknown tool names
`-32602`. A tool that fails returns a normal result with
`"isError": true` and the error message in the text block. Nothing but
protocol messages is ever written to stdout.

Tools (each returns one `{"type":"text","text":"<JSON>"}` content block
holding the same JSON the CLI prints):

| Tool | Arguments |
| --- | --- |
| `list_notes` | `tag?`, `folder?`, `include_trashed?` |
| `read_note` | `ref` |
| `write_note` | `ref`, `body?`, `tags?`, `folder?`, `create_if_missing?` |
| `create_note` | `title`, `body?`, `folder?`, `tags?` |
| `trash_note` | `ref` |
| `search_notes` | `query`, `k?` (10), `semantic?` (true) |
| `get_backlinks` | `ref` |
| `get_links` | `ref` |
| `get_graph` | – |
| `reindex` | `full?`, `keyword_only?` |
| `ask_vault` | `question`, `max_tokens?` (512) |
| `list_diagrams` | `ref` |
| `validate_diagram` | `source?` or `ref` + `index?` (0) |
| `render_diagram` | `source?` or `ref` + `index?` (0), `dark?` (false) |
| `list_sheets` | – |
| `read_sheet` | `ref`, `sheet?`, `offset?` (0), `limit?` (100) |
| `query_sheet` | `ref`, `sheet?`, `filters?` (`[{column, op?, value?}]`), `columns?`, `sort_by?`, `descending?`, `limit?` (100) |
| `set_sheet_cell` | `ref`, `row` (1-based), `column`, `value` |
| `append_sheet_rows` | `ref`, `rows` (arrays or objects) |
| `create_sheet` | `path`, `headers`, `rows?` |
| `list_themes` | – |
| `export_note` | `ref`, `format?` (`html`), `theme?`, `out?` (required for `pdf`; without it HTML is returned in `html`) |

### Claude Code

Project-scoped `.mcp.json` (checked into the repo that should see the vault):

```json
{
  "mcpServers": {
    "mnemonic": {
      "command": "/path/to/mnemonic-cli",
      "args": ["--vault", "/Users/me/Notes", "mcp"]
    }
  }
}
```

Or from the shell:

```
claude mcp add mnemonic -- /path/to/mnemonic-cli --vault /Users/me/Notes mcp
```

Other MCP clients (Cursor, Codex, Zed, …) take the same `command` + `args`
pair in their own config file. Set `MNEMONIC_VAULT` in `env` instead of
`--vault` if you prefer.

### Hand-testing the protocol

```
printf '%s\n' \
 '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"sh","version":"0"}}}' \
 '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
 '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_notes","arguments":{}}}' \
 | mnemonic-cli --vault ~/Notes mcp
```

## Note file format

A note is a `.md` file, optionally starting with a YAML frontmatter block:

```md
---
id: 8f3a1c2e-91b4-4d2f-9a7e-1234567890ab
title: Belanja Mingguan
type: note            # note | checklist | canvas
created: 2026-08-20T09:15:00+00:00
modified: 2026-08-23T10:02:00+00:00
pinned: false
color: yellow         # optional
tags: [rumah, belanja]
aliases: [groceries]  # optional
archived: false
trashed: false
reminder: 2026-09-01T08:00:00+00:00   # optional
cssclasses: [wide]    # any other key is preserved verbatim
---
Body markdown. Link to [[Another Note]], [[Another Note#Heading]],
[[Another Note|shown text]], or by alias [[groceries]].
```

Rules agents should rely on:

- **File name = title.** `Note::create` names the file after the title
  (Obsidian's forbidden characters `* " \ / < > : | ? # ^ [ ]` become
  spaces; collisions get ` (2)`, ` (3)`, …). A file without `title:` is
  titled after its file stem; a file without `id:` gets a deterministic
  UUIDv5 of its path, so ids are stable across rescans.
- **Unknown frontmatter keys survive** every write (`extra` in the API).
  Keys MNEMONIC models are rewritten in a fixed order; a frontmatter block
  that is not valid YAML is written back untouched (and metadata edits
  are then not persisted; `write_note` reports this in `warnings`).
- **Writes are atomic** (temp file + rename in the same folder) and bump
  `modified`.
- **`[[wikilinks]]` resolve by title, file stem or alias**,
  case-insensitively, vault-wide (folders don't matter). `[[x#Heading]]`
  and `[[x|alias]]` are understood; `![[embeds]]` and links inside fenced
  code blocks are not links. Unresolved targets show up as `ghost` graph
  nodes and `resolved_path: null`.
- `^block-id` anchors (`[[Note#^abc123]]`) mark blocks that can be linked and bound to diagram nodes (see below)
  (`src/block`); today they are parsed as part of the heading text.
- Hidden folders (`.mnemonic`, `.trash`, `.obsidian`, `.git`, …) and
  `node_modules` are never scanned for notes. Trashed notes live flat in
  `.trash/` and are listed only with `include_trashed`.
- Canvas notes (`type: canvas`, or a ```` ```canvas ```` / ```` ```drawio ```` body)
  carry a serialized diagram, not prose: they are indexed by the text on
  their shapes and never contribute wikilinks.

## The index: `.mnemonic/index.sqlite3`

Inside every vault, `.mnemonic/index.sqlite3` caches what is derived from
the files: note metadata + the wikilink table (`notes_index`, `links`),
text chunks with an FTS5 index (`document_chunks`, `chunks_fts`),
`sqlite-vec` vector tables (`vec_chunks`, `vec_docs`), per-note content
hashes for incremental indexing, the imported-PDF list and saved graph
layout.

- It is **derived and rebuildable**: deleting it loses nothing but time
  (`index --full` recreates it). The Markdown files are the only source of
  truth.
- It is opened in **WAL mode with a 5 s busy timeout**, so the desktop app
  and one or more `mnemonic-cli` processes can read and write it
  concurrently. Opening the CLI refreshes `notes_index`/`links` from disk;
  chunks and vectors are only touched by `index`.
- Don't hand-edit it, and don't sync it: it's machine-local cache. Add
  `.mnemonic/` to the vault's `.gitignore` if the vault is a git repo.


## Diagram-bound notes (`.canvas` sidecar) and block anchors

A note may carry a diagram layer in `<Title>.canvas` next to `<Title>.md`, in the
[Obsidian JSON Canvas 1.0](https://jsoncanvas.org) format. Rules an agent can rely on:

- The Markdown is the source of truth for text. A block that ends with ` ^id`
  (6 lowercase alphanumerics, Obsidian block reference) can be linked as
  `[[Title#^id]]` and bound to a diagram node.
- A **bound node** is a JSON Canvas `file` node: `{"type":"file","file":"Title.md","subpath":"#^id",…}`.
  It stores no text; the app derives it from the block. Editing the node in the
  app rewrites the block; editing the block updates the node.
- **Diagram-only** content (arrows, decorative shapes, groups, free connectors,
  strokes) lives only in the sidecar (`text`/`group` nodes, `edges`, and a
  top-level `"mnemonic"` extension object). It never appears in the Markdown.
- Draw.io import binds only `text;`-style vertices (they become anchored
  paragraphs appended to the note); other vertices stay diagram-only. Export writes
  `mnemonicBlock=<id>` into the style of bound vertices.
- Renaming, trashing, restoring and deleting a note moves its sidecar with it.
- To turn any note into a diagram, open it in the app in "Text + Diagram" mode:
  every block gets an anchor and one node; or create the sidecar yourself with
  `file` nodes pointing at anchors you added.


## Mermaid diagrams

Structured diagrams belong in the note itself, as ```` ```mermaid ```` fences
(the same syntax Obsidian and GitHub render). MNEMONIC parses and draws them
natively (spec §3.7). Rendered types: flowchart/graph, sequence, class, state,
ER, pie; other Mermaid types are recognised and shown as source until
supported (`supported: false` in the JSON).

Recommended agent loop: write the diagram text, run `validate_diagram`
(`{"source": "..."}`), fix every `error` diagnostic, then write the note.
Diagnostics look like:

```json
{"line": 2, "note_line": 9, "col": 5, "severity": "error",
 "message": "link needs an arrow head (`-->`) or a third dash (`---`)"}
```

`line`/`col` are 1-based within the fence; `note_line` (diagrams read from a
note) is the 1-based line in the note body. `DiagramCheck` =
`{kind, supported, valid, diagnostics}`; `DiagramRender` adds `format`
(`"svg"`), `width`, `height`, `svg`.


## Sheets (CSV / XLSX)

Spreadsheets live in the vault next to notes (spec §3.8). CSV/TSV files are
the editable format and stay the source of truth; workbooks
(`.xlsx .xlsm .xlsb .xls .ods`) are **read-only** — rewriting them would drop
formulas, formatting and charts. The app's *Convert to CSV* makes an editable
copy; *Export as XLSX* writes a new workbook from a CSV.

- The first row is the header row. Row numbers everywhere (`row` in
  `read`/`query` output, `set_sheet_cell`, search hits, citations) are
  **1-based data rows**, header not counted.
- Writes keep the file's delimiter (`,` `;` tab `|`, sniffed on read), UTF-8
  BOM and line endings; quoting is normalized to "only when needed". Files
  that aren't valid UTF-8 are refused for writing.
- Numbers are parsed for sorting, comparisons and stats, never rewritten:
  `1,234.5`, `1.234,5`, `Rp 25.000`, `$1,200`, `12%` all count.
- For totals, averages and "rows where …" questions use `query_sheet`: the
  arithmetic is exact and covers every matched row. `search_notes`/`ask_vault`
  find *which* rows are relevant (sheets are chunked as `Column: value; …`
  lines, the first 5,000 rows of files up to 64 MB) but the local LLM is not
  reliable at adding numbers.
- In notes, link a sheet with `[[Budget.csv]]` or embed a preview table with
  `![[Budget.csv]]`.

## Reading themes and export

A rendered note, its printout and its HTML/PDF export share one colour
set: the note's reading theme (spec §3.2.5). A note picks one with
`theme: <id>` in its frontmatter (ignored when no such theme exists);
otherwise the app uses `reading_theme` from `config.toml`, and the CLI/MCP
use `mnemonic` unless `--theme`/`theme` is given. Exports use the theme's
`[print]` colours.

Themes are declarative TOML files — no code runs — in
`<config_dir>/mnemonic/themes/` (per user) or `<vault>/.mnemonic/themes/`
(travels with the vault; wins on equal ids). The file stem is the id.
Built-ins: `mnemonic`, `pelangi`, `ocean`, `sunset`, `forest`,
`print-classic` (sources in `themes/`). Format and every colour key:
[`docs/themes.md`](themes.md).

## User configuration (`config.toml`)

`<config_dir>/mnemonic/config.toml` (macOS: `~/Library/Application Support/mnemonic/config.toml`).
Besides `vault_path`, `theme`, `locale`, `reading_theme` (reading theme id,
default `mnemonic`), an optional `[hotkeys]` table remaps app shortcuts;
any action left out keeps its default:

```toml
[hotkeys]
palette = "Cmd+P"        # default Cmd+K
new_note = "Cmd+N"
search = "Cmd+Shift+F"
save = "Cmd+S"
toggle_source = "Cmd+E"  # full Markdown source ⇄ Live view
print = "Cmd+P"
sidebar = "Cmd+\\"
ai = "Cmd+J"
shortcuts = "Cmd+/"
graph = "Cmd+G"
daily = "Cmd+D"
```

Modifiers: `Cmd` (⌘ on macOS, Ctrl elsewhere), `Ctrl`, `Shift`, `Alt`; keys are a letter,
digit, punctuation character or an egui key name such as `Enter`, `F5`, `Space`.

Math: `$x^2$` and `$$\sum_{i=1}^n x_i$$` render as Unicode (Greek letters, operators,
super/subscripts, fractions, roots); the Markdown keeps the raw LaTeX.
