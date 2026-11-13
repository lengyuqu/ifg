//! `ifg pr checks`: the combined status of the PR's head commit. Requests go
//! PR detail first, then the combined status; a failing check is not an error.

use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

fn pull(head_sha: Value) -> Value {
    json!({
        "id": 1, "repo_id": 1, "number": 12, "title": "Add checks", "body": null,
        "state": "open", "is_draft": false, "author_id": 1,
        "head_branch": "feature", "base_branch": "main",
        "head_sha": head_sha, "merge_commit_sha": null, "head_repo_id": null,
        "created_at": "2026-11-09T10:00:00Z", "updated_at": "2026-11-09T10:00:00Z"
    })
}

fn check(context: &str, state: &str, description: &str, target: Option<&str>) -> Value {
    json!({
        "context": context, "state": state, "description": description,
        "target_url": target
    })
}

fn combined(state: &str, statuses: Vec<Value>) -> Value {
    json!({
        "state": state,
        "sha": SHA,
        "total_count": statuses.len(),
        "statuses": statuses,
        "repository": {"owner": "a", "name": "r"}
    })
}

async fn mount_pr(server: &MockServer, head_sha: Value, status_code: u16) {
    let body = if status_code == 200 {
        pull(head_sha)
    } else {
        json!({"error": {"code": "NOT_FOUND", "message": "pull request not found", "request_id": null}})
    };
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/pulls/12"))
        .respond_with(ResponseTemplate::new(status_code).set_body_json(body))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_status(server: &MockServer, body: Value) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/repos/a/r/commits/{SHA}/status")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(1)
        .mount(server)
        .await;
}

fn ifg(server: &MockServer, home: &TempDir, extra: &[&str]) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["-R", "a/r", "pr", "checks", "12"])
        .args(extra);
    cmd
}

fn stdout(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

#[tokio::test]
async fn requests_pull_then_combined_status_in_order() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_pr(&server, json!(SHA), 200).await;
    mount_status(&server, combined("pending", vec![])).await;

    ifg(&server, &home, &[]).assert().success();

    let requests = server.received_requests().await.unwrap();
    let paths: Vec<String> = requests
        .iter()
        .map(|req| format!("{} {}", req.method, req.url.path()))
        .collect();
    assert_eq!(
        paths,
        vec![
            "GET /api/v1/repos/a/r/pulls/12".to_string(),
            format!("GET /api/v1/repos/a/r/commits/{SHA}/status"),
        ]
    );
}

#[tokio::test]
async fn json_wraps_the_status_object_unchanged() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let status = combined(
        "failure",
        vec![
            check("ci/build", "success", "built", Some("https://ci.example/1")),
            check("ci/test", "failure", "3 tests failed", None),
        ],
    );
    mount_pr(&server, json!(SHA), 200).await;
    mount_status(&server, status.clone()).await;

    let assert = ifg(&server, &home, &["--json"]).assert().success();
    let value: Value = serde_json::from_str(&stdout(&assert)).unwrap();

    let object = value.as_object().expect("one object");
    assert_eq!(
        object.len(),
        3,
        "keys: {:?}",
        object.keys().collect::<Vec<_>>()
    );
    assert_eq!(value["number"], json!(12));
    assert_eq!(value["head_sha"], json!(SHA));
    assert_eq!(value["status"], status);
}

#[tokio::test]
async fn template_reads_the_same_aggregate_object() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_pr(&server, json!(SHA), 200).await;
    mount_status(&server, combined("success", vec![])).await;

    let assert = ifg(&server, &home, &["--template", "{{.head_sha}}"])
        .assert()
        .success();
    assert_eq!(stdout(&assert).trim(), SHA);
}

#[tokio::test]
async fn human_output_lists_pending_checks() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_pr(&server, json!(SHA), 200).await;
    mount_status(
        &server,
        combined(
            "pending",
            vec![
                check("ci/build", "success", "built", Some("https://ci.example/1")),
                check("ci/test", "pending", "running", None),
            ],
        ),
    )
    .await;

    let assert = ifg(&server, &home, &[]).assert().success();
    let out = stdout(&assert);
    let header = out.lines().next().unwrap();
    assert_eq!(header, "PR #12 @ 0123456  state: pending");
    assert!(out.contains("ci/build"), "stdout: {out}");
    assert!(out.contains("ci/test"), "stdout: {out}");
    assert!(out.contains("pending"), "stdout: {out}");
    assert!(out.contains("https://ci.example/1"), "stdout: {out}");
    assert!(!out.contains("No checks"), "stdout: {out}");
}

#[tokio::test]
async fn human_output_reports_failures_with_exit_zero() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_pr(&server, json!(SHA), 200).await;
    mount_status(
        &server,
        combined(
            "failure",
            vec![
                check("ci/build", "success", "built", None),
                check("lint", "failure", "2 warnings as errors", None),
                check("deploy", "pending", "waiting for approval", None),
            ],
        ),
    )
    .await;

    let assert = ifg(&server, &home, &[]).assert().code(0);
    let out = stdout(&assert);
    assert!(
        out.starts_with("PR #12 @ 0123456  state: failure"),
        "stdout: {out}"
    );
    assert!(out.contains("lint"), "stdout: {out}");
    assert!(out.contains("2 warnings as errors"), "stdout: {out}");
    assert!(out.contains("waiting for approval"), "stdout: {out}");
}

#[tokio::test]
async fn no_checks_is_said_explicitly() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_pr(&server, json!(SHA), 200).await;
    mount_status(&server, combined("success", vec![])).await;

    let assert = ifg(&server, &home, &[]).assert().success();
    let out = stdout(&assert);
    assert!(
        out.contains("No checks reported for this commit."),
        "stdout: {out}"
    );
}

#[tokio::test]
async fn missing_pull_request_exits_with_not_found() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_pr(&server, json!(SHA), 404).await;
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/repos/a/r/commits/{SHA}/status")))
        .respond_with(ResponseTemplate::new(200).set_body_json(combined("success", vec![])))
        .expect(0)
        .mount(&server)
        .await;

    ifg(&server, &home, &[]).assert().code(8);
}

#[tokio::test]
async fn missing_head_sha_fails_without_calling_status() {
    for head in [Value::Null, json!("")] {
        let server = MockServer::start().await;
        let home = TempDir::new().unwrap();
        mount_pr(&server, head.clone(), 200).await;

        let assert = ifg(&server, &home, &[]).assert().failure();
        let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
        assert!(
            stderr.contains("has no head commit SHA"),
            "head {head}: stderr: {stderr}"
        );
        let requests = server.received_requests().await.unwrap();
        assert!(
            requests
                .iter()
                .all(|req| req.url.path() == "/api/v1/repos/a/r/pulls/12"),
            "head {head}: only the PR request may be sent"
        );
    }
}

#[tokio::test]
async fn control_characters_in_check_text_are_stripped() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_pr(&server, json!(SHA), 200).await;
    mount_status(
        &server,
        combined(
            "failure",
            vec![check(
                "ci/build",
                "failure",
                "ok\u{1b}]0;pwned\u{7}done\u{1b}[2J",
                None,
            )],
        ),
    )
    .await;

    let assert = ifg(&server, &home, &[]).assert().success();
    let out = stdout(&assert);
    assert!(!out.contains('\u{1b}'), "escape leaked: {out:?}");
    assert!(!out.contains('\u{7}'), "bell leaked: {out:?}");
    assert!(out.contains("ok]0;pwneddone[2J"), "stdout: {out}");
}
