//! CLI tests for collaboration work packages, dependencies and decisions, plus
//! session heartbeat and end. Session credentials come from a private session
//! file, so the file-based tests are Unix-only like the session storage.
#![cfg(unix)]

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::{
    matchers::{any, body_json, header, method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};

const COLLAB: &str = "/api/v1/repos/a/r/collaboration";

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
async fn work_package_create_sends_session_headers_and_exact_body() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    let response = json!({"id":"wp1","task_id":"t1","goal":"ship"});
    Mock::given(method("POST"))
        .and(path(format!("{COLLAB}/tasks/t1/work-packages")))
        .and(header("authorization", "Bearer ifp_test"))
        .and(header("x-ironforge-session", "sid"))
        .and(header("x-ironforge-session-secret", "ssecret"))
        .and(body_json(json!({
            "client_package_id": "wp-1",
            "goal": "ship",
            "scope": "cli",
            "commit_sha": "0123456789abcdef0123456789abcdef01234567",
            "expected_task_version": 3,
            "artifacts": ["report.txt"],
            "completed": ["parser"],
            "next_steps": ["docs", "tests"],
            "open_items": [],
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
            "work-package",
            "create",
            "t1",
            "--key",
            "wp-1",
            "--goal",
            "ship",
            "--scope",
            "cli",
            "--commit",
            "0123456789abcdef0123456789abcdef01234567",
            "--expected-task-version",
            "3",
            "--artifact",
            "report.txt",
            "--completed",
            "parser",
            "--next-step",
            "docs",
            "--next-step",
            "tests",
        ],
    )
    .assert()
    .success();
    assert_eq!(stdout_json(out.get_output()), response);
}

#[tokio::test]
async fn work_package_create_omits_unset_optional_fields() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    Mock::given(method("POST"))
        .and(path(format!("{COLLAB}/tasks/t1/work-packages")))
        .and(body_json(json!({
            "client_package_id": "wp-2",
            "artifacts": [],
            "completed": [],
            "next_steps": [],
            "open_items": [],
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id":"wp2"})))
        .expect(1)
        .mount(&server)
        .await;
    ifg(
        &server,
        &home,
        &work,
        &session,
        &["work-package", "create", "t1", "--key", "wp-2"],
    )
    .assert()
    .success();
}

#[tokio::test]
async fn work_package_create_conflict_exits_nonzero_without_retry() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    Mock::given(method("POST"))
        .and(path(format!("{COLLAB}/tasks/t1/work-packages")))
        .respond_with(conflict())
        .expect(1)
        .mount(&server)
        .await;
    ifg(
        &server,
        &home,
        &work,
        &session,
        &[
            "work-package",
            "create",
            "t1",
            "--key",
            "wp-3",
            "--expected-task-version",
            "1",
        ],
    )
    .assert()
    .code(1);
}

#[tokio::test]
async fn work_package_create_without_session_file_is_a_usage_error() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    Mock::given(any())
        .respond_with(ResponseTemplate::new(201))
        .expect(0)
        .mount(&server)
        .await;
    Command::cargo_bin("ifg")
        .unwrap()
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .current_dir(work.path())
        .args([
            "-R",
            "a/r",
            "collab",
            "work-package",
            "create",
            "t1",
            "--key",
            "wp-4",
        ])
        .assert()
        .code(2);
}

#[tokio::test]
async fn work_package_list_and_view_are_reads_with_json_passthrough() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    let list = json!({"items":[{"id":"wp1","goal":"ship"}],"next_cursor":"c2"});
    Mock::given(method("GET"))
        .and(path(format!("{COLLAB}/tasks/t1/work-packages")))
        .and(query_param("limit", "2"))
        .and(query_param("cursor", "c1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list.clone()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{COLLAB}/work-packages/wp1")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id":"wp1","evidence_resolution":[]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let out = ifg(
        &server,
        &home,
        &work,
        &session,
        &[
            "work-package",
            "list",
            "t1",
            "--limit",
            "2",
            "--cursor",
            "c1",
        ],
    )
    .assert()
    .success();
    assert_eq!(stdout_json(out.get_output()), list);
    let out = ifg(
        &server,
        &home,
        &work,
        &session,
        &["work-package", "view", "wp1"],
    )
    .assert()
    .success();
    assert_eq!(
        stdout_json(out.get_output()),
        json!({"id":"wp1","evidence_resolution":[]})
    );
}

#[tokio::test]
async fn dependency_add_sends_expected_graph_version_and_session() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    let response = json!({"dependency":{"task_id":"t1","depends_on_task_id":"t2"},"graph_version":1,"replayed":false});
    Mock::given(method("POST"))
        .and(path(format!("{COLLAB}/tasks/t1/dependencies")))
        .and(header("x-ironforge-session", "sid"))
        .and(header("x-ironforge-session-secret", "ssecret"))
        .and(body_json(json!({
            "client_request_id": "dep-1",
            "depends_on_task_id": "t2",
            "expected_graph_version": 0,
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
            "dependency",
            "add",
            "t1",
            "t2",
            "--key",
            "dep-1",
            "--expected-graph-version",
            "0",
        ],
    )
    .assert()
    .success();
    assert_eq!(stdout_json(out.get_output()), response);
}

#[tokio::test]
async fn dependency_add_conflict_exits_nonzero_without_retry() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    Mock::given(method("POST"))
        .and(path(format!("{COLLAB}/tasks/t1/dependencies")))
        .respond_with(conflict())
        .expect(1)
        .mount(&server)
        .await;
    ifg(
        &server,
        &home,
        &work,
        &session,
        &[
            "dependency",
            "add",
            "t1",
            "t2",
            "--key",
            "dep-2",
            "--expected-graph-version",
            "4",
        ],
    )
    .assert()
    .code(1);
}

#[tokio::test]
async fn dependency_remove_deletes_edge_with_body_and_session() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    let response = json!({"removed":true,"graph_version":5});
    Mock::given(method("DELETE"))
        .and(path(format!("{COLLAB}/tasks/t1/dependencies/t2")))
        .and(header("x-ironforge-session", "sid"))
        .and(header("x-ironforge-session-secret", "ssecret"))
        .and(body_json(json!({
            "client_request_id": "dep-3",
            "expected_graph_version": 4,
        })))
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
            "dependency",
            "remove",
            "t1",
            "t2",
            "--key",
            "dep-3",
            "--expected-graph-version",
            "4",
        ],
    )
    .assert()
    .success();
    assert_eq!(stdout_json(out.get_output()), response);
}

#[tokio::test]
async fn dependency_remove_conflict_exits_nonzero_without_retry() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    Mock::given(method("DELETE"))
        .and(path(format!("{COLLAB}/tasks/t1/dependencies/t2")))
        .respond_with(conflict())
        .expect(1)
        .mount(&server)
        .await;
    ifg(
        &server,
        &home,
        &work,
        &session,
        &[
            "dependency",
            "remove",
            "t1",
            "t2",
            "--key",
            "dep-4",
            "--expected-graph-version",
            "4",
        ],
    )
    .assert()
    .code(1);
}

#[tokio::test]
async fn dependency_list_is_a_read_with_json_passthrough() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    let response = json!({"task_id":"t1","graph_version":4,"dependencies":[],"blocked_by":[],"status_source":"declared"});
    Mock::given(method("GET"))
        .and(path(format!("{COLLAB}/tasks/t1/dependencies")))
        .respond_with(ResponseTemplate::new(200).set_body_json(response.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let out = ifg(
        &server,
        &home,
        &work,
        &session,
        &["dependency", "list", "t1"],
    )
    .assert()
    .success();
    assert_eq!(stdout_json(out.get_output()), response);
}

#[tokio::test]
async fn decision_create_omits_context_when_no_context_field_is_given() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    Mock::given(method("POST"))
        .and(path(format!("{COLLAB}/tasks/t1/decisions")))
        .and(header("x-ironforge-session", "sid"))
        .and(header("x-ironforge-session-secret", "ssecret"))
        .and(body_json(json!({
            "client_decision_id": "dec-1",
            "title": "Use SQLite",
            "rationale": "Simple deployment",
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id":"d1"})))
        .expect(1)
        .mount(&server)
        .await;
    ifg(
        &server,
        &home,
        &work,
        &session,
        &[
            "decision",
            "create",
            "t1",
            "--key",
            "dec-1",
            "--title",
            "Use SQLite",
            "--rationale",
            "Simple deployment",
        ],
    )
    .assert()
    .success();
}

#[tokio::test]
async fn decision_create_sends_question_and_flattened_context() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    let response = json!({"id":"d2","status":"active"});
    Mock::given(method("POST"))
        .and(path(format!("{COLLAB}/tasks/t1/decisions")))
        .and(body_json(json!({
            "client_decision_id": "dec-2",
            "title": "Pick option one",
            "rationale": "Matches the question",
            "question_id": "q1",
            "answer_id": "a1",
            "context": {"task_version": 7, "base_sha": null, "head_sha": "fedcba9876543210fedcba9876543210fedcba98"},
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
            "decision",
            "create",
            "t1",
            "--key",
            "dec-2",
            "--title",
            "Pick option one",
            "--rationale",
            "Matches the question",
            "--question-id",
            "q1",
            "--answer-id",
            "a1",
            "--task-version",
            "7",
            "--head-sha",
            "fedcba9876543210fedcba9876543210fedcba98",
        ],
    )
    .assert()
    .success();
    assert_eq!(stdout_json(out.get_output()), response);
}

#[tokio::test]
async fn decision_create_conflict_exits_nonzero_without_retry() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    Mock::given(method("POST"))
        .and(path(format!("{COLLAB}/tasks/t1/decisions")))
        .respond_with(conflict())
        .expect(1)
        .mount(&server)
        .await;
    ifg(
        &server,
        &home,
        &work,
        &session,
        &[
            "decision",
            "create",
            "t1",
            "--key",
            "dec-3",
            "--title",
            "T",
            "--rationale",
            "R",
            "--task-version",
            "2",
        ],
    )
    .assert()
    .code(1);
}

#[tokio::test]
async fn decision_list_and_view_are_reads_with_json_passthrough() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    let list = json!({"items":[{"id":"d1","status":"active","context_freshness":"fresh"}],"next_cursor":null});
    Mock::given(method("GET"))
        .and(path(format!("{COLLAB}/tasks/t1/decisions")))
        .and(query_param("limit", "5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let view = json!({"id":"d1","change":null,"supersedes":[]});
    Mock::given(method("GET"))
        .and(path(format!("{COLLAB}/decisions/d1")))
        .respond_with(ResponseTemplate::new(200).set_body_json(view.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let out = ifg(
        &server,
        &home,
        &work,
        &session,
        &["decision", "list", "t1", "--limit", "5"],
    )
    .assert()
    .success();
    assert_eq!(stdout_json(out.get_output()), list);
    let out = ifg(&server, &home, &work, &session, &["decision", "view", "d1"])
        .assert()
        .success();
    assert_eq!(stdout_json(out.get_output()), view);
}

#[tokio::test]
async fn session_heartbeat_and_end_use_session_credentials() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    let heartbeat = json!({"id":"sid","state":"online"});
    Mock::given(method("POST"))
        .and(path(format!("{COLLAB}/sessions/sid/heartbeat")))
        .and(header("authorization", "Bearer ifp_test"))
        .and(header("x-ironforge-session", "sid"))
        .and(header("x-ironforge-session-secret", "ssecret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(heartbeat.clone()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{COLLAB}/sessions/sid/end")))
        .and(header("x-ironforge-session", "sid"))
        .and(header("x-ironforge-session-secret", "ssecret"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let out = ifg(&server, &home, &work, &session, &["session", "heartbeat"])
        .assert()
        .success();
    assert_eq!(stdout_json(out.get_output()), heartbeat);
    // An empty 204 body prints as JSON null, as the collab group always does.
    let out = ifg(&server, &home, &work, &session, &["session", "end"])
        .assert()
        .success();
    assert_eq!(stdout_json(out.get_output()), Value::Null);
}

#[tokio::test]
async fn session_heartbeat_conflict_exits_nonzero_without_retry() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = session_file(&dir, &server);
    Mock::given(method("POST"))
        .and(path(format!("{COLLAB}/sessions/sid/heartbeat")))
        .respond_with(
            ResponseTemplate::new(409)
                .set_body_json(json!({"error":{"code":"SESSION_EXPIRED","message":"expired"}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    ifg(&server, &home, &work, &session, &["session", "heartbeat"])
        .assert()
        .code(1);
}
