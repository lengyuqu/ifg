//! CLI tests for completion conditions, completion status, budgets, usage,
//! repository rules and the overview. Writes carry session credentials from a
//! private session file, so the file-based tests are Unix-only like the
//! session storage.
#![cfg(unix)]

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::{
    matchers::{any, body_json, header, method, path, query_param, query_param_is_missing},
    Match, Mock, MockServer, Request, ResponseTemplate,
};

const COLLAB: &str = "/api/v1/repos/a/r/collaboration";

/// Matches only when the named header is absent. Reads must not carry
/// session credentials.
struct HeaderAbsent(&'static str);

impl Match for HeaderAbsent {
    fn matches(&self, request: &Request) -> bool {
        !request.headers.contains_key(self.0)
    }
}

/// Write a private session file bound to this server and repository.
fn session_file(dir: &TempDir, server: &MockServer) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.path().join("session.json");
    let record = json!({
        "host": server.uri(),
        "repo": "a/r",
        "id": "sid",
        "secret": "ssecret",
    });
    std::fs::write(&path, record.to_string()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    path
}

/// Write a JSON file in `dir` and return its path.
fn json_file(dir: &TempDir, name: &str, value: &Value) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, value.to_string()).unwrap();
    path
}

/// `ifg --json -R a/r collab --session-file <file> <args...>`, run from a
/// directory that is not a git checkout.
fn ifg(
    server: &MockServer,
    home: &TempDir,
    work: &TempDir,
    session: &Path,
    args: &[&str],
) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .current_dir(work.path())
        .args(["--json", "-R", "a/r", "collab", "--session-file"])
        .arg(session)
        .args(args);
    cmd
}

fn stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout is one JSON value")
}

fn conflict() -> ResponseTemplate {
    ResponseTemplate::new(409).set_body_json(
        json!({"error":{"code":"VERSION_CONFLICT","reason":"stale_version","message":"stale"}}),
    )
}

#[tokio::test]
async fn condition_list_is_a_read_without_session_headers() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    let response = json!({
        "task_id": "t1",
        "version": 2,
        "conditions": [{"id": "c1", "type": "manual_confirmation", "label": "Reviewed", "params": {}}],
        "created_by_kind": "session",
        "created_by_id": "sid",
        "created_at": "2026-11-01T00:00:00Z",
        "origin": "api",
    });
    Mock::given(method("GET"))
        .and(path(format!("{COLLAB}/tasks/t1/completion-conditions")))
        .and(HeaderAbsent("x-ironforge-session"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let out = ifg(
        &server,
        &home,
        &work,
        &session,
        &["condition", "list", "t1"],
    )
    .assert()
    .success();
    assert_eq!(stdout_json(out.get_output()), response);
}

#[tokio::test]
async fn condition_set_sends_file_body_unchanged_with_session() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    let body = json!({
        "client_request_id": "cond-1",
        "expected_version": 2,
        "conditions": [
            {"type": "manual_confirmation", "label": "Reviewed", "params": {}},
            {"type": "ci_passed_for_commit", "params": {"commit_sha": "0123456789abcdef0123456789abcdef01234567"}},
        ],
    });
    let file = json_file(&dir, "conditions.json", &body);
    let response = json!({"task_id":"t1","version":3,"replayed":false});
    Mock::given(method("PUT"))
        .and(path(format!("{COLLAB}/tasks/t1/completion-conditions")))
        .and(header("authorization", "Bearer ifp_test"))
        .and(header("x-ironforge-session", "sid"))
        .and(header("x-ironforge-session-secret", "ssecret"))
        .and(body_json(body.clone()))
        .respond_with(ResponseTemplate::new(201).set_body_json(response.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let out = ifg(
        &server,
        &home,
        &work,
        &session,
        &["condition", "set", "t1", "--file"],
    )
    .arg(&file)
    .assert()
    .success();
    assert_eq!(stdout_json(out.get_output()), response);
}

#[tokio::test]
async fn condition_set_reads_body_from_stdin_with_dash() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    let body = json!({
        "client_request_id": "cond-2",
        "expected_version": 0,
        "conditions": [],
    });
    Mock::given(method("PUT"))
        .and(path(format!("{COLLAB}/tasks/t1/completion-conditions")))
        .and(header("x-ironforge-session", "sid"))
        .and(body_json(body.clone()))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"version":1})))
        .expect(1)
        .mount(&server)
        .await;
    ifg(
        &server,
        &home,
        &work,
        &session,
        &["condition", "set", "t1", "--file", "-"],
    )
    .write_stdin(body.to_string())
    .assert()
    .success();
}

#[tokio::test]
async fn condition_set_rejects_non_object_or_invalid_json_before_any_request() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    Mock::given(any())
        .respond_with(ResponseTemplate::new(201))
        .expect(0)
        .mount(&server)
        .await;
    for (name, text) in [
        ("broken.json", "{\"client_request_id\":"),
        ("array.json", "[]"),
        ("string.json", "\"conditions\""),
    ] {
        let file = dir.path().join(name);
        std::fs::write(&file, text).unwrap();
        ifg(
            &server,
            &home,
            &work,
            &session,
            &["condition", "set", "t1", "--file"],
        )
        .arg(&file)
        .assert()
        .code(2);
    }
    ifg(
        &server,
        &home,
        &work,
        &session,
        &["condition", "set", "t1", "--file", "-"],
    )
    .write_stdin("not json")
    .assert()
    .code(2);
}

#[tokio::test]
async fn condition_set_conflict_exits_nonzero_without_retry() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    let file = json_file(
        &dir,
        "stale.json",
        &json!({"client_request_id":"cond-3","expected_version":1,"conditions":[]}),
    );
    Mock::given(method("PUT"))
        .and(path(format!("{COLLAB}/tasks/t1/completion-conditions")))
        .respond_with(conflict())
        .expect(1)
        .mount(&server)
        .await;
    ifg(
        &server,
        &home,
        &work,
        &session,
        &["condition", "set", "t1", "--file"],
    )
    .arg(&file)
    .assert()
    .code(1);
}

#[tokio::test]
async fn condition_confirm_sends_key_and_note_with_session() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    let response = json!({"id":"conf1","condition_id":"c1","replayed":false});
    Mock::given(method("POST"))
        .and(path(format!(
            "{COLLAB}/tasks/t1/completion-conditions/c1/confirmations"
        )))
        .and(header("x-ironforge-session", "sid"))
        .and(header("x-ironforge-session-secret", "ssecret"))
        .and(body_json(json!({
            "client_request_id": "confirm-1",
            "note": "checked by hand",
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(response.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let out = ifg(
        &server,
        &home,
        &work,
        &session,
        &[
            "condition",
            "confirm",
            "t1",
            "c1",
            "--key",
            "confirm-1",
            "--note",
            "checked by hand",
        ],
    )
    .assert()
    .success();
    assert_eq!(stdout_json(out.get_output()), response);
}

#[tokio::test]
async fn condition_confirm_omits_unset_note() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    Mock::given(method("POST"))
        .and(path(format!(
            "{COLLAB}/tasks/t1/completion-conditions/c1/confirmations"
        )))
        .and(body_json(json!({"client_request_id": "confirm-2"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id":"conf2"})))
        .expect(1)
        .mount(&server)
        .await;
    ifg(
        &server,
        &home,
        &work,
        &session,
        &["condition", "confirm", "t1", "c1", "--key", "confirm-2"],
    )
    .assert()
    .success();
}

#[tokio::test]
async fn condition_confirm_conflict_exits_nonzero_without_retry() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    Mock::given(method("POST"))
        .and(path(format!(
            "{COLLAB}/tasks/t1/completion-conditions/c1/confirmations"
        )))
        .respond_with(
            ResponseTemplate::new(409).set_body_json(
                json!({"error":{"code":"IDEMPOTENCY_KEY_REUSED","message":"reused"}}),
            ),
        )
        .expect(1)
        .mount(&server)
        .await;
    ifg(
        &server,
        &home,
        &work,
        &session,
        &["condition", "confirm", "t1", "c1", "--key", "confirm-3"],
    )
    .assert()
    .code(1);
}

#[tokio::test]
async fn task_status_is_a_read_with_json_passthrough() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    let response = json!({
        "task_id": "t1",
        "conditions": [{"id": "c1", "status": "unknown"}],
        "read_only": true,
    });
    Mock::given(method("GET"))
        .and(path(format!("{COLLAB}/tasks/t1/completion-status")))
        .and(HeaderAbsent("x-ironforge-session"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let out = ifg(&server, &home, &work, &session, &["task", "status", "t1"])
        .assert()
        .success();
    assert_eq!(stdout_json(out.get_output()), response);
}

#[tokio::test]
async fn budget_view_is_a_read_with_json_passthrough() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    let response = json!({"task_id":"t1","budgets":[],"unmatched_usage":[]});
    Mock::given(method("GET"))
        .and(path(format!("{COLLAB}/tasks/t1/budget")))
        .and(HeaderAbsent("x-ironforge-session"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let out = ifg(&server, &home, &work, &session, &["budget", "view", "t1"])
        .assert()
        .success();
    assert_eq!(stdout_json(out.get_output()), response);
}

#[tokio::test]
async fn usage_list_passes_filters_and_is_a_read() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    let filtered = json!({"items":[{"id":"u1","unit":"tokens"}],"next_cursor":"c2"});
    Mock::given(method("GET"))
        .and(path(format!("{COLLAB}/tasks/t1/usage")))
        .and(query_param("unit", "tokens"))
        .and(query_param("source", "ci"))
        .and(query_param("cursor", "c1"))
        .and(query_param("limit", "2"))
        .and(HeaderAbsent("x-ironforge-session"))
        .respond_with(ResponseTemplate::new(200).set_body_json(filtered.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let plain = json!({"items":[],"next_cursor":null});
    Mock::given(method("GET"))
        .and(path(format!("{COLLAB}/tasks/t2/usage")))
        .and(query_param_is_missing("unit"))
        .and(query_param_is_missing("source"))
        .and(query_param_is_missing("cursor"))
        .and(query_param_is_missing("limit"))
        .respond_with(ResponseTemplate::new(200).set_body_json(plain.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let out = ifg(
        &server,
        &home,
        &work,
        &session,
        &[
            "usage", "list", "t1", "--unit", "tokens", "--source", "ci", "--cursor", "c1",
            "--limit", "2",
        ],
    )
    .assert()
    .success();
    assert_eq!(stdout_json(out.get_output()), filtered);
    let out = ifg(&server, &home, &work, &session, &["usage", "list", "t2"])
        .assert()
        .success();
    assert_eq!(stdout_json(out.get_output()), plain);
}

#[tokio::test]
async fn usage_report_sends_session_and_exact_body() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    let response = json!({"id":"u1","unit":"tokens","amount":"0.42"});
    Mock::given(method("POST"))
        .and(path(format!("{COLLAB}/tasks/t1/usage")))
        .and(header("authorization", "Bearer ifp_test"))
        .and(header("x-ironforge-session", "sid"))
        .and(header("x-ironforge-session-secret", "ssecret"))
        .and(body_json(json!({
            "client_usage_id": "usage-1",
            "unit": "tokens",
            "source": "ci",
            "amount": "0.42",
            "period_start": "2026-11-01T00:00:00Z",
            "period_end": "2026-11-02T00:00:00Z",
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(response.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let out = ifg(
        &server,
        &home,
        &work,
        &session,
        &[
            "usage",
            "report",
            "t1",
            "--key",
            "usage-1",
            "--unit",
            "tokens",
            "--source",
            "ci",
            "--amount",
            "0.42",
            "--period-start",
            "2026-11-01T00:00:00Z",
            "--period-end",
            "2026-11-02T00:00:00Z",
        ],
    )
    .assert()
    .success();
    assert_eq!(stdout_json(out.get_output()), response);
}

#[tokio::test]
async fn usage_report_conflict_exits_nonzero_without_retry() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    Mock::given(method("POST"))
        .and(path(format!("{COLLAB}/tasks/t1/usage")))
        .respond_with(
            ResponseTemplate::new(409)
                .set_body_json(json!({"error":{"code":"USAGE_LIMIT_REACHED","message":"limit"}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    ifg(
        &server,
        &home,
        &work,
        &session,
        &[
            "usage",
            "report",
            "t1",
            "--key",
            "usage-2",
            "--unit",
            "tokens",
            "--source",
            "ci",
            "--amount",
            "1",
            "--period-start",
            "2026-11-01T00:00:00Z",
            "--period-end",
            "2026-11-02T00:00:00Z",
        ],
    )
    .assert()
    .code(1);
}

#[tokio::test]
async fn rules_is_a_read_with_optional_ref_and_path() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    let response = json!({"resolved_commit":"abc","rules":[{"path":"AGENTS.md"}]});
    Mock::given(method("GET"))
        .and(path(format!("{COLLAB}/rules")))
        .and(query_param("ref", "main"))
        .and(query_param("path", "src/lib.rs"))
        .and(HeaderAbsent("x-ironforge-session"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response.clone()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{COLLAB}/rules")))
        .and(query_param_is_missing("ref"))
        .and(query_param_is_missing("path"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"rules":[]})))
        .expect(1)
        .mount(&server)
        .await;
    let out = ifg(
        &server,
        &home,
        &work,
        &session,
        &["rules", "--ref", "main", "--path", "src/lib.rs"],
    )
    .assert()
    .success();
    assert_eq!(stdout_json(out.get_output()), response);
    let out = ifg(&server, &home, &work, &session, &["rules"])
        .assert()
        .success();
    assert_eq!(stdout_json(out.get_output()), json!({"rules":[]}));
}

#[tokio::test]
async fn overview_is_a_read_with_sections_and_limit() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    let response = json!({"sections":[{"name":"awaiting_me","available":true,"items":[]}],"consistency":"per_section_live_query"});
    Mock::given(method("GET"))
        .and(path(format!("{COLLAB}/overview")))
        .and(query_param("sections", "awaiting_me,in_progress"))
        .and(query_param("limit", "10"))
        .and(HeaderAbsent("x-ironforge-session"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let out = ifg(
        &server,
        &home,
        &work,
        &session,
        &[
            "overview",
            "--sections",
            "awaiting_me,in_progress",
            "--limit",
            "10",
        ],
    )
    .assert()
    .success();
    assert_eq!(stdout_json(out.get_output()), response);
}
