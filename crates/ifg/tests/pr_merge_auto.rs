//! `ifg pr merge --auto`: `--json` stdout must parse as a single JSON
//! document, and `-q/--template` must be honoured instead of ignored.

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
        .args(["-R", "a/r", "pr", "merge", "5", "--auto"]);
    cmd
}

async fn mount_auto_merge(server: &MockServer) {
    Mock::given(method("PUT"))
        .and(path("/api/v1/repos/a/r/pulls/5/auto-merge"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "enabled": true,
            "strategy": "merge",
            "message": "auto-merge enabled"
        })))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn json_mode_prints_only_json() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_auto_merge(&server).await;

    let output = command(&server, &home).args(["--json"]).assert().success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).into_owned();
    // The old output appended "Auto-merge enabled …" after the JSON, which
    // broke every scripted consumer.
    let value: Value = serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!("stdout is not one JSON document ({error}): {stdout:?}");
    });
    assert_eq!(value["enabled"], true);

    let stderr = String::from_utf8_lossy(&output.get_output().stderr).into_owned();
    assert!(stderr.contains("Auto-merge enabled"), "stderr: {stderr}");
}

#[tokio::test]
async fn template_mode_renders_the_response() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_auto_merge(&server).await;

    let output = command(&server, &home)
        .args(["-q", "{{.strategy}}"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).into_owned();
    assert_eq!(stdout.trim(), "merge");
}
