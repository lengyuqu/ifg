//! `-q/--template` renders server strings straight into a terminal, so it
//! must apply the same control-character sanitizing as tables and comments.

use assert_cmd::Command;
use serde_json::json;
use tempfile::TempDir;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

fn command(server: &MockServer, home: &TempDir, template: &str) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["-R", "a/r", "issue", "view", "1", "-q", template]);
    cmd
}

async fn mount_issue(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/issues/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 1, "repo_id": 1, "number": 1,
            // OSC 0 + CSI clear-screen inside a title, and a body that must
            // keep its newline, tab and CJK text.
            "title": "evil\u{1b}]0;pwned\u{7}title",
            "body": "第一行\n第二行\t结束\u{1b}[2J",
            "state": "open", "author_id": 1, "labels": "[]",
            "author": "alice", "assignees": [],
            "created_at": "2026-11-09T10:00:00Z"
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn template_output_has_no_terminal_escapes() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_issue(&server).await;

    let output = command(&server, &home, "{{.title}}|{{.body}}")
        .assert()
        .success();
    let stdout = output.get_output().stdout.clone();
    assert!(
        !stdout.contains(&0x1b),
        "escape byte survived: {:?}",
        String::from_utf8_lossy(&stdout)
    );
    assert!(!stdout.contains(&0x07), "BEL survived");
    let text = String::from_utf8(stdout).unwrap();
    assert!(text.contains("evil]0;pwnedtitle"), "text: {text:?}");
    // Legitimate layout and non-ASCII content are preserved.
    assert!(text.contains("第一行\n第二行\t结束"), "text: {text:?}");
}

#[tokio::test]
async fn template_keeps_multiline_layout() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_issue(&server).await;

    let output = command(&server, &home, "{{.body}}").assert().success();
    let text = String::from_utf8(output.get_output().stdout.clone()).unwrap();
    assert_eq!(text.lines().count(), 2, "text: {text:?}");
    assert!(text.contains('\t'), "tab dropped: {text:?}");
}
