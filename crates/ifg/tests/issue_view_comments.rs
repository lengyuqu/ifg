//! `ifg issue view --comments`: machine-readable modes must fetch and attach
//! the comments instead of returning before the request.

use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

fn command(server: &MockServer, home: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["-R", "a/r", "issue", "view", "1", "--comments"]);
    cmd
}

fn issue() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "id": 1, "repo_id": 1, "number": 1, "title": "bug",
        "body": "details", "state": "open", "author_id": 1, "labels": "[]",
        "author": "alice", "assignees": [],
        "created_at": "2026-11-09T10:00:00Z"
    }))
}

fn comments(count: usize) -> ResponseTemplate {
    let data: Vec<Value> = (1..=count)
        .map(|index| {
            json!({
                "id": index as i64, "issue_id": 1, "author_id": 1,
                "body": format!("comment {index}"),
                "created_at": "2026-11-09T10:05:00Z"
            })
        })
        .collect();
    ResponseTemplate::new(200).set_body_json(Value::Array(data))
}

#[tokio::test]
async fn json_mode_attaches_comments() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/issues/1"))
        .respond_with(issue())
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/issues/1/comments"))
        .respond_with(comments(2))
        .expect(1)
        .mount(&server)
        .await;

    let output = command(&server, &home).args(["--json"]).assert().success();
    let value: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    let items = value["comments"].as_array().expect("comments array");
    assert_eq!(items.len(), 2);
    assert_eq!(items[1]["body"], "comment 2");
}

#[tokio::test]
async fn template_mode_can_range_over_comments() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/issues/1"))
        .respond_with(issue())
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/issues/1/comments"))
        .respond_with(comments(2))
        .expect(1)
        .mount(&server)
        .await;

    let output = command(&server, &home)
        .args(["-q", "{{range .comments}}{{.body}};{{end}}"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).into_owned();
    assert_eq!(stdout.trim(), "comment 1;comment 2;");
}

#[tokio::test]
async fn no_comments_yields_an_empty_array() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/issues/1"))
        .respond_with(issue())
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/issues/1/comments"))
        .respond_with(comments(0))
        .mount(&server)
        .await;

    let output = command(&server, &home).args(["--json"]).assert().success();
    let value: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert_eq!(value["comments"].as_array().map(Vec::len), Some(0));
}

/// A failing comment request must not silently render the issue alone.
#[tokio::test]
async fn comment_request_failure_is_an_error() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/issues/1"))
        .respond_with(issue())
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/issues/1/comments"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "error": {"code": "INTERNAL", "message": "boom"}
        })))
        .mount(&server)
        .await;

    command(&server, &home).args(["--json"]).assert().failure();
}
