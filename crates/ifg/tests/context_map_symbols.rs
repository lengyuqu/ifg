//! `ifg context map`, `symbols` and `index`: request shapes, the checks that
//! happen before a request, the human views (sanitized, caveats kept) and
//! the error exits.

use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(server: &MockServer, args: &[&str]) -> Run {
    let home = TempDir::new().unwrap();
    let output = Command::cargo_bin("ifg")
        .unwrap()
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["-R", "a/r"])
        .args(args)
        .output()
        .unwrap();
    Run {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

fn map_body() -> Value {
    json!({
        "ref": "main",
        "commit_sha": SHA,
        "budget": 4000,
        "estimated_tokens": 216,
        "truncated": true,
        "directories": [
            {"path": "", "files": 1, "languages": ["rust"]},
            {"path": "src", "files": 2, "languages": ["rust", "svelte"]}
        ],
        "files": [{
            "path": "src/lib.rs",
            "language": "rust",
            "notice": "partial\nparse \u{1b}[2J",
            "items": [{
                "name": "Greeter",
                "kind": "struct",
                "start_line": 1,
                "end_line": 3,
                "signature": "pub struct Greeter \u{1b}]0;pwned\u{7}{",
                "exported": true
            }, {
                "name": "Greeter",
                "kind": "impl",
                "start_line": 5,
                "end_line": 13,
                "signature": "impl Greeter {",
                "exported": false,
                "children": [{
                    "name": "new",
                    "kind": "method",
                    "start_line": 6,
                    "end_line": 8,
                    "signature": "pub fn new(name: &str) -> Self {",
                    "exported": true
                }]
            }],
            "items_omitted": true
        }],
        "skipped": [{"path": "big.rs", "reason": "file too large"}]
    })
}

fn symbols_body(items: Value, partial: bool) -> Value {
    json!({
        "commit_sha": SHA,
        "partial": partial,
        "limitations": ["Go interface calls are not resolvable"],
        "items": items
    })
}

async fn mount_get(server: &MockServer, route: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path(route))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

fn query_of(request: &Request) -> String {
    request.url.query().unwrap_or("").to_string()
}

#[tokio::test]
async fn map_sends_only_given_parameters_and_passes_json_through() {
    let server = MockServer::start().await;
    mount_get(&server, "/api/v1/ai/repos/a/r/map", map_body()).await;

    let all = run(
        &server,
        &[
            "context", "map", "src/app", "--ref", "dev", "--budget", "64", "--json",
        ],
    );
    assert_eq!(all.code, 0, "{}", all.stderr);
    assert_eq!(
        serde_json::from_str::<Value>(&all.stdout).unwrap(),
        map_body()
    );
    let none = run(&server, &["context", "map", "--json"]);
    assert_eq!(none.code, 0);

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    let query = query_of(&requests[0]);
    assert!(query.contains("ref=dev"), "{query}");
    assert!(query.contains("path=src%2Fapp") || query.contains("path=src/app"));
    assert!(query.contains("budget=64"), "{query}");
    assert_eq!(query_of(&requests[1]), "", "no flags, no query string");
}

#[tokio::test]
async fn map_budget_out_of_range_is_a_usage_error_without_a_request() {
    let server = MockServer::start().await;
    for budget in ["63", "32001", "x"] {
        let result = run(&server, &["context", "map", "--budget", budget]);
        assert_eq!(result.code, 2, "budget {budget}: {}", result.stderr);
    }
    assert!(server.received_requests().await.unwrap().is_empty());
    mount_get(&server, "/api/v1/ai/repos/a/r/map", map_body()).await;
    for budget in ["64", "32000"] {
        assert_eq!(
            run(&server, &["context", "map", "--budget", budget]).code,
            0,
            "budget {budget}"
        );
    }
}

#[tokio::test]
async fn map_human_view_shows_structure_notes_and_what_was_left_out() {
    let server = MockServer::start().await;
    mount_get(&server, "/api/v1/ai/repos/a/r/map", map_body()).await;

    let result = run(&server, &["context", "map"]);
    assert_eq!(result.code, 0, "{}", result.stderr);
    let out = result.stdout;
    assert!(!out.contains('\u{1b}') && !out.contains('\u{7}'), "{out:?}");
    assert!(out.contains("a/r @ main (0123456789ab)"), "{out}");
    assert!(out.contains("216 of 4000 tokens"), "{out}");
    assert!(out.contains("Trimmed to fit the budget"), "{out}");
    assert!(out.contains("- /: 1 file(s) [rust]"), "{out}");
    assert!(out.contains("- src: 2 file(s) [rust, svelte]"), "{out}");
    assert!(out.contains("## src/lib.rs (rust)"), "{out}");
    assert!(out.contains("note: partial parse"), "{out}");
    assert!(
        out.contains("  pub struct Greeter ]0;pwned{  (L1-3)"),
        "{out}"
    );
    assert!(out.contains("  impl Greeter {  (L5-13)"), "{out}");
    assert!(
        out.contains("    pub fn new(name: &str) -> Self {  (L6-8)"),
        "children are indented one level deeper:\n{out}"
    );
    assert!(out.contains("more declarations omitted"), "{out}");
    assert!(out.contains("## Skipped (1)"), "{out}");
    assert!(out.contains("- big.rs (file too large)"), "{out}");
}

#[tokio::test]
async fn symbols_send_name_path_ref_and_limit() {
    let server = MockServer::start().await;
    mount_get(
        &server,
        "/api/v1/ai/repos/a/r/symbols",
        symbols_body(json!([]), false),
    )
    .await;

    let by_ref = run(
        &server,
        &[
            "context",
            "symbols",
            "Greeter",
            "--path",
            "src/lib.rs",
            "--ref",
            "dev",
            "--limit",
            "5",
            "--json",
        ],
    );
    assert_eq!(by_ref.code, 0, "{}", by_ref.stderr);
    let by_commit = run(
        &server,
        &[
            "context",
            "symbols",
            "--path",
            "src/lib.rs",
            "--commit",
            SHA,
        ],
    );
    assert_eq!(by_commit.code, 0, "{}", by_commit.stderr);

    let requests = server.received_requests().await.unwrap();
    let first = query_of(&requests[0]);
    for part in ["name=Greeter", "ref=dev", "limit=5"] {
        assert!(first.contains(part), "{first}");
    }
    assert!(first.contains("path=src%2Flib.rs") || first.contains("path=src/lib.rs"));
    let second = query_of(&requests[1]);
    assert!(second.contains(&format!("commit_sha={SHA}")), "{second}");
    assert!(!second.contains("name="), "{second}");
}

#[tokio::test]
async fn symbols_need_a_name_or_a_path_and_a_sane_limit() {
    let server = MockServer::start().await;
    for args in [
        vec!["context", "symbols"],
        vec!["context", "symbols", "x", "--limit", "0"],
        vec!["context", "symbols", "x", "--limit", "201"],
        vec!["context", "symbols", "x", "--ref", "dev", "--commit", SHA],
    ] {
        let result = run(&server, &args);
        assert_eq!(result.code, 2, "{args:?}: {}", result.stderr);
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn symbols_human_view_marks_candidates_and_keeps_the_caveats() {
    let server = MockServer::start().await;
    let items = json!([
        {"name": "Greeter", "kind": "struct", "path": "src/lib.rs", "start_line": 1, "end_line": 3,
         "signature": "pub struct Greeter \u{1b}[2J{", "confidence": "exact"},
        {"name": "GreeterExt", "kind": "trait", "path": "src/ext.rs", "start_line": 4, "end_line": 9,
         "signature": null, "confidence": "name_match"}
    ]);
    mount_get(
        &server,
        "/api/v1/ai/repos/a/r/symbols",
        symbols_body(items, true),
    )
    .await;

    let result = run(&server, &["context", "symbols", "Greeter"]);
    assert_eq!(result.code, 0, "{}", result.stderr);
    let out = result.stdout;
    assert!(!out.contains('\u{1b}'), "{out:?}");
    assert!(out.contains("Symbols in a/r @ 0123456789ab"), "{out}");
    assert!(out.contains("Partial index"), "{out}");
    assert!(out.contains("- struct Greeter  src/lib.rs:L1-3\n"), "{out}");
    assert!(
        out.contains("- trait GreeterExt  src/ext.rs:L4-9 [name_match]"),
        "{out}"
    );
    assert!(out.contains("    pub struct Greeter [2J{"), "{out}");
    assert!(
        !out.contains("Go interface"),
        "with rows the caveats are one pointer, not the full list:\n{out}"
    );
    assert!(out.contains("see \"limitations\" in --json"), "{out}");
}

#[tokio::test]
async fn symbols_with_no_rows_spell_out_the_limitations() {
    let server = MockServer::start().await;
    mount_get(
        &server,
        "/api/v1/ai/repos/a/r/symbols",
        symbols_body(json!([]), false),
    )
    .await;

    let result = run(&server, &["context", "symbols", "ghost"]);
    assert_eq!(result.code, 0, "{}", result.stderr);
    assert!(result.stdout.contains("No matching definitions."));
    assert!(result
        .stdout
        .contains("- Go interface calls are not resolvable"));
}

#[tokio::test]
async fn symbols_on_an_unindexed_commit_exit_8() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/a/r/symbols"))
        .and(query_param("name", "x"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "code": "SYMBOLS_NOT_INDEXED",
            "message": "commit has no symbol index in this repository"
        })))
        .mount(&server)
        .await;

    let result = run(&server, &["context", "symbols", "x"]);
    assert_eq!(result.code, 8, "{}", result.stderr);
    assert!(result.stdout.is_empty());
    assert!(
        result.stderr.contains("no symbol index"),
        "{}",
        result.stderr
    );
}

#[tokio::test]
async fn index_posts_an_empty_body_by_default_and_the_commit_when_given() {
    let server = MockServer::start().await;
    let report = json!({
        "commit_id": 3,
        "commit_sha": SHA,
        "indexed_files": 12,
        "definitions_written": 340,
        "pruned_commits": 1
    });
    Mock::given(method("POST"))
        .and(path("/api/v1/ai/repos/a/r/index"))
        .respond_with(ResponseTemplate::new(200).set_body_json(report.clone()))
        .mount(&server)
        .await;

    let plain = run(&server, &["context", "index"]);
    assert_eq!(plain.code, 0, "{}", plain.stderr);
    assert!(
        plain.stdout.contains("Indexed a/r at 0123456789ab."),
        "{}",
        plain.stdout
    );
    assert!(plain.stdout.contains("files in code search: 12"));
    assert!(plain.stdout.contains("definitions written: 340"));
    assert!(plain.stdout.contains("old commits pruned: 1"));

    let json_run = run(&server, &["context", "index", "--commit", "dev", "--json"]);
    assert_eq!(json_run.code, 0, "{}", json_run.stderr);
    assert_eq!(
        serde_json::from_str::<Value>(&json_run.stdout).unwrap(),
        report
    );

    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[0].body).unwrap(),
        json!({})
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[1].body).unwrap(),
        json!({"commit_sha": "dev"})
    );
}

#[tokio::test]
async fn index_refuses_an_idempotency_key_before_any_request() {
    // The server does not list this route as key-capable.
    let server = MockServer::start().await;
    let result = run(&server, &["context", "index", "--idempotency-key", "k1"]);
    assert_eq!(result.code, 2, "{}", result.stderr);
    assert!(server.received_requests().await.unwrap().is_empty());
}
