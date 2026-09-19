# MNEMONIC — guide for coding agents

Rust 2024 desktop note app (eframe/egui) over an Obsidian-style vault of
`.md` files, with a rebuildable SQLite index (FTS5 + sqlite-vec), local
embeddings (fastembed) and a local LLM (candle, Qwen2.5). Crate name
`mnemonic`; binaries `mnemonic` (GUI, `src/main.rs`) and `mnemonic-cli`
(`src/bin/mnemonic-cli.rs`).

## Spec and citations

`docs/pengembangan.md` is the product/technical spec. Doc comments cite it
as `§3.1.4`, `§Fase 2`, etc. — when you touch a module, keep those
citations accurate, and look the section up before changing behaviour.
`docs/agent-interface.md` documents the CLI/MCP contract.

## Architecture map (`src/`)

| Module | Role | Notes |
| --- | --- | --- |
| `notes/` | Vault scan, `Note` CRUD, YAML frontmatter, trash, watcher, tags, query | Source of truth is the `.md` file. `Note::save` is atomic; unknown frontmatter keys are preserved in `extra`. File name = title. |
| `markdown/` | Editor session (Live/Source/Split/Canvas modes, sidecar sync; `editor/canvas_sync` = notes ⇄ section canvas, §3.9.2), `sections` (body → non-overlapping anchored segments: heading + own prose, table, fence, trailing text), `live_blocks` (body → Live blocks, `replace_lines`, list continuation), `renderer/` (Live view per block: wikilinks, `#tags`, embeds/transclusion, callouts, hidden `^anchors`, theme colours; `transform` is shared with export), `highlight` (styled source), `blocks` (`^block-id` anchors), `outline` (heading tree; select/replace/append a section or block — agent edits, §3.10.2), `wikilink` | No separate write/read mode: the note is rendered and only the clicked line/block becomes raw Markdown (`app::editor::live`). `WikilinkIndex` resolves by title, file stem, alias (case-insensitive) or `Folder/Name`; same-named notes are all kept as candidates and `resolve_from` prefers the linking note's folder (Obsidian's rule). Lines are 0-based. |
| `core/` | `storage` (SQLite index, `.mnemonic/index.sqlite3`, WAL), `ingestion`/`chunker` (text → chunks), `embedding` (fastembed), `search` (RRF hybrid rank), `indexer` (background worker) | Everything in the index is derived and rebuildable. Retrieval runs inside SQLite (`knn_chunks`, `keyword_chunks`). |
| `llm/` | `candle_engine` (Qwen2.5 GGUF), `prompt` (strict-grounding RAG prompt, `SIMILARITY_THRESHOLD`), `stream` (`Generator` trait + worker) | Retrieval stays in the caller; `llm` only builds prompts and generates. |
| `graph/` | `model` (nodes/edges from notes, PDFs, links, similarity), `layout` (force-directed) | Pure, no egui. Drawing is in `app::graph`. |
| `api/` | Egui-free `VaultService` (CRUD, links, graph, reindex, search, ask, sheets, canvas sections + Mermaid export), `memory/` (agent memory §3.10: `overview`, piecewise `edit` with `content_hash`/`if_hash`, `recall` = remember/recall/related, MCP `tools`), `mcp` (JSON-RPC over stdio), `types` (stable JSON shapes) | Used by `src/bin/mnemonic-cli/` (`main.rs` + `memory.rs`). Adding a capability: service method → `types` struct → CLI subcommand → MCP tool + schema → tests. A ref to a title several notes share is an error (pass the path). |
| `app/` | `MnemonicApp` state machine, editor (`editor/`: `live`, `source`, `panel`, `canvas_surface`), `reading` (themes, print/export), grid, palette, graph view, PDF view | Owns workers (`IndexingWorker`, `GenerationWorker`) and polls them per frame. |
| `ui/` | Widgets, theme, sidebar, top bar, chat sidebar, toasts, modals | Presentation only. |
| `canvas/` | Diagram model (`element`, `diagram_kinds`: ER entity, UML class, relations, `ConnectorMeta`), painter (+ `painter_content` Markdown/table/mermaid inside boxes, `painter_diagram`), `outline` (section boxes as a mind map), `mermaid_export` / `mermaid_import` (§3.9.4), Draw.io import/export, `jsoncanvas` (Obsidian JSON Canvas sidecar `<note>.canvas`), `BlockBinding` | Bound nodes derive their text from the Markdown (`scope: block` = one `^id` block, `segment` = a whole section/table/fence via `markdown::sections`); diagram-only shapes live only in the sidecar. Canvas → Mermaid is one diagram per family; every export must pass `mermaid::validate`. Legacy ```` ```drawio ```` fences still open. |
| `mermaid/` | Native Mermaid (§3.7): `source` (frontmatter/directives), per-type parsers (`flowchart`, `sequence`, `class`, `state`, `er`, `pie`, `mindmap`), `layout::layered` (Sugiyama), `route`, `scene` (display list), `paint` (egui, only egui file), `svg`, `theme`, `text` | ```` ```mermaid ```` fences are the diagram format for agents; `.canvas` stays for free-form boards. Parsers return `Diagnostic`s, never panic. Add a type: parser+build module → `dispatch` in `mermaid/mod.rs` → `is_supported`. |
| `pdf/` | Extraction (lopdf/pdf-extract), rendering (PDFium, loaded at runtime), annotation | PDFium may be absent; renderer degrades. |
| `sheet/` | CSV/XLSX sheets (§3.8): `model` (table, sort/filter/stats, number parsing), `history` (undoable edits), `csv_io`, `xlsx_io`, `ingest` (rows → `Column: value` chunks) | CSV/TSV are editable and the source of truth (delimiter/BOM/CRLF kept, atomic save); workbooks are read-only — never write them back, only convert/export. Grid UI is `app::sheet` + `app::sheet_grid`; agent API is `api::sheet`. |
| `block/` | Block tree/store used for Markdown ↔ canvas projections | Block anchors themselves live in `markdown::blocks`. |
| `reading_theme/` | Reading themes (§3.2.5): `ThemeColors` (light/dark/print), TOML plugin loader, `ThemeRegistry`, built-ins from `themes/*.toml` | Declarative only, no code. Plugins in `<config_dir>/mnemonic/themes/` and `<vault>/.mnemonic/themes/`; broken files are reported, never fatal. Add a colour: one line in the `theme_colors!` list + CSS in `export::html`. |
| `export/` | Note → themed standalone HTML (`html`), headless-Chromium PDF + open-with-system (`system`), `resolve_embed` | Egui-free. Uses the theme's `[print]` colours; PDF runs off the UI thread in the app. |
| `i18n/`, `settings.rs` | Fluent locales (`locales/*/main.ftl`), persisted `AppSettings` (`vault_path`, …) | User strings go through Fluent, never hard-coded. |

## Conventions (from the module doc comments)

- Every module starts with a `//!` comment: purpose, spec citation, and a
  `Callers:` line. Keep them current when you change dependencies.
- Errors: `anyhow::{Result, Context}` with `.with_context(|| format!(...))`;
  `bail!` for invariant violations. Never `unwrap` on IO in non-test code.
- Logging: `log::{warn,info,debug}`; binaries call `env_logger`. Nothing
  else writes to stdout in `mnemonic-cli` (MCP uses stdout for protocol).
- Background work (indexing, generation) runs on `std::thread` workers with
  channels polled per frame; a panicking worker must not take the UI down
  (`panic = "unwind"` is deliberate, see `Cargo.toml`).
- Data safety first: atomic note writes, trash instead of delete, index is
  cache. Don't add a second source of truth.
- Tests: inline `#[cfg(test)]` modules next to the code, integration tests
  in `tests/*.rs` using `tempfile`. Tests that need a model download are
  `#[ignore]`d. `IndexStore::open_in_memory()` exists for unit tests.
- Serialized/JSON field names are snake_case and stable once shipped.
- Keep files under ~800 lines; split modules instead of growing them.

## Validation

```
cargo check --all-targets      # must be clean of errors
cargo test --locked            # unit + integration (models not required)
cargo clippy --all-targets     # keep new warnings at zero
cargo test --test cli_tests    # CLI + MCP round trips
cargo test --lib api           # agent-interface unit tests
```

`cargo run` starts the GUI; `cargo run --bin mnemonic-cli -- --vault <dir> notes list --json`
exercises the agent interface. `RUST_LOG=info` for model-loading logs.

## Gotchas

- Edition 2024: `gen` is reserved; let-chains are used freely.
- First semantic index/search downloads `multilingual-e5-small`; first
  `ask` downloads ~1 GB Qwen2.5 GGUF. Use `--keyword-only` in tests.
- `replace_chunks` requires one vector per chunk (dimension =
  `EMBEDDING_DIM`); keyword-only indexing stores zero vectors and a
  `kw:`-prefixed content hash so semantic runs redo those notes.
- `.mnemonic/`, `.trash/` and other dot-folders are never scanned as notes.
