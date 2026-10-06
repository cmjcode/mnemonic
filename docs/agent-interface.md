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
| `notes read <REF> [--section S \| --block ID]` | Fresh from disk: frontmatter + body (or only section `S` / block `^ID`) + `extra` (unmodelled frontmatter keys) + outgoing link targets + the whole note's `outline` + `content_hash`. |
| `notes write <REF> [--body-file F \| --stdin] [--tag T]... [--create] [--folder F] [--if-hash H] [--agent A]` | Replace the body and/or the tag list of an existing note. `--create` creates it (titled `REF`, in `--folder`) when no note has that name. `--if-hash` refuses the write if the file changed since that read. |
| `notes create <TITLE> [--folder F] [--body-file F \| --stdin] [--tag T]... [--agent A]` | New `<folder>/<title>.md` (collision-safe name). |
| `notes append <REF> (--text T \| --body-file F \| --stdin) [--section S] [--if-hash H] [--agent A] [--create] [--folder F] [--tag T]...` | Append Markdown at the end of the note or of section `S` (see "Agent memory"). |
| `notes patch <REF> [--section S \| --block ID] [--old OLD] (--new NEW \| --body-file F \| --stdin) [--replace-all] [--if-hash H] [--agent A]` | Replace a section's content (heading kept), a block's text (anchor kept) and/or an exact `--old` string. |
| `notes trash <REF>` | Soft delete: move into `.trash/`, mark `trashed: true`, drop its search chunks. |
| `search <QUERY> [-k N] [--keyword-only] [--folder F] [--tag T]` | Hybrid BM25 + vector search over indexed chunks (RRF-fused, one hit per note), optionally within a folder subtree / tag. Hits carry the `heading` and `line` they come from. |
| `backlinks <REF>` | Notes whose links resolve to `REF` (by title, file stem, alias or `Folder/Name`, resolved from the linking note). |
| `links <REF>` | Outgoing `[[wikilinks]]`, what each resolves to and, for shared names, the `other_candidates`. |
| `folders` | Every non-hidden folder with direct and subtree note counts. |
| `overview` | One-call orientation: folders, top tags, recent notes, duplicate titles, the vault's `AGENTS.md`. |
| `remember [TEXT \| --body-file F \| --stdin] [--title T] [--folder F] [--tag T]... [--into REF [--section S]] [--link N]... [--agent A] [--allow-duplicate] [--keyword-only]` | Store a memory (new note in `Memory/`, or appended to `--into`); nothing is written when the vault already holds it. |
| `recall <QUERY> [--budget N] [-k N] [--folder F] [--tag T] [--keyword-only] [--include-stale]` | What the vault knows about `QUERY`: sections packed into a token budget, stale notes last. |
| `related <REF> [-k N]` | Notes connected to `REF`: `links_to`, `linked_from`, `similar`, `shared_tag:<tag>`. |
| `graph` | Nodes (`note`/`canvas`/`pdf`/`ghost`) and edges (indices into `nodes`). |
| `index [--full] [--keyword-only]` | Rescan, refresh `notes_index`/`links`, chunk + embed changed notes, prune stale chunks. |
| `ask <QUESTION> [--max-tokens N]` | RAG answer from the local LLM with citations. |
| `diagram list <REF>` | Every ```` ```mermaid ```` fence of a note: index, fence line, type, source, diagnostics. |
| `diagram validate (--file F \| --stdin \| --note REF [--index N])` | Syntax check; exits non-zero on errors. `--file`/`--stdin` need no vault. |
| `diagram render (--file F \| --stdin \| --note REF [--index N]) [--out F] [--dark]` | Render to SVG (stdout, or `--out`; `--json` wraps it with size and diagnostics). |
| `canvas sections <REF>` | The note's section segments — the boxes its canvas shows: `id` (anchor, `null` until the note is opened as a canvas), `kind` (`section`/`table`/`mermaid`/`code`/`text`), `level`, `parent` (section id), 1-based `line`/`end_line`, `summary`. |
| `canvas mermaid <REF> [--mindmap] [--out F]` | The note's canvas as Mermaid, one diagram per family (see "Section canvas" below). Read-only. Prints the fences (or writes them to `--out`); `--json` gives `{path, has_sidecar, diagrams: [{kind, source}], warnings, markdown}`. |
| `sheets list` | Every CSV/TSV (editable) and XLSX/XLSM/XLSB/XLS/ODS (read-only) file in the vault. |
| `sheets read <SHEET> [--sheet W] [--offset N] [--limit N]` | Headers, a page of rows (1-based `row` numbers) and per-column stats. `--sheet` picks a worksheet (name or 0-based index). |
| `sheets query <SHEET> [--where COL:OP:VALUE]... [--column C]... [--sort C] [--desc] [--limit N]` | Filter (all conditions AND) + sort + project; `columns` stats cover every matched row. OP: `eq ne contains not_contains gt gte lt lte empty not_empty`. |
| `sheets set <SHEET> <ROW> <COLUMN> <VALUE>` | Set one cell of a CSV/TSV (row 1-based, column by name or 0-based index). |
| `sheets append <SHEET> --row JSON...` | Append rows: a JSON array (positional) or object keyed by column name. |
| `sheets create <PATH> --header H...` | New `.csv`/`.tsv` (fails if it exists). |
| `themes list` | Reading themes (built-in + plugin `*.toml`) with their source, plus the plugin folders and any load problems. `--json` includes every light/dark/print colour. |
| `notes export <REF> [--format html\|pdf] [--theme ID] [--out F]` | The note as a standalone HTML page (stdout without `--out`) or PDF (`--out` required; needs Chrome/Edge/Chromium/Brave, or `MNEMONIC_BROWSER`), in the theme's print colours. `--out` may be vault-relative. |
| `mcp` | Serve MCP over stdio until stdin closes. |

`<REF>` resolves, in order: UUID (`id` frontmatter); a vault-relative path
(`Kuliah/Algoritma Graph.md`, or without `.md` when it contains a `/`);
then wikilink rules — title, file stem, alias (case-insensitive), or a
folder-qualified name matched as a path suffix (`Kuliah/Algoritma Graph`);
then bare file name. **A name several notes share is an error** listing
their paths (`ambiguous note reference …`): agents must pass a path then,
never get one of them silently. `--create`/`create_if_missing` only create
when no note has the name at all. `<SHEET>` is a vault-relative path
(`Data/Budget.csv`) or a file name/stem that matches exactly one sheet.

Folders work like an Obsidian vault: any tree of subfolders, hidden folders
(`.obsidian`, `.git`, `.trash`, `.mnemonic`) skipped. Links resolve like
Obsidian too: `[[Name]]` shared by several notes means the one in the
linking note's folder, else the one closest to the root; `[[Folder/Name]]`
pins one. Renaming a note in the app rewrites `[[Folder/Old]]` links as
well as plain ones.

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
// notes read → NoteSummary fields + "body", "extra": {…}, "links": ["Target", …],
//              "content_hash": "16 hex", "outline": [ { "level", "text",
//              "path": "Parent#Child", "anchor": null|"id", "line", "end_line" } ],
//              "selection": null | { "kind": "section"|"block", "label", "line", "end_line" }
// notes write → NoteSummary fields + "created", "warnings", "content_hash"
// notes append/patch → NoteSummary fields + "content_hash", "changed", "created",
//              "selection", "replacements", "warnings"
// search → { "query", "semantic": bool, "hits": [ { "doc_id", "path", "title",
//            "page": null|N (PDF), "row": null|N (sheet, first data row),
//            "char_offset", "score": null|f32,
//            "kind": "semantic"|"keyword"|"both", "snippet", "text",
//            "heading": null|"Parent#Child", "line": null|N (1-based) } ], "warnings": [] }
// backlinks → { "target": {id,title,path}, "backlinks": [ { "source_id",
//               "source_title", "source_path", "line" (0-based), "context" } ] }
// links → { "source": {…}, "links": [ { "target", "heading", "alias", "line",
//           "context", "resolved_path": null|"…", "other_candidates": ["…"] } ] }
// folders → { "folders": [ { "path" ("" = root), "name", "depth", "notes", "notes_total" } ] }
// overview → { "name", "notes", "canvases", "sheets", "pdfs", "folders": [Folder],
//              "tags": [ { "tag", "count" } ], "recent": [ { "id", "title", "path", "modified" } ],
//              "duplicate_titles": [ { "title", "paths" } ],
//              "guide": null | { "path": "AGENTS.md", "text", "truncated" }, "tips": [] }
// remember → { "status": "created"|"appended"|"duplicate", "note": null|NoteSummary,
//              "content_hash": null|"…", "similar": [ { "path", "title", "score",
//              "kind": "exact"|"semantic"|"keyword"|"both", "heading", "snippet" } ], "warnings" }
// recall → { "query", "semantic", "budget_tokens", "used_tokens", "omitted",
//            "items": [ { "path", "title", "section": null|"A#B", "line", "text",
//            "score", "kind", "stale": null|"reason", "truncated", "tokens" } ], "warnings" }
// related → { "note": {id,title,path}, "related": [ { "path", "title",
//             "reasons": ["links_to"|"linked_from"|"similar"|"shared_tag:x"], "score" } ], "warnings" }
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

## Agent memory (§3.10)

The vault doubles as an agent's long-term memory. What makes it work well
for agents, beyond a plain folder of Markdown:

1. **Orient in one call.** `overview` / `vault_overview` returns the folder
   tree, the most used tags, recent notes, titles several notes share, and
   the vault's own **`AGENTS.md`** (at the vault root): write your vault's
   conventions there ("projects live in `Proyek/`, one note per client,
   decisions go under `## Keputusan`") and every agent reads them first.
2. **Read only what you need.** `read_note` always returns the note's
   `outline` (every heading with `path` `Parent#Child` and 1-based line
   range). Ask for one part with `section` — `Status`, `Proyek#Status`
   (ancestors in order, not necessarily adjacent) or `^anchor` — or with
   `block` (an anchored `^id` block). A section runs to the next heading of
   the same or a higher level, subsections included. Search hits and
   recall items carry the `heading`/`section` they came from.
3. **Never overwrite someone else's edit.** Every read returns
   `content_hash` (FNV-1a of the file bytes). Pass it back as `if_hash` to
   `write_note`/`append_note`/`patch_note`: when the file changed since
   (the user typed in the app, another agent wrote), the edit is refused
   with the new hash — read again and redo it. Every edit also reloads the
   note from disk first, so a long-running MCP server never writes back a
   stale copy.
4. **Edit surgically.** `append_note` adds text at the end of a note or a
   section (blank-line/list spacing handled; `create_if_missing` for logs).
   `patch_note` replaces a section's content (heading line kept), a
   block's text (anchor and list marker kept) and/or an exact `old_str`
   (must be unique unless `replace_all`; searched only inside the
   section/block when one is given). Line endings (LF/CRLF) are kept.
5. **Provenance.** `agent` (on every write) is recorded in the frontmatter
   as `updated_by` (and `created_by` for new notes) — plain keys, so the
   note stays a normal Obsidian note.
6. **Memory semantics.** `remember` writes a self-contained note into
   `Memory/` (or `folder`, or appends to `ref`/`section`), links it to
   `links`, and indexes it at once. It refuses (`status: "duplicate"`,
   nothing written) when the text is already contained in a note, or a
   chunk is ≥ 0.92 cosine-similar (semantic mode). `recall` expands the best
   chunks to their whole sections (a heading-less note up to 2000 chars is
   returned whole), deduplicates, and packs them into `budget_tokens`
   (≈ 4 chars/token; `omitted` counts what didn't fit). `related_notes`
   explains each neighbour.
7. **Forgetting without deleting.** A note is *stale* when its frontmatter
   has `valid_until: YYYY-MM-DD` in the past, `superseded_by: "[[Newer]]"`,
   `archived: true`, or another note lists it in `supersedes:` (a link or a
   list of links). `recall` flags it in `stale` and ranks it last (unless
   `include_stale`). Nothing is removed; the user stays in control.

After any agent write the note's chunks are refreshed immediately — with
embeddings when the model is already loaded in that process, otherwise
keyword-only with the `kw:` marker so the next semantic `index` (or the
app's indexer) embeds it.

Suggested agent loop: `vault_overview` → `recall` (or `search_notes`) →
`read_note` with `section` → `patch_note`/`append_note` with `if_hash` →
`remember` for new durable facts.

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
| `read_note` | `ref`, `section?`, `block?` |
| `write_note` | `ref`, `body?`, `tags?`, `folder?`, `create_if_missing?`, `if_hash?`, `agent?` |
| `create_note` | `title`, `body?`, `folder?`, `tags?`, `agent?` |
| `trash_note` | `ref` |
| `search_notes` | `query`, `k?` (10), `semantic?` (true), `folder?`, `tag?` |
| `vault_overview` | – |
| `list_folders` | – |
| `append_note` | `ref`, `text`, `section?`, `if_hash?`, `agent?`, `create_if_missing?`, `folder?`, `tags?` |
| `patch_note` | `ref`, `new_str`, `section?` or `block?`, `old_str?`, `replace_all?`, `if_hash?`, `agent?` |
| `remember` | `text`, `title?`, `folder?` (`Memory`), `tags?`, `ref?`, `section?`, `links?`, `agent?`, `allow_duplicate?`, `semantic?` (true) |
| `recall` | `query`, `budget_tokens?` (1500), `k?` (8), `folder?`, `tag?`, `semantic?` (true), `include_stale?` |
| `related_notes` | `ref`, `k?` (10) |
| `get_backlinks` | `ref` |
| `get_links` | `ref` |
| `get_graph` | – |
| `reindex` | `full?`, `keyword_only?` |
| `ask_vault` | `question`, `max_tokens?` (512) |
| `list_diagrams` | `ref` |
| `validate_diagram` | `source?` or `ref` + `index?` (0) |
| `render_diagram` | `source?` or `ref` + `index?` (0), `dark?` (false) |
| `list_sections` | `ref` |
| `export_canvas_mermaid` | `ref`, `mindmap?` (false) |
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
- To turn any note into a diagram, open it in the app in "Text + Diagram" mode
  (see "Section canvas"); or create the sidecar yourself with `file` nodes
  pointing at anchors you added.

### Section canvas (spec §3.9)

Opening a plain note as a canvas gives one box per **section segment**: a
heading with its own prose (up to the next heading, table or fence), each
table, each ```` ```mermaid ```` / code fence, and prose that follows a
component. Segments never overlap. Anchors: ` ^id` at the end of the heading
line, at the end of the last line of trailing prose, and on a line of its own
right below a table or fence. Boxes are laid out as a mind map following the
heading nesting (edges with `mnemonic.meta.outline: true`).

- Section nodes are `file` nodes with `subpath: "#Heading text"` (Obsidian
  embeds the whole section) plus `"mnemonic": {"scope": "segment",
  "block_id": "<id>"}`; the id is authoritative (a renamed heading still
  resolves). Tables and fences use `subpath: "#^id"`.
- Add a section as an agent by writing Markdown (a new heading with text); the
  app anchors it and adds its box on the next save. Removing a section's
  Markdown leaves its box marked as an orphan; nothing is deleted silently.
- Boxes removed from the canvas are listed in `mnemonic.hidden_segments`.
- Entity (`mnemonic.kind: "entity"`, `attributes: [{ty, name, keys, comment}]`)
  and class (`"class"`, `annotation`, `members`, `methods`) nodes are `text`
  nodes whose text is a readable form of the same data. Edges may carry
  `mnemonic.meta.relation`: `{"type":"er","from":"exactly_one","to":"zero_or_more","identifying":true}`
  or `{"type":"class","kind":"inheritance","card_from":"1","card_to":"*"}`,
  and `mnemonic.meta.dashed`.
- `export_canvas_mermaid` returns one diagram per family: `flowchart`
  (boxes, shapes, frames as subgraphs, `click … href "[[Note#^id]]"`),
  `erDiagram`, `classDiagram`, `stateDiagram-v2` (shapes connected to a `[*]`
  dot), `embedded` (a fence box, copied verbatim) and, with `mindmap`, a
  `mindmap` of the outline. Everything it returns passes `validate_diagram`.


## Mermaid diagrams

Structured diagrams belong in the note itself, as ```` ```mermaid ```` fences
(the same syntax Obsidian and GitHub render). MNEMONIC parses and draws them
natively (spec §3.7). Rendered types: flowchart/graph, sequence, class, state,
ER, pie, mindmap; other Mermaid types are recognised and shown as source until
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
