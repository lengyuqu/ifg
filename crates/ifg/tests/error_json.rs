//! `--json` failures end stderr with one error object and never touch stdout;
//! without `--json` the human output is unchanged (plus an optional `hint:`).

use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(url: &str, home: &TempDir, args: &[&str]) -> Run {
    let output = Command::cargo_bin("ifg")
        .unwrap()
        .args(args)
        .env("IRONFORGE_URL", url)
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
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

async fn serve(status: u16, body: Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo"))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .expect(1)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn conflict_under_json_reports_reason_and_hint() {
    let server = serve(
        409,
        json!({"error": {
            "code": "CONFLICT",
            "message": "branch already exists",
            "request_id": null,
            "reason": "branch_exists",
            "hint": "choose another name",
        }}),
    )
    .await;
    let home = TempDir::new().unwrap();
    let result = run(&server.uri(), &home, &["api", "repos/alice/demo", "--json"]);

    assert_eq!(result.code, 1);
    assert_eq!(result.stdout, "", "error object must not reach stdout");
    assert_eq!(
        result.stderr.lines().next(),
        Some("ifg: API error (HTTP 409): branch already exists")
    );
    assert_eq!(
        last_line_as_error(&result.stderr),
        json!({"error": {
            "kind": "conflict",
            "http_status": 409,
            "reason": "branch_exists",
            "hint": "choose another name",
            "message": "API error (HTTP 409): branch already exists",
            "retryable": false,
        }})
    );
}

#[tokio::test]
async fn not_found_under_json_reports_kind_and_status() {
    let server = serve(
        404,
        json!({"error": {"code": "NOT_FOUND", "message": "repository not found", "request_id": null}}),
    )
    .await;
    let home = TempDir::new().unwrap();
    let result = run(&server.uri(), &home, &["api", "repos/alice/demo", "--json"]);

    assert_eq!(result.code, 8);
    assert_eq!(result.stdout, "");
    assert_eq!(
        last_line_as_error(&result.stderr),
        json!({"error": {
            "kind": "not_found",
            "http_status": 404,
            "reason": null,
            "hint": null,
            "message": "API error (HTTP 404): repository not found",
            "retryable": false,
        }})
    );
}

#[tokio::test]
async fn failure_without_json_keeps_the_human_output() {
    let server = serve(
        404,
        json!({"error": {"code": "NOT_FOUND", "message": "repository not found", "request_id": null}}),
    )
    .await;
    let home = TempDir::new().unwrap();
    let result = run(&server.uri(), &home, &["api", "repos/alice/demo"]);

    assert_eq!(result.code, 8);
    assert_eq!(result.stdout, "");
    assert_eq!(
        result.stderr,
        "ifg: API error (HTTP 404): repository not found\n"
    );
}

#[tokio::test]
async fn usage_failure_under_json_is_kind_usage() {
    let home = TempDir::new().unwrap();
    let result = run(
        "http://127.0.0.1:9",
        &home,
        &["api", "search", "-X", "POST", "--paginate", "--json"],
    );

    assert_eq!(result.code, 2);
    assert_eq!(result.stdout, "");
    let error = last_line_as_error(&result.stderr);
    assert_eq!(error["error"]["kind"], "usage");
    assert_eq!(error["error"]["http_status"], Value::Null);
    assert_eq!(error["error"]["retryable"], false);
}

#[tokio::test]
async fn clap_refusal_under_json_still_exits_2_with_an_object() {
    let home = TempDir::new().unwrap();
    let result = run("http://127.0.0.1:9", &home, &["--json", "no-such-command"]);

    assert_eq!(result.code, 2);
    assert_eq!(result.stdout, "");
    assert!(
        result.stderr.contains("unrecognized subcommand"),
        "stderr: {}",
        result.stderr
    );
    let error = last_line_as_error(&result.stderr);
    assert_eq!(error["error"]["kind"], "usage");
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unrecognized subcommand"));
}

#[tokio::test]
async fn transport_failure_under_json_is_retryable_network() {
    let home = TempDir::new().unwrap();
    // POST is never retried by the client, so a refused connection fails fast.
    let result = run(
        "http://127.0.0.1:9",
        &home,
        &["api", "users/me", "-X", "POST", "--json"],
    );

    assert_eq!(result.code, 1);
    assert_eq!(result.stdout, "");
    let error = last_line_as_error(&result.stderr);
    assert_eq!(error["error"]["kind"], "network");
    assert_eq!(error["error"]["http_status"], Value::Null);
    assert_eq!(error["error"]["reason"], Value::Null);
    assert_eq!(error["error"]["hint"], Value::Null);
    assert_eq!(error["error"]["retryable"], true);
}
