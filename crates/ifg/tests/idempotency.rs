//! `--idempotency-key` / `IRONFORGE_IDEMPOTENCY_KEY` end to end: which command
//! sends the header, how a refusal happens before any request, and how the
//! server's answers surface.

use std::time::Duration;

use assert_cmd::{assert::Assert, Command};
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, Request, ResponseTemplate,
};

const COMMENT_PATH: &str = "/api/v1/repos/a/r/issues/3/comments";

fn ifg(server: &MockServer, home: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .env_remove("IRONFORGE_IDEMPOTENCY_KEY")
        .env_remove("IFG_NO_INPUT")
        .current_dir(home.path())
        .timeout(Duration::from_secs(30));
    cmd
}

fn comment_created() -> ResponseTemplate {
    ResponseTemplate::new(201)
        .set_body_json(json!({"id": 1, "issue_id": 3, "author_id": 1, "body": "hi"}))
}

/// `Idempotency-Key` header of every request the server received.
async fn keys_of(server: &MockServer) -> Vec<Option<String>> {
    let requests: Vec<Request> = server.received_requests().await.unwrap_or_default();
    requests
        .iter()
        .map(|request| {
            request
                .headers
                .get("idempotency-key")
                .map(|value| value.to_str().unwrap().to_string())
        })
        .collect()
}

fn stderr_of(assert: &Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stderr).into_owned()
}

/// The last stderr line under `--json` is the error object.
fn json_error(assert: &Assert) -> Value {
    let stderr = stderr_of(assert);
    let last = stderr.lines().last().expect("stderr has a line");
    serde_json::from_str(last).expect("last stderr line is the JSON error object")
}

#[tokio::test]
async fn flag_key_is_sent_on_the_supported_write() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("POST"))
        .and(path(COMMENT_PATH))
        .and(header("Idempotency-Key", "comment-7"))
        .respond_with(comment_created())
        .expect(1)
        .mount(&server)
        .await;

    ifg(&server, &home)
        .args([
            "--idempotency-key",
            "comment-7",
            "-R",
            "a/r",
            "issue",
            "comment",
            "3",
            "-b",
            "hi",
        ])
        .assert()
        .success();
    assert_eq!(keys_of(&server).await, vec![Some("comment-7".to_string())]);
}

#[tokio::test]
async fn env_var_supplies_the_key_and_the_flag_overrides_it() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("POST"))
        .and(path(COMMENT_PATH))
        .and(header("Idempotency-Key", "from-env"))
        .respond_with(comment_created())
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(COMMENT_PATH))
        .and(header("Idempotency-Key", "from-flag"))
        .respond_with(comment_created())
        .expect(1)
        .mount(&server)
        .await;

    ifg(&server, &home)
        .env("IRONFORGE_IDEMPOTENCY_KEY", "from-env")
        .args(["-R", "a/r", "issue", "comment", "3", "-b", "hi"])
        .assert()
        .success();
    ifg(&server, &home)
        .env("IRONFORGE_IDEMPOTENCY_KEY", "from-env")
        .args([
            "--idempotency-key",
            "from-flag",
            "-R",
            "a/r",
            "issue",
            "comment",
            "3",
            "-b",
            "hi",
        ])
        .assert()
        .success();
    assert_eq!(
        keys_of(&server).await,
        vec![Some("from-env".to_string()), Some("from-flag".to_string())]
    );
}

#[tokio::test]
async fn keyed_write_is_retried_after_503_with_the_same_key() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("POST"))
        .and(path(COMMENT_PATH))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(COMMENT_PATH))
        .respond_with(comment_created())
        .mount(&server)
        .await;

    ifg(&server, &home)
        .args([
            "--idempotency-key",
            "retry-me",
            "-R",
            "a/r",
            "issue",
            "comment",
            "3",
            "-b",
            "hi",
        ])
        .assert()
        .success();
    assert_eq!(
        keys_of(&server).await,
        vec![Some("retry-me".to_string()), Some("retry-me".to_string())]
    );
}

#[tokio::test]
async fn unkeyed_write_is_not_retried_after_503() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("POST"))
        .and(path(COMMENT_PATH))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;

    ifg(&server, &home)
        .args(["-R", "a/r", "issue", "comment", "3", "-b", "hi"])
        .assert()
        .code(1);
    assert_eq!(keys_of(&server).await, vec![None]);
}

#[tokio::test]
async fn read_command_refuses_the_flag_and_sends_nothing() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(0)
        .mount(&server)
        .await;

    let assert = ifg(&server, &home)
        .args(["--idempotency-key", "k-read", "-R", "a/r", "issue", "list"])
        .assert()
        .code(2);
    let stderr = stderr_of(&assert);
    assert!(stderr.contains("`ifg issue list`"), "{stderr}");
    assert!(stderr.contains("--idempotency-key"), "{stderr}");
}

#[tokio::test]
async fn multi_write_command_refuses_the_flag_and_sends_nothing() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 1})))
        .expect(0)
        .mount(&server)
        .await;
    let asset = home.path().join("app.tar.gz");
    std::fs::write(&asset, b"bytes").unwrap();

    let assert = ifg(&server, &home)
        .args([
            "--idempotency-key",
            "k-release",
            "-R",
            "a/r",
            "release",
            "create",
            "v1",
            asset.to_str().unwrap(),
        ])
        .assert()
        .code(2);
    let stderr = stderr_of(&assert);
    assert!(stderr.contains("`ifg release create`"), "{stderr}");
}

#[tokio::test]
async fn env_var_on_a_refused_command_is_a_usage_error_too() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(0)
        .mount(&server)
        .await;

    let assert = ifg(&server, &home)
        .env("IRONFORGE_IDEMPOTENCY_KEY", "k-env")
        .args(["-R", "a/r", "issue", "list"])
        .assert()
        .code(2);
    assert!(stderr_of(&assert).contains("IRONFORGE_IDEMPOTENCY_KEY"));
}

#[tokio::test]
async fn malformed_key_is_a_usage_error_before_any_request() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(wiremock::matchers::any())
        .respond_with(comment_created())
        .expect(0)
        .mount(&server)
        .await;

    for bad in ["has space", "semi;colon", ""] {
        let assert = ifg(&server, &home)
            .args([
                "--idempotency-key",
                bad,
                "-R",
                "a/r",
                "issue",
                "comment",
                "3",
                "-b",
                "hi",
            ])
            .assert()
            .code(2);
        let stderr = stderr_of(&assert);
        assert!(stderr.contains("--idempotency-key"), "{stderr}");
        assert!(stderr.contains("1 to 255"), "{stderr}");
    }
}

#[tokio::test]
async fn server_refusal_surfaces_as_reason_in_the_json_error_object() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("POST"))
        .and(path(COMMENT_PATH))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {
                "code": "IDEMPOTENCY_NOT_SUPPORTED",
                "message": "this route does not support idempotency keys",
                "request_id": null,
                "reason": "IDEMPOTENCY_NOT_SUPPORTED",
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let assert = ifg(&server, &home)
        .args([
            "--json",
            "--idempotency-key",
            "k-400",
            "-R",
            "a/r",
            "issue",
            "comment",
            "3",
            "-b",
            "hi",
        ])
        .assert()
        .code(1);
    let report = json_error(&assert);
    assert_eq!(report["error"]["kind"], "other");
    assert_eq!(report["error"]["http_status"], 400);
    assert_eq!(report["error"]["reason"], "IDEMPOTENCY_NOT_SUPPORTED");
    assert_eq!(report["error"]["retryable"], false);
}

#[tokio::test]
async fn schema_reports_which_commands_accept_the_key() {
    let home = TempDir::new().unwrap();
    let schema_for = |args: &[&str]| -> Value {
        let output = Command::cargo_bin("ifg")
            .unwrap()
            .current_dir(home.path())
            .env("XDG_CONFIG_HOME", home.path())
            .env("HOME", home.path())
            .arg("schema")
            .args(args)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        serde_json::from_slice(&output).unwrap()
    };
    assert_eq!(
        schema_for(&["issue", "create"])["commands"][0]["accepts_idempotency_key"],
        true
    );
    assert_eq!(
        schema_for(&["release", "create"])["commands"][0]["accepts_idempotency_key"],
        false
    );
    assert_eq!(
        schema_for(&["issue", "list"])["commands"][0]["accepts_idempotency_key"],
        false
    );
}
