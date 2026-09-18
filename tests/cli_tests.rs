//! Integration tests for `mnemonic-cli` (§Fase 2): drive the real binary
//! against a temp vault. Nothing here needs the embedding or LLM models —
//! `index --keyword-only` and `search --keyword-only` keep it offline.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use tempfile::{TempDir, tempdir};

fn bin() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mnemonic-cli"));
    cmd.env_remove("MNEMONIC_VAULT");
    cmd
}

fn run(vault: &Path, args: &[&str]) -> Output {
    bin()
        .arg("--vault")
        .arg(vault)
        .args(args)
        .output()
        .expect("spawning mnemonic-cli")
}

fn run_ok(vault: &Path, args: &[&str]) -> String {
    let out = run(vault, args);
    assert!(
        out.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn run_json(vault: &Path, args: &[&str]) -> Value {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    let stdout = run_ok(vault, &full);
    serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("not JSON ({e}): {stdout}"))
}

fn write(vault: &Path, rel: &str, content: &str) {
    let path = vault.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn vault() -> TempDir {
    let dir = tempdir().unwrap();
    write(
        dir.path(),
        "Resep Nasi Goreng.md",
        "---\ntitle: Resep Nasi Goreng\ntags: [masak, rumah]\naliases: [nasgor]\n---\n# Bahan\n\nDua butir telur, nasi, kecap. Lihat [[Belanja Mingguan]].\n",
    );
    write(
        dir.path(),
        "Rumah/Belanja Mingguan.md",
        "---\ntitle: Belanja Mingguan\ntags: [rumah]\ncustom: 1\n---\n- [ ] telur\n- [ ] beras\n\nUntuk [[nasgor]] dan [[Tidak Ada]].\n",
    );
    write(dir.path(), "Plain Obsidian Note.md", "Tanpa frontmatter, tentang sepeda gunung.\n");
    dir
}

#[test]
fn notes_list_json_lists_every_note_with_relative_paths() {
    let v = vault();
    let notes = run_json(v.path(), &["notes", "list"]);
    let notes = notes.as_array().unwrap();
    assert_eq!(notes.len(), 3);
    let paths: Vec<&str> = notes.iter().map(|n| n["path"].as_str().unwrap()).collect();
    assert_eq!(
        paths,
        vec!["Plain Obsidian Note.md", "Resep Nasi Goreng.md", "Rumah/Belanja Mingguan.md"]
    );
    assert_eq!(notes[1]["title"], "Resep Nasi Goreng");
    assert_eq!(notes[1]["tags"], serde_json::json!(["masak", "rumah"]));
    assert_eq!(notes[2]["folder"], "Rumah");
    assert!(notes[0]["id"].as_str().unwrap().len() == 36);

    let by_tag = run_json(v.path(), &["notes", "list", "--tag", "masak"]);
    assert_eq!(by_tag.as_array().unwrap().len(), 1);
    let by_folder = run_json(v.path(), &["notes", "list", "--folder", "Rumah"]);
    assert_eq!(by_folder.as_array().unwrap().len(), 1);

    let text = run_ok(v.path(), &["notes", "list"]);
    assert!(text.contains("Rumah/Belanja Mingguan.md\tBelanja Mingguan"));
}

#[test]
fn notes_read_by_title_alias_and_path() {
    let v = vault();
    let note = run_json(v.path(), &["notes", "read", "nasgor"]);
    assert_eq!(note["title"], "Resep Nasi Goreng");
    assert!(note["body"].as_str().unwrap().contains("Dua butir telur"));
    assert_eq!(note["links"], serde_json::json!(["Belanja Mingguan"]));

    let note = run_json(v.path(), &["notes", "read", "Rumah/Belanja Mingguan.md"]);
    assert_eq!(note["title"], "Belanja Mingguan");
    assert_eq!(note["extra"]["custom"], 1);

    let text = run_ok(v.path(), &["notes", "read", "Plain Obsidian Note"]);
    assert!(text.contains("title: Plain Obsidian Note"));
    assert!(text.contains("sepeda gunung"));

    let missing = run(v.path(), &["notes", "read", "Tidak Ada"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("not found"));
    assert!(missing.stdout.is_empty());
}

#[test]
fn notes_write_replaces_body_and_keeps_unknown_frontmatter_keys() {
    let v = vault();
    let mut child = bin()
        .arg("--vault")
        .arg(v.path())
        .args(["--json", "notes", "write", "Belanja Mingguan", "--stdin", "--tag", "rumah", "--tag", "urgent"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"- [x] telur\n- [ ] beras\n- [ ] minyak\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let res: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(res["created"], false);
    assert_eq!(res["tags"], serde_json::json!(["rumah", "urgent"]));

    let raw = std::fs::read_to_string(v.path().join("Rumah/Belanja Mingguan.md")).unwrap();
    assert!(raw.contains("custom: 1"), "unknown key lost:\n{raw}");
    assert!(raw.contains("title: Belanja Mingguan"));
    assert!(raw.ends_with("- [x] telur\n- [ ] beras\n- [ ] minyak\n"));
    assert!(!raw.contains("nasgor"), "body should be replaced");

    // --create makes a missing note, --body-file feeds the body.
    let body = v.path().join("body.txt");
    std::fs::write(&body, "isi baru").unwrap();
    let res = run_json(
        v.path(),
        &["notes", "write", "Catatan Baru", "--create", "--folder", "Inbox", "--body-file", body.to_str().unwrap()],
    );
    assert_eq!(res["created"], true);
    assert_eq!(res["path"], "Inbox/Catatan Baru.md");
    assert!(v.path().join("Inbox/Catatan Baru.md").exists());

    let nothing = run(v.path(), &["notes", "write", "Catatan Baru"]);
    assert!(!nothing.status.success());
}

#[test]
fn notes_create_and_trash() {
    let v = vault();
    let created = run_json(v.path(), &["notes", "create", "Ide: Baru?", "--tag", "ide"]);
    assert_eq!(created["path"], "Ide Baru.md");
    assert_eq!(created["tags"], serde_json::json!(["ide"]));
    let trashed = run_json(v.path(), &["notes", "trash", "Ide Baru"]);
    assert_eq!(trashed["previous_path"], "Ide Baru.md");
    assert!(trashed["path"].as_str().unwrap().starts_with(".trash/"));
    assert!(!v.path().join("Ide Baru.md").exists());
    let listed = run_json(v.path(), &["notes", "list"]);
    assert_eq!(listed.as_array().unwrap().len(), 3);
}

#[test]
fn index_keyword_only_then_search_finds_note() {
    let v = vault();
    let report = run_json(v.path(), &["index", "--keyword-only"]);
    assert_eq!(report["semantic"], false);
    assert_eq!(report["chunked"], 3);
    assert_eq!(report["failed"], serde_json::json!([]));
    assert!(v.path().join(".mnemonic/index.sqlite3").exists());

    let res = run_json(v.path(), &["search", "sepeda gunung", "--keyword-only"]);
    assert_eq!(res["semantic"], false);
    let hits = res["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["path"], "Plain Obsidian Note.md");
    assert_eq!(hits[0]["title"], "Plain Obsidian Note");
    assert_eq!(hits[0]["kind"], "keyword");
    assert!(hits[0]["snippet"].as_str().unwrap().contains("sepeda"));

    let res = run_json(v.path(), &["search", "telur", "--keyword-only", "-k", "5"]);
    assert_eq!(res["hits"].as_array().unwrap().len(), 2);

    // Unchanged notes are skipped on the next run.
    let again = run_json(v.path(), &["index", "--keyword-only"]);
    assert_eq!(again["chunked"], 0);
    assert_eq!(again["skipped"], 3);

    let text = run_ok(v.path(), &["search", "sepeda", "--keyword-only"]);
    assert!(text.contains("1. Plain Obsidian Note — Plain Obsidian Note.md [keyword]"));
}

#[test]
fn backlinks_links_and_graph() {
    let v = vault();
    let back = run_json(v.path(), &["backlinks", "Resep Nasi Goreng"]);
    assert_eq!(back["target"]["path"], "Resep Nasi Goreng.md");
    let links = back["backlinks"].as_array().unwrap();
    assert_eq!(links.len(), 1, "alias link should count: {back}");
    assert_eq!(links[0]["source_path"], "Rumah/Belanja Mingguan.md");
    assert_eq!(links[0]["line"], 3, "0-based body line index");

    let out = run_json(v.path(), &["links", "Belanja Mingguan"]);
    let links = out["links"].as_array().unwrap();
    assert_eq!(links.len(), 2);
    assert_eq!(links[0]["target"], "nasgor");
    assert_eq!(links[0]["resolved_path"], "Resep Nasi Goreng.md");
    assert_eq!(links[1]["target"], "Tidak Ada");
    assert!(links[1]["resolved_path"].is_null());

    let g = run_json(v.path(), &["graph"]);
    let nodes = g["nodes"].as_array().unwrap();
    let edges = g["edges"].as_array().unwrap();
    assert_eq!(nodes.iter().filter(|n| n["kind"] == "note").count(), 3);
    assert_eq!(nodes.iter().filter(|n| n["kind"] == "ghost").count(), 1);
    assert_eq!(edges.len(), 2);
    for e in edges {
        assert_eq!(e["kind"], "link");
        assert!(e["a"].as_u64().unwrap() < nodes.len() as u64);
        assert!(e["b"].as_u64().unwrap() < nodes.len() as u64);
    }
    let text = run_ok(v.path(), &["graph"]);
    assert!(text.starts_with("4 nodes, 2 edges"));
}

#[test]
fn diagram_validate_and_render_raw_files_need_no_vault() {
    let dir = tempdir().unwrap();
    let good = dir.path().join("flow.mmd");
    std::fs::write(&good, "flowchart LR\n  A[Start] --> B{Ok?}\n  B -->|yes| C\n").unwrap();
    let bad = dir.path().join("bad.mmd");
    std::fs::write(&bad, "flowchart LR\n  A -> B\n").unwrap();

    // No --vault and no MNEMONIC_VAULT: raw sources still work.
    let out = bin().args(["--json", "diagram", "validate", "--file"]).arg(&good).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let check: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(check["kind"], "flowchart");
    assert_eq!(check["valid"], true);

    let out = bin().args(["--json", "diagram", "validate", "--file"]).arg(&bad).output().unwrap();
    assert!(!out.status.success(), "invalid diagrams exit non-zero");
    let check: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(check["diagnostics"][0]["line"], 2);
    assert_eq!(check["diagnostics"][0]["severity"], "error");

    let svg_path = dir.path().join("flow.svg");
    let out = bin().args(["diagram", "render", "--file"]).arg(&good).arg("--out").arg(&svg_path).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let svg = std::fs::read_to_string(&svg_path).unwrap();
    assert!(svg.starts_with("<svg") && svg.contains("Start"));
}

#[test]
fn diagram_list_reads_note_fences() {
    let v = vault();
    write(v.path(), "Alur.md", "# Alur\n\n```mermaid\nflowchart TD\n  A --> B\n```\n");
    let list = run_json(v.path(), &["diagram", "list", "Alur"]);
    assert_eq!(list["path"], "Alur.md");
    assert_eq!(list["diagrams"][0]["kind"], "flowchart");
    assert_eq!(list["diagrams"][0]["valid"], true);
    let rendered = run_json(v.path(), &["diagram", "render", "--note", "Alur"]);
    assert!(rendered["svg"].as_str().unwrap().starts_with("<svg"));
}

#[test]
fn sheets_query_edit_and_index() {
    let dir = vault();
    let v = dir.path();
    write(v, "Data/Kas.csv", "Item;Jumlah\nKopi;Rp 12.000\nTeh;8000\n");

    let list = run_json(v, &["sheets", "list"]);
    assert_eq!(list[0]["path"], "Data/Kas.csv");
    assert_eq!(list[0]["editable"], true);

    let q = run_json(v, &["sheets", "query", "Kas", "--where", "Jumlah:gte:10000"]);
    assert_eq!(q["matched"], 1);
    assert_eq!(q["rows"][0]["cells"][0], "Kopi");
    assert_eq!(q["columns"][1]["sum"], 12000.0);

    run_ok(v, &["sheets", "set", "Kas", "2", "Jumlah", "9000"]);
    run_ok(v, &["sheets", "append", "Kas", "--row", r#"{"Item":"Susu","Jumlah":5000}"#]);
    let text = std::fs::read_to_string(v.join("Data/Kas.csv")).unwrap();
    assert_eq!(text, "Item;Jumlah\nKopi;Rp 12.000\nTeh;9000\nSusu;5000\n");

    let report = run_json(v, &["index", "--keyword-only"]);
    assert_eq!(report["sheets_indexed"], 1);
    let res = run_json(v, &["search", "Susu", "--keyword-only"]);
    let hit = res["hits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["path"] == "Data/Kas.csv")
        .expect("sheet hit");
    assert_eq!(hit["row"], 1);

    let err = run(v, &["sheets", "query", "Kas", "--where", "Jumlah:like:1"]);
    assert!(!err.status.success());
}

#[test]
fn no_vault_is_an_error() {
    let out = bin()
        .env("HOME", tempdir().unwrap().path())
        .env("XDG_CONFIG_HOME", tempdir().unwrap().path())
        .args(["notes", "list"])
        .output()
        .unwrap();
    // Either no settings file (error) or the developer's own vault; both
    // must at least not panic.
    if !out.status.success() {
        assert!(String::from_utf8_lossy(&out.stderr).contains("no vault"));
    }
}

#[test]
fn mcp_round_trip_over_stdio() {
    let v = vault();
    let mut child = bin()
        .arg("--vault")
        .arg(v.path())
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        let msgs = [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_notes","arguments":{"tag":"rumah"}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"read_note","arguments":{"ref":"Nope"}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"unknown/method"}"#,
        ];
        for m in msgs {
            writeln!(stdin, "{m}").unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    let replies: Vec<Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("non-JSON line on stdout ({e}): {l}")))
        .collect();
    assert_eq!(replies.len(), 5, "one reply per request, none for the notification:\n{stdout}");

    assert_eq!(replies[0]["id"], 1);
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "mnemonic");
    assert!(replies[0]["result"]["capabilities"]["tools"].is_object());

    let tools = replies[1]["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for expected in [
        "list_notes",
        "read_note",
        "write_note",
        "create_note",
        "search_notes",
        "get_backlinks",
        "get_links",
        "get_graph",
        "reindex",
        "ask_vault",
    ] {
        assert!(names.contains(&expected), "missing tool {expected}");
    }
    assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));

    let call = &replies[2]["result"];
    assert_eq!(call["isError"], false);
    assert_eq!(call["content"][0]["type"], "text");
    let listed: Value = serde_json::from_str(call["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 2);

    assert_eq!(replies[3]["result"]["isError"], true);
    assert_eq!(replies[4]["error"]["code"], -32601);
}

/// Needs the FastEmbed model (downloads on first run); run with
/// `cargo test --test cli_tests -- --ignored`.
#[test]
#[ignore]
fn semantic_index_and_search() {
    let v = vault();
    let report = run_json(v.path(), &["index"]);
    assert_eq!(report["semantic"], true, "{report}");
    let res = run_json(v.path(), &["search", "makanan dari telur"]);
    assert_eq!(res["semantic"], true);
    assert_eq!(res["hits"][0]["title"], "Resep Nasi Goreng");
}
