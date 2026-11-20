//! `ifg pr merge --expected-head <SHA>`: the merge body carries the reviewed
//! commit, a refused head surfaces as a 409 `REF_ADVANCED` with no retry, and
//! invalid SHAs or `--auto` are refused before any request.

use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const SHA_MIXED: &str = "0123456789ABCDEF0123456789abcdef01234567";
const SHA_LOWER: &str = "0123456789abcdef0123456789abcdef01234567";
const MERGE_PATH: &str = "/api/v1/repos/a/r/pulls/5/merge";

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(url: &str, home: &TempDir, args: &[&str]) -> Run {
    let output = Command::cargo_bin("ifg")
        .unwrap()
        .env("IRONFORGE_URL", url)
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["-R", "a/r", "pr", "merge", "5"])
        .args(args)
        .output()
        .unwrap();
    Run {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

/// The last stderr line, parsed as the error object.
fn last_line_as_error(stderr: &str) -> Value {
    let line = stderr.lines().last().expect("stderr is empty");
    let value: Value = serde_json::from_str(line)
        .unwrap_or_else(|error| panic!("last stderr line is not JSON ({error}): {line}"));
    assert!(value.get("error").is_some(), "not an error object: {value}");
    value
}

async fn mount_merge(server: &MockServer, status: u16, body: Value) {
    Mock::given(method("POST"))
        .and(path(MERGE_PATH))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .expect(1)
        .mount(server)
        .await;
}

async fn merge_requests(server: &MockServer) -> Vec<Request> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|req| req.url.path() == MERGE_PATH)
        .collect()
}

fn body_json(request: &Request) -> Value {
    serde_json::from_slice(&request.body).expect("request body is JSON")
}

fn merged() -> Value {
    json!({"merge_commit_sha": SHA_LOWER, "strategy": "merge"})
}

fn ref_advanced_body() -> Value {
    json!({"error": {
        "code": "CONFLICT",
        "message": "head branch has advanced",
        "request_id": null,
        "reason": "REF_ADVANCED",
        "hint": "re-read the pull request and retry with the current head sha",
    }})
}

#[tokio::test]
async fn expected_head_is_sent_lower_cased_for_each_strategy() {
    for (flag, strategy) in [
        ("--merge", "merge"),
        ("--squash", "squash"),
        ("--rebase", "rebase"),
    ] {
        let server = MockServer::start().await;
        let home = TempDir::new().unwrap();
        mount_merge(&server, 200, merged()).await;

        let result = run(&server.uri(), &home, &[flag, "--expected-head", SHA_MIXED]);
        assert_eq!(result.code, 0, "{flag}: stderr {}", result.stderr);

        let requests = merge_requests(&server).await;
        assert_eq!(requests.len(), 1, "{flag}");
        assert_eq!(
            body_json(&requests[0]),
            json!({"strategy": strategy, "expected_head_sha": SHA_LOWER}),
            "{flag}"
        );
    }
}

#[tokio::test]
async fn body_is_unchanged_without_the_flag() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_merge(&server, 200, merged()).await;

    let result = run(&server.uri(), &home, &[]);
    assert_eq!(result.code, 0, "stderr {}", result.stderr);

    let requests = merge_requests(&server).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        String::from_utf8(requests[0].body.clone()).unwrap(),
        r#"{"strategy":"merge"}"#
    );
}

#[tokio::test]
async fn invalid_shas_are_usage_errors_with_no_request() {
    let cases = [
        (
            "short",
            "0123456789abcdef0123456789abcdef0123456".to_string(),
        ),
        ("abbreviated", "0123456".to_string()),
        (
            "non-hex",
            "0123456789abcdef0123456789abcdef0123456g".to_string(),
        ),
        ("41 chars", format!("{SHA_LOWER}0")),
        ("empty", String::new()),
    ];
    for (label, sha) in cases {
        let server = MockServer::start().await;
        let home = TempDir::new().unwrap();
        // No mock is mounted: any request would fail the assertion below.
        let result = run(&server.uri(), &home, &["--expected-head", &sha]);

        assert_eq!(result.code, 2, "{label}: stderr {}", result.stderr);
        assert_eq!(result.stdout, "", "{label}");
        assert!(
            merge_requests(&server).await.is_empty(),
            "{label}: a request was sent"
        );
    }
}

#[tokio::test]
async fn expected_head_conflicts_with_auto_before_any_request() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();

    let result = run(
        &server.uri(),
        &home,
        &["--auto", "--expected-head", SHA_LOWER],
    );

    assert_eq!(result.code, 2, "stderr {}", result.stderr);
    assert!(result.stderr.contains("--auto"), "stderr {}", result.stderr);
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty(),
        "no request may be sent"
    );
}

#[tokio::test]
async fn ref_advanced_under_json_is_a_conflict_sent_once() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_merge(&server, 409, ref_advanced_body()).await;

    let result = run(
        &server.uri(),
        &home,
        &["--expected-head", SHA_LOWER, "--json"],
    );

    assert_eq!(result.code, 1);
    assert_eq!(result.stdout, "", "error object must not reach stdout");
    assert_eq!(
        last_line_as_error(&result.stderr),
        json!({"error": {
            "kind": "conflict",
            "http_status": 409,
            "reason": "REF_ADVANCED",
            "hint": "re-read the pull request and retry with the current head sha",
            "message": "API error (HTTP 409): head branch has advanced",
            "retryable": false,
        }})
    );
    // `expect(1)` on the mock also checks this; the explicit count documents
    // that a refused merge is never retried.
    assert_eq!(merge_requests(&server).await.len(), 1);
}

#[tokio::test]
async fn ref_advanced_in_human_mode_points_at_pr_checks() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_merge(&server, 409, ref_advanced_body()).await;

    let result = run(&server.uri(), &home, &["--expected-head", SHA_LOWER]);

    assert_eq!(result.code, 1);
    assert_eq!(result.stdout, "");
    assert!(
        result
            .stderr
            .contains("rerun 'ifg pr checks 5' to read the current head\n"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result
            .stderr
            .contains("ifg: API error (HTTP 409): head branch has advanced\n"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result
            .stderr
            .contains("hint: re-read the pull request and retry with the current head sha\n"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        !result.stderr.contains("reason"),
        "no JSON object in human mode"
    );
    assert_eq!(merge_requests(&server).await.len(), 1);
}

#[tokio::test]
async fn other_failures_do_not_get_the_pr_checks_pointer() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_merge(
        &server,
        409,
        json!({"error": {"code": "CONFLICT", "message": "already merged", "request_id": null}}),
    )
    .await;

    let result = run(&server.uri(), &home, &["--expected-head", SHA_LOWER]);

    assert_eq!(result.code, 1);
    assert!(
        !result.stderr.contains("rerun"),
        "stderr: {}",
        result.stderr
    );
}
