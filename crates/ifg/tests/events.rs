//! `ifg events list`: one cursor read per run, never a wait.

use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::{
    matchers::{method, path, query_param, query_param_is_missing},
    Mock, MockServer, ResponseTemplate,
};

fn command(server: &MockServer, home: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path());
    cmd
}

fn events_path() -> &'static str {
    "/api/v1/repos/alice/demo/events"
}

/// The server body carries a field the snapshot does not describe; `--json`
/// must still print it.
fn server_body() -> Value {
    json!({
        "events": [
            {
                "created_at": "2026-11-09T10:00:00Z",
                "kind": "issue.opened",
                "actor": {"username": "alice"},
                "summary": "opened #3",
                "future_field": {"kept": true}
            },
            {
                "created_at": "2026-11-09T10:05:00Z",
                "type": "push",
                "actor": "bob",
                "message": "pushed main"
            }
        ],
        "next_cursor": 57,
        "server_only": "kept"
    })
}

#[tokio::test]
async fn sends_cursor_kinds_and_limit_and_never_asks_to_wait() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path(events_path()))
        .and(query_param("since", "42"))
        .and(query_param("types", "issue,pr"))
        .and(query_param("limit", "5"))
        .and(query_param_is_missing("timeout"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"events": [], "next_cursor": 42})),
        )
        .expect(1)
        .mount(&server)
        .await;

    command(&server, &home)
        .args([
            "-R",
            "alice/demo",
            "events",
            "list",
            "--since",
            "42",
            "--kind",
            "issue",
            "--kind",
            "pr",
            "--limit",
            "5",
            "--json",
        ])
        .assert()
        .success();
}

#[tokio::test]
async fn without_options_the_request_has_no_query_string() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path(events_path()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"next_cursor": 57})))
        .expect(1)
        .mount(&server)
        .await;

    command(&server, &home)
        .args(["-R", "alice/demo", "events", "list"])
        .assert()
        .success();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url.query(),
        None,
        "no since, types, limit or timeout"
    );
}

#[tokio::test]
async fn json_prints_the_server_object_unchanged() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path(events_path()))
        .and(query_param("since", "40"))
        .respond_with(ResponseTemplate::new(200).set_body_json(server_body()))
        .mount(&server)
        .await;

    let output = command(&server, &home)
        .args([
            "-R",
            "alice/demo",
            "events",
            "list",
            "--since",
            "40",
            "--json",
        ])
        .assert()
        .success();
    let printed: Value =
        serde_json::from_slice(&output.get_output().stdout).expect("valid JSON object");
    assert_eq!(printed, server_body());
    assert_eq!(printed["next_cursor"], json!(57));
}

#[tokio::test]
async fn human_output_has_one_row_per_event_and_the_cursor_on_stderr() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path(events_path()))
        .and(query_param("since", "40"))
        .respond_with(ResponseTemplate::new(200).set_body_json(server_body()))
        .mount(&server)
        .await;

    let output = command(&server, &home)
        .args(["-R", "alice/demo", "events", "list", "--since", "40"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr).into_owned();

    let row_one = stdout
        .lines()
        .find(|line| line.contains("issue.opened"))
        .expect("row for the first event");
    for field in ["2026-11-09", "alice", "opened #3"] {
        assert!(row_one.contains(field), "{row_one}");
    }
    let row_two = stdout
        .lines()
        .find(|line| line.contains("pushed main"))
        .expect("row for the second event");
    for field in ["push", "bob"] {
        assert!(row_two.contains(field), "{row_two}");
    }
    assert!(stderr.contains("next cursor: 57"), "{stderr}");
    assert!(
        !stdout.contains("next cursor"),
        "stdout stays a table: {stdout}"
    );
}

/// Without `--since` the server answers with only a cursor. That is the
/// starting position, so stdout stays empty and stderr says so.
#[tokio::test]
async fn cursor_only_response_without_since_is_a_starting_position() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path(events_path()))
        .and(query_param_is_missing("since"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"next_cursor": 57})))
        .expect(1)
        .mount(&server)
        .await;

    let output = command(&server, &home)
        .args(["-R", "alice/demo", "events", "list"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr).into_owned();
    assert!(stdout.is_empty(), "no events were printed: {stdout:?}");
    assert!(stderr.contains("next cursor: 57"), "{stderr}");
    assert!(stderr.contains("starting position"), "{stderr}");
    assert!(!stderr.contains("No events"), "{stderr}");
}

#[tokio::test]
async fn empty_read_after_a_cursor_says_so_on_stdout() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path(events_path()))
        .and(query_param("since", "57"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"events": [], "next_cursor": 57})),
        )
        .mount(&server)
        .await;

    let output = command(&server, &home)
        .args(["-R", "alice/demo", "events", "list", "--since", "57"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).into_owned();
    assert_eq!(stdout, "No events after cursor 57.\n");
}

#[tokio::test]
async fn terminal_control_characters_are_stripped_from_rows() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path(events_path()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "events": [{
                "kind": "issue\u{1b}[2J.opened",
                "actor": "al\u{7}ice",
                "summary": "ok\u{1b}]0;pwned\u{7}\u{1b}[2J line\nbreak \u{202e}done"
            }],
            "next_cursor": 9
        })))
        .mount(&server)
        .await;

    let output = command(&server, &home)
        .args(["-R", "alice/demo", "events", "list", "--since", "1"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).into_owned();
    for bad in ['\u{1b}', '\u{7}', '\u{202e}'] {
        assert!(
            !stdout.contains(bad),
            "control character {bad:?} leaked: {stdout:?}"
        );
    }
    assert!(stdout.contains("done"), "{stdout}");
    assert!(stdout.contains("line break"), "{stdout}");
}

#[tokio::test]
async fn missing_repository_exits_with_not_found_code() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path(events_path()))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "error": {"code": "not_found", "message": "repository not found"}
        })))
        .mount(&server)
        .await;

    command(&server, &home)
        .args([
            "-R",
            "alice/demo",
            "events",
            "list",
            "--since",
            "1",
            "--json",
        ])
        .assert()
        .code(8);
}

#[tokio::test]
async fn invalid_options_fail_before_any_request() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    for args in [
        vec!["--kind", "issue,pr"],
        vec!["--kind", ""],
        vec!["--limit", "201"],
        vec!["--since", "abc"],
    ] {
        command(&server, &home)
            .args(["-R", "alice/demo", "events", "list"])
            .args(&args)
            .assert()
            .code(2);
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}
