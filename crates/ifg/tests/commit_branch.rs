//! `ifg commit …` and `ifg branch delete`: request shapes, input handling,
//! summaries, idempotency keys and the refusals that happen before a request.

use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const TIP_MIXED: &str = "0123456789ABCDEF0123456789abcdef01234567";
const TIP: &str = "0123456789abcdef0123456789abcdef01234567";
const NEW: &str = "fedcba9876543210fedcba9876543210fedcba98";
const COMMITS: &str = "/api/v1/repos/a/r/commits";

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run_with(url: &str, home: &TempDir, args: &[&str], stdin: Option<&str>) -> Run {
    let mut command = Command::cargo_bin("ifg").unwrap();
    command
        .env("IRONFORGE_URL", url)
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["-R", "a/r"])
        .args(args);
    if let Some(stdin) = stdin {
        command.write_stdin(stdin.to_string());
    }
    let output = command.output().unwrap();
    Run {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

fn run(url: &str, home: &TempDir, args: &[&str]) -> Run {
    run_with(url, home, args, None)
}

async fn mount(server: &MockServer, http_method: &str, route: &str, status: u16, body: Value) {
    Mock::given(method(http_method))
        .and(path(route))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

async fn requests_to(server: &MockServer, route: &str) -> Vec<Request> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|req| req.url.path() == route)
        .collect()
}

fn body_of(request: &Request) -> Value {
    serde_json::from_slice(&request.body).unwrap()
}

fn batch_answer() -> Value {
    json!({
        "dry_run": false, "branch": "main", "tip_before": TIP, "commit_sha": NEW,
        "files": [
            {"operation": "create", "path": "new.txt", "blob_before": null},
            {"operation": "move", "from": "old.txt", "path": "moved.txt", "blob_before": "b1"},
        ],
    })
}

fn files_file(dir: &TempDir) -> String {
    let file = dir.path().join("files.json");
    std::fs::write(
        &file,
        json!([
            {"operation": "create", "path": "new.txt", "content": "hi"},
            {"operation": "move", "from": "old.txt", "to": "moved.txt", "sha": "b1"},
        ])
        .to_string(),
    )
    .unwrap();
    file.to_str().unwrap().to_string()
}

#[tokio::test]
async fn create_sends_the_plan_and_summarizes_the_commit() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount(&server, "POST", COMMITS, 200, batch_answer()).await;
    let files = files_file(&home);

    let result = run(
        &server.uri(),
        &home,
        &[
            "--idempotency-key",
            "batch-1",
            "commit",
            "create",
            "--branch",
            "main",
            "--expected-sha",
            TIP_MIXED,
            "-m",
            "add and move",
            "--files",
            &files,
        ],
    );
    assert_eq!(result.code, 0, "stderr {}", result.stderr);
    assert!(
        result
            .stdout
            .contains("Committed 2 file(s) to main: fedcba987654 (was 0123456789ab)"),
        "{}",
        result.stdout
    );
    assert!(
        result.stdout.contains("create  new.txt"),
        "{}",
        result.stdout
    );
    assert!(
        result.stdout.contains("move  old.txt -> moved.txt"),
        "{}",
        result.stdout
    );

    let requests = requests_to(&server, COMMITS).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        body_of(&requests[0]),
        json!({
            "branch": "main",
            "expected_sha": TIP,
            "message": "add and move",
            "files": [
                {"operation": "create", "path": "new.txt", "content": "hi"},
                {"operation": "move", "from": "old.txt", "to": "moved.txt", "sha": "b1"},
            ],
        })
    );
    assert_eq!(
        requests[0].headers.get("idempotency-key").unwrap(),
        "batch-1"
    );
}

#[tokio::test]
async fn create_reads_stdin_and_dry_run_prints_the_preview() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let preview = json!({"dry_run": true, "would": [{"effect": "ref_update"}], "not_evaluated": ["ci_trigger"]});
    mount(&server, "POST", COMMITS, 200, preview.clone()).await;

    let result = run_with(
        &server.uri(),
        &home,
        &[
            "commit",
            "create",
            "--branch",
            "main",
            "--expected-sha",
            TIP,
            "-m",
            "m",
            "--files",
            "-",
            "--dry-run",
        ],
        Some(r#"[{"operation":"delete","path":"x","sha":"b"}]"#),
    );
    assert_eq!(result.code, 0, "stderr {}", result.stderr);
    assert!(
        result.stdout.contains("Dry run: nothing was written"),
        "{}",
        result.stdout
    );
    assert!(result.stdout.contains("ref_update"), "{}", result.stdout);
    let sent = body_of(&requests_to(&server, COMMITS).await[0]);
    assert_eq!(sent["dry_run"], true);
    assert_eq!(sent["files"][0]["operation"], "delete");
}

#[tokio::test]
async fn create_json_is_the_server_object() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount(&server, "POST", COMMITS, 200, batch_answer()).await;
    let files = files_file(&home);
    let result = run(
        &server.uri(),
        &home,
        &[
            "commit",
            "create",
            "--branch",
            "main",
            "--expected-sha",
            TIP,
            "-m",
            "m",
            "--files",
            &files,
            "--json",
        ],
    );
    assert_eq!(result.code, 0, "stderr {}", result.stderr);
    assert_eq!(
        serde_json::from_str::<Value>(&result.stdout).unwrap(),
        batch_answer()
    );
}

#[tokio::test]
async fn bad_input_is_a_usage_error_before_any_request() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let mut cases: Vec<(&str, Vec<String>, Option<&str>)> = Vec::new();
    let base = |extra: &[&str]| -> Vec<String> {
        ["commit", "create", "--branch", "main", "-m", "m"]
            .iter()
            .chain(extra)
            .map(|s| s.to_string())
            .collect()
    };
    cases.push((
        "object, not array",
        base(&["--expected-sha", TIP, "--files", "-"]),
        Some(r#"{"files": []}"#),
    ));
    cases.push((
        "empty array",
        base(&["--expected-sha", TIP, "--files", "-"]),
        Some("[]"),
    ));
    cases.push((
        "not json",
        base(&["--expected-sha", TIP, "--files", "-"]),
        Some("nope"),
    ));
    cases.push((
        "short sha",
        base(&["--expected-sha", "0123456", "--files", "-"]),
        Some("[{}]"),
    ));
    cases.push(("no expected sha", base(&["--files", "-"]), Some("[{}]")));
    cases.push(("no files flag", base(&["--expected-sha", TIP]), None));
    for (label, args, stdin) in cases {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let result = run_with(&server.uri(), &home, &refs, stdin);
        assert_eq!(result.code, 2, "{label}: stderr {}", result.stderr);
    }
    // A missing file is an I/O error, still before any request.
    let result = run(
        &server.uri(),
        &home,
        &[
            "commit",
            "create",
            "--branch",
            "main",
            "--expected-sha",
            TIP,
            "-m",
            "m",
            "--files",
            "/nonexistent/files.json",
        ],
    );
    assert_ne!(result.code, 0);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn apply_patch_sends_the_diff_and_omits_a_missing_message() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let route = format!("{COMMITS}/apply-patch");
    mount(
        &server,
        "POST",
        &route,
        200,
        json!({"dry_run": false, "branch": "main", "tip_before": TIP, "commit_sha": NEW,
               "tree_sha": "t", "files_changed": 1,
               "changes": [{"path": "a.txt", "status": "modified"}]}),
    )
    .await;
    let diff = "--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-x\n+y\n";
    let result = run_with(
        &server.uri(),
        &home,
        &[
            "commit",
            "apply-patch",
            "--branch",
            "main",
            "--expected-sha",
            TIP,
            "--patch-file",
            "-",
        ],
        Some(diff),
    );
    assert_eq!(result.code, 0, "stderr {}", result.stderr);
    assert!(
        result
            .stdout
            .contains("Committed 1 file(s) to main: fedcba987654 (was 0123456789ab)"),
        "{}",
        result.stdout
    );
    assert!(
        result.stdout.contains("modified  a.txt"),
        "{}",
        result.stdout
    );
    let sent = body_of(&requests_to(&server, &route).await[0]);
    assert_eq!(
        sent,
        json!({"branch": "main", "expected_sha": TIP, "patch": diff})
    );

    let with_message = run_with(
        &server.uri(),
        &home,
        &[
            "commit",
            "apply-patch",
            "--branch",
            "main",
            "--expected-sha",
            TIP,
            "--patch-file",
            "-",
            "-m",
            "fix x",
        ],
        Some(diff),
    );
    assert_eq!(with_message.code, 0, "stderr {}", with_message.stderr);
    assert_eq!(
        body_of(&requests_to(&server, &route).await[1])["message"],
        "fix x"
    );
}

#[tokio::test]
async fn cherry_pick_and_revert_use_their_own_routes() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    for (verb, operation, said) in [
        ("cherry-pick", "cherry_pick", "Cherry-picked"),
        ("revert", "revert", "Reverted"),
    ] {
        let route = format!("{COMMITS}/abc1234/{verb}");
        mount(
            &server,
            "POST",
            &route,
            200,
            json!({"dry_run": false, "operation": operation, "branch": "main",
                   "tip_before": TIP, "commit_sha": NEW, "source_sha": "abc1234deadbeefabc1234deadbeefabc1234dea"}),
        )
        .await;
        let result = run(
            &server.uri(),
            &home,
            &[
                "commit",
                verb,
                "abc1234",
                "--branch",
                "main",
                "--expected-sha",
                TIP,
                "--mainline",
                "2",
                "-m",
                "why",
            ],
        );
        assert_eq!(result.code, 0, "{verb}: stderr {}", result.stderr);
        assert!(
            result.stdout.contains(&format!(
                "{said} abc1234deadb onto main: fedcba987654 (was 0123456789ab)"
            )),
            "{verb}: {}",
            result.stdout
        );
        assert_eq!(
            body_of(&requests_to(&server, &route).await[0]),
            json!({"branch": "main", "expected_sha": TIP, "mainline": 2, "message": "why"}),
            "{verb}"
        );
    }
    // `--mainline 0` is refused by the parser.
    let zero = run(
        &server.uri(),
        &home,
        &[
            "commit",
            "revert",
            "abc",
            "--branch",
            "main",
            "--expected-sha",
            TIP,
            "--mainline",
            "0",
        ],
    );
    assert_eq!(zero.code, 2, "stderr {}", zero.stderr);
}

#[tokio::test]
async fn a_moved_branch_is_a_409_and_is_not_retried() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("POST"))
        .and(path(format!("{COMMITS}/apply-patch")))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({"error": {
            "code": "CONFLICT", "message": "moved", "request_id": null,
            "reason": "REF_ADVANCED", "hint": "re-read the branch",
        }})))
        .expect(1)
        .mount(&server)
        .await;
    let result = run_with(
        &server.uri(),
        &home,
        &[
            "--json",
            "commit",
            "apply-patch",
            "--branch",
            "main",
            "--expected-sha",
            TIP,
            "--patch-file",
            "-",
        ],
        Some("diff"),
    );
    assert_eq!(result.code, 1, "stderr {}", result.stderr);
    let error: Value = serde_json::from_str(result.stderr.lines().last().unwrap()).unwrap();
    assert_eq!(error["error"]["reason"], "REF_ADVANCED");
}

#[tokio::test]
async fn branch_delete_needs_yes_and_keeps_slashes_in_the_path() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let route = "/api/v1/repos/a/r/branches/feature/my%20x";
    Mock::given(method("DELETE"))
        .and(path(route))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"deleted": true, "branch": "feature/my x", "sha": NEW})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let refused = run(
        &server.uri(),
        &home,
        &["--no-input", "branch", "delete", "feature/my x"],
    );
    assert_eq!(refused.code, 2, "stderr {}", refused.stderr);
    assert!(server.received_requests().await.unwrap().is_empty());

    let result = run(
        &server.uri(),
        &home,
        &[
            "--idempotency-key",
            "del-1",
            "branch",
            "delete",
            "feature/my x",
            "--yes",
            "--expected-sha",
            TIP_MIXED,
        ],
    );
    assert_eq!(result.code, 0, "stderr {}", result.stderr);
    assert!(
        result
            .stdout
            .contains("Deleted branch feature/my x (fedcba987654)."),
        "{}",
        result.stdout
    );
    let requests = requests_to(&server, route).await;
    assert_eq!(
        requests[0].url.query(),
        Some(&*format!("expected_sha={TIP}"))
    );
    assert_eq!(requests[0].headers.get("idempotency-key").unwrap(), "del-1");
}

#[tokio::test]
async fn branch_delete_dry_run_needs_no_confirmation() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let route = "/api/v1/repos/a/r/branches/old";
    mount(
        &server,
        "DELETE",
        route,
        200,
        json!({"dry_run": true, "would_delete": {"branch": "old", "sha": NEW}}),
    )
    .await;
    let result = run(
        &server.uri(),
        &home,
        &["--no-input", "branch", "delete", "old", "--dry-run"],
    );
    assert_eq!(result.code, 0, "stderr {}", result.stderr);
    assert!(
        result.stdout.contains("Dry run: nothing was deleted"),
        "{}",
        result.stdout
    );
    assert_eq!(
        requests_to(&server, route).await[0].url.query(),
        Some("dry_run=true")
    );
}

#[tokio::test]
async fn branch_delete_refusal_surfaces_as_a_json_error() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount(
        &server,
        "DELETE",
        "/api/v1/repos/a/r/branches/main",
        409,
        json!({"error": {"code": "CONFLICT", "message": "default branch", "request_id": null,
                          "reason": "DEFAULT_BRANCH", "hint": "pick another"}}),
    )
    .await;
    let result = run(
        &server.uri(),
        &home,
        &["--json", "branch", "delete", "main", "--yes"],
    );
    assert_eq!(result.code, 1, "stderr {}", result.stderr);
    let error: Value = serde_json::from_str(result.stderr.lines().last().unwrap()).unwrap();
    assert_eq!(error["error"]["reason"], "DEFAULT_BRANCH");
}
