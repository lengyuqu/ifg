//! `ifg context …`: each subcommand hits its AI aggregate route with the right
//! path and query, `--json` prints the server object unchanged, the human
//! view shows the key fields with server text sanitized, and a 404 exits 8.

use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::{
    matchers::{method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};

fn ifg(server: &MockServer, home: &TempDir, args: &[&str]) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .args(args);
    cmd
}

fn summary_body() -> Value {
    json!({
        "full_name": "alice/demo",
        "default_branch": "main",
        "description": "demo \u{1b}]0;pwned\u{7}repo",
        "stars_count": 3,
        "forks_count": 1,
        "created_at": "2026-11-01T00:00:00Z",
        "updated_at": "2026-11-02T00:00:00Z",
        "server_only_field": {"kept": true}
    })
}

fn tree_body() -> Value {
    json!({
        "ref": "dev",
        "path": "src",
        "commit": "abc123",
        "entries": [
            {"path": "src/main.rs", "sha": "s1", "type": "blob", "size": 120},
            {"path": "src/lib", "sha": "s2", "type": "tree", "size": null}
        ],
        "truncated": true,
        "next_cursor": "CUR1"
    })
}

fn search_body() -> Value {
    json!([{
        "repo_id": 1,
        "file_path": "src/main.rs",
        "file_name": "main.rs",
        "language": "rust",
        "snippet": "fn main() {\u{1b}[2J}\n    println!(\"hi\");"
    }])
}

fn pr_body() -> Value {
    json!({
        "number": 12,
        "title": "Add \u{1b}[2Jfeature",
        "state": "open",
        "body": "line one\nline two",
        "files": [{"path": "a.rs"}],
        "server_only_field": {"x": 1}
    })
}

#[tokio::test]
async fn repo_summary_uses_summary_route_and_passes_json_through() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/summary"))
        .respond_with(ResponseTemplate::new(200).set_body_json(summary_body()))
        .expect(1)
        .mount(&server)
        .await;

    let output = ifg(
        &server,
        &home,
        &["-R", "alice/demo", "context", "repo", "--json"],
    )
    .assert()
    .success();
    let value: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert_eq!(value, summary_body());
}

#[tokio::test]
async fn repo_summary_human_view_has_key_fields_and_no_control_sequences() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/summary"))
        .respond_with(ResponseTemplate::new(200).set_body_json(summary_body()))
        .mount(&server)
        .await;

    let output = ifg(&server, &home, &["-R", "alice/demo", "context", "repo"])
        .assert()
        .success();
    let stdout = output.get_output().stdout.clone();
    assert!(!stdout.contains(&0x1b), "ESC must be stripped");
    assert!(!stdout.contains(&0x07), "BEL must be stripped");
    let text = String::from_utf8(stdout).unwrap();
    assert!(text.contains("# alice/demo"), "{text}");
    assert!(text.contains("- default branch: main"), "{text}");
    assert!(text.contains("- description: demo ]0;pwnedrepo"), "{text}");
    assert!(text.contains("- stars: 3"), "{text}");
    assert!(text.contains("- forks: 1"), "{text}");
    assert!(
        !text.contains("server_only_field"),
        "human view stays compact"
    );
}

#[tokio::test]
async fn tree_sends_only_given_parameters_and_passes_json_through() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/tree"))
        .and(query_param("ref", "dev"))
        .and(query_param("path", "src"))
        .and(query_param("depth", "2"))
        .and(query_param("limit", "50"))
        .and(query_param("cursor", "abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tree_body()))
        .expect(1)
        .mount(&server)
        .await;

    let output = ifg(
        &server,
        &home,
        &[
            "-R",
            "alice/demo",
            "context",
            "tree",
            "src",
            "--ref",
            "dev",
            "--depth",
            "2",
            "--limit",
            "50",
            "--cursor",
            "abc",
            "--json",
        ],
    )
    .assert()
    .success();
    let value: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert_eq!(value, tree_body());
}

#[tokio::test]
async fn tree_without_options_sends_no_query_string() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/tree"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tree_body()))
        .expect(1)
        .mount(&server)
        .await;

    ifg(&server, &home, &["-R", "alice/demo", "context", "tree"])
        .assert()
        .success();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.query(), None);
}

#[tokio::test]
async fn tree_human_view_lists_entries_and_next_cursor() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/tree"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tree_body()))
        .mount(&server)
        .await;

    let output = ifg(
        &server,
        &home,
        &["-R", "alice/demo", "context", "tree", "src"],
    )
    .assert()
    .success();
    let text = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert!(text.contains("# alice/demo @ dev:src"), "{text}");
    assert!(text.contains("- blob src/main.rs (120 bytes)"), "{text}");
    assert!(text.contains("- tree src/lib"), "{text}");
    assert!(text.contains("--cursor CUR1"), "{text}");
}

#[tokio::test]
async fn search_sends_query_and_passes_json_array_through() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/search/code"))
        .and(query_param("q", "fn main"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-total-count", "7")
                .set_body_json(search_body()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let output = ifg(
        &server,
        &home,
        &["-R", "alice/demo", "context", "search", "fn main", "--json"],
    )
    .assert()
    .success();
    let value: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert_eq!(value, search_body());
}

#[tokio::test]
async fn search_human_view_shows_path_snippet_total_and_strips_controls() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/search/code"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-total-count", "7")
                .set_body_json(search_body()),
        )
        .mount(&server)
        .await;

    let output = ifg(
        &server,
        &home,
        &["-R", "alice/demo", "context", "search", "fn main"],
    )
    .assert()
    .success();
    let stdout = output.get_output().stdout.clone();
    assert!(!stdout.contains(&0x1b), "ESC must be stripped");
    let text = String::from_utf8(stdout).unwrap();
    assert!(text.contains("1 of 7 result(s)"), "{text}");
    assert!(text.contains("## src/main.rs (rust)"), "{text}");
    assert!(text.contains("    fn main() {[2J}"), "{text}");
    assert!(text.contains("    println!(\"hi\");"), "{text}");
}

#[tokio::test]
async fn search_template_renders_each_hit() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/search/code"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_body()))
        .mount(&server)
        .await;

    let output = ifg(
        &server,
        &home,
        &[
            "-R",
            "alice/demo",
            "context",
            "search",
            "fn main",
            "-q",
            "{{.file_path}}",
        ],
    )
    .assert()
    .success();
    assert_eq!(output.get_output().stdout, b"src/main.rs\n");
}

#[tokio::test]
async fn pr_context_uses_pull_route_and_passes_json_through() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/pulls/12/context"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body()))
        .expect(1)
        .mount(&server)
        .await;

    let output = ifg(
        &server,
        &home,
        &["-R", "alice/demo", "context", "pr", "12", "--json"],
    )
    .assert()
    .success();
    let value: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert_eq!(value, pr_body());
}

#[tokio::test]
async fn pr_context_human_view_lists_fields_and_strips_controls() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/pulls/12/context"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body()))
        .mount(&server)
        .await;

    let output = ifg(&server, &home, &["-R", "alice/demo", "context", "pr", "12"])
        .assert()
        .success();
    let stdout = output.get_output().stdout.clone();
    assert!(!stdout.contains(&0x1b), "ESC must be stripped");
    let text = String::from_utf8(stdout).unwrap();
    assert!(text.contains("# alice/demo #12"), "{text}");
    assert!(text.contains("- title: Add [2Jfeature"), "{text}");
    assert!(text.contains("- body:\n  line one\n  line two"), "{text}");
    assert!(text.contains("- files: 1 item(s)"), "{text}");
    assert!(text.contains("- state: open"), "{text}");
}

fn issues_body() -> Value {
    json!([
        {
            "number": 3,
            "title": "Crash on\u{1b}[2J start\nsecond line",
            "state": "open",
            "author_id": 7,
            "created_at": "2026-11-03T00:00:00Z",
            "server_only_field": true
        },
        {
            "number": 4,
            "title": "Plain",
            "state": "closed",
            "author_id": 8,
            "created_at": "2026-11-04T00:00:00Z"
        }
    ])
}

fn prs_body() -> Value {
    json!([{
        "number": 12,
        "title": "Add \u{1b}]0;pwned\u{7}feature",
        "state": "merged",
        "head_branch": "feat/x",
        "base_branch": "main",
        "author_id": 7,
        "created_at": "2026-11-05T00:00:00Z"
    }])
}

#[tokio::test]
async fn pr_context_sends_budget_when_given() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/pulls/12/context"))
        .and(query_param("budget", "2000"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body()))
        .expect(1)
        .mount(&server)
        .await;

    let output = ifg(
        &server,
        &home,
        &[
            "-R",
            "alice/demo",
            "context",
            "pr",
            "12",
            "--budget",
            "2000",
            "--json",
        ],
    )
    .assert()
    .success();
    let value: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert_eq!(value, pr_body());
}

#[tokio::test]
async fn pr_context_budget_bounds_are_inclusive() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    for budget in ["500", "50000"] {
        Mock::given(method("GET"))
            .and(path("/api/v1/ai/repos/alice/demo/pulls/12/context"))
            .and(query_param("budget", budget))
            .respond_with(ResponseTemplate::new(200).set_body_json(pr_body()))
            .expect(1)
            .mount(&server)
            .await;
        ifg(
            &server,
            &home,
            &[
                "-R",
                "alice/demo",
                "context",
                "pr",
                "12",
                "--budget",
                budget,
            ],
        )
        .assert()
        .success();
    }
    server.verify().await;
}

#[tokio::test]
async fn pr_context_without_budget_sends_no_budget_parameter() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/pulls/12/context"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pr_body()))
        .expect(1)
        .mount(&server)
        .await;

    ifg(
        &server,
        &home,
        &["-R", "alice/demo", "context", "pr", "12", "--json"],
    )
    .assert()
    .success();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.query(), None);
}

#[tokio::test]
async fn pr_context_budget_out_of_range_is_usage_error_without_request() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    for budget in ["499", "50001", "0", "-1", "abc"] {
        ifg(
            &server,
            &home,
            &[
                "-R",
                "alice/demo",
                "context",
                "pr",
                "12",
                "--budget",
                budget,
            ],
        )
        .assert()
        .code(2);
    }
    let requests = server.received_requests().await.unwrap_or_default();
    assert!(requests.is_empty(), "no request for a bad budget");
}

#[tokio::test]
async fn issues_sends_state_and_limit_and_passes_json_through() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/issues"))
        .and(query_param("state", "closed"))
        .and(query_param("limit", "5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(issues_body()))
        .expect(1)
        .mount(&server)
        .await;

    let output = ifg(
        &server,
        &home,
        &[
            "-R",
            "alice/demo",
            "context",
            "issues",
            "--state",
            "closed",
            "--limit",
            "5",
            "--json",
        ],
    )
    .assert()
    .success();
    let value: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert_eq!(value, issues_body());
}

#[tokio::test]
async fn issues_without_flags_sends_no_query_string() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(issues_body()))
        .expect(1)
        .mount(&server)
        .await;

    ifg(&server, &home, &["-R", "alice/demo", "context", "issues"])
        .assert()
        .success();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.query(), None);
}

#[tokio::test]
async fn issues_human_view_lists_one_line_per_issue_and_strips_controls() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(issues_body()))
        .mount(&server)
        .await;

    let output = ifg(&server, &home, &["-R", "alice/demo", "context", "issues"])
        .assert()
        .success();
    let stdout = output.get_output().stdout.clone();
    assert!(!stdout.contains(&0x1b), "ESC must be stripped");
    let text = String::from_utf8(stdout).unwrap();
    assert!(text.contains("# alice/demo issues"), "{text}");
    assert!(
        text.contains(
            "- #3 Crash on[2J start second line (open, author 7, created 2026-11-03T00:00:00Z)"
        ),
        "{text}"
    );
    assert!(
        text.contains("- #4 Plain (closed, author 8, created 2026-11-04T00:00:00Z)"),
        "{text}"
    );
    assert!(!text.contains("server_only_field"), "{text}");
}

#[tokio::test]
async fn issues_template_renders_each_item() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/issues"))
        .respond_with(ResponseTemplate::new(200).set_body_json(issues_body()))
        .mount(&server)
        .await;

    let output = ifg(
        &server,
        &home,
        &["-R", "alice/demo", "context", "issues", "-q", "{{.number}}"],
    )
    .assert()
    .success();
    assert_eq!(output.get_output().stdout, b"3\n4\n");
}

#[tokio::test]
async fn issues_rejects_unknown_state_without_request() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    ifg(
        &server,
        &home,
        &["-R", "alice/demo", "context", "issues", "--state", "merged"],
    )
    .assert()
    .code(2);
    let requests = server.received_requests().await.unwrap_or_default();
    assert!(requests.is_empty(), "no request for an unknown state");
}

#[tokio::test]
async fn prs_sends_state_and_limit_and_passes_json_through() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/prs"))
        .and(query_param("state", "merged"))
        .and(query_param("limit", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(prs_body()))
        .expect(1)
        .mount(&server)
        .await;

    let output = ifg(
        &server,
        &home,
        &[
            "-R",
            "alice/demo",
            "context",
            "prs",
            "--state",
            "merged",
            "--limit",
            "10",
            "--json",
        ],
    )
    .assert()
    .success();
    let value: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert_eq!(value, prs_body());
}

#[tokio::test]
async fn prs_without_flags_sends_no_query_string() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/prs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(prs_body()))
        .expect(1)
        .mount(&server)
        .await;

    ifg(&server, &home, &["-R", "alice/demo", "context", "prs"])
        .assert()
        .success();
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.query(), None);
}

#[tokio::test]
async fn prs_human_view_shows_branches_and_strips_controls() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/prs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(prs_body()))
        .mount(&server)
        .await;

    let output = ifg(&server, &home, &["-R", "alice/demo", "context", "prs"])
        .assert()
        .success();
    let stdout = output.get_output().stdout.clone();
    assert!(!stdout.contains(&0x1b), "ESC must be stripped");
    assert!(!stdout.contains(&0x07), "BEL must be stripped");
    let text = String::from_utf8(stdout).unwrap();
    assert!(text.contains("# alice/demo pull requests"), "{text}");
    assert!(
        text.contains(
            "- #12 Add ]0;pwnedfeature (merged, feat/x -> main, author 7, created 2026-11-05T00:00:00Z)"
        ),
        "{text}"
    );
}

#[tokio::test]
async fn prs_template_renders_each_item() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/prs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(prs_body()))
        .mount(&server)
        .await;

    let output = ifg(
        &server,
        &home,
        &[
            "-R",
            "alice/demo",
            "context",
            "prs",
            "-q",
            "{{.head_branch}}",
        ],
    )
    .assert()
    .success();
    assert_eq!(output.get_output().stdout, b"feat/x\n");
}

#[tokio::test]
async fn prs_rejects_unknown_state_without_request() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    ifg(
        &server,
        &home,
        &["-R", "alice/demo", "context", "prs", "--state", "all"],
    )
    .assert()
    .code(2);
    let requests = server.received_requests().await.unwrap_or_default();
    assert!(requests.is_empty(), "no request for an unknown state");
}

#[tokio::test]
async fn not_found_exits_with_code_8_for_every_subcommand() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    for route in [
        "/api/v1/ai/repos/alice/demo/summary",
        "/api/v1/ai/repos/alice/demo/tree",
        "/api/v1/ai/repos/alice/demo/search/code",
        "/api/v1/ai/repos/alice/demo/pulls/12/context",
        "/api/v1/ai/repos/alice/demo/issues",
        "/api/v1/ai/repos/alice/demo/prs",
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "error": {"code": "NOT_FOUND", "message": "not found"}
            })))
            .mount(&server)
            .await;
    }
    let cases: [&[&str]; 6] = [
        &["context", "repo"],
        &["context", "tree"],
        &["context", "search", "fn main"],
        &["context", "pr", "12"],
        &["context", "issues"],
        &["context", "prs"],
    ];
    for case in cases {
        let mut args = vec!["-R", "alice/demo"];
        args.extend_from_slice(case);
        let output = ifg(&server, &home, &args).assert().code(8);
        assert!(output.get_output().stdout.is_empty(), "{case:?}");
    }
}

#[tokio::test]
async fn repository_is_inferred_from_git_remote_without_dash_r() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/ai/repos/alice/demo/summary"))
        .respond_with(ResponseTemplate::new(200).set_body_json(summary_body()))
        .expect(1)
        .mount(&server)
        .await;

    let checkout = TempDir::new().unwrap();
    let git = |args: &[&str]| {
        assert!(std::process::Command::new("git")
            .args(args)
            .current_dir(checkout.path())
            .status()
            .unwrap()
            .success());
    };
    git(&["init", "-q"]);
    let remote = format!("{}/git/alice/demo", server.uri());
    git(&["remote", "add", "origin", &remote]);

    Command::cargo_bin("ifg")
        .unwrap()
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .current_dir(checkout.path())
        .args(["context", "repo", "--json"])
        .assert()
        .success();
}
