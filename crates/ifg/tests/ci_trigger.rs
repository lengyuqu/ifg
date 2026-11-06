use assert_cmd::Command;
use serde_json::json;
use tempfile::TempDir;
use wiremock::{
    matchers::{body_json, method, path},
    Mock, MockServer, ResponseTemplate,
};

fn command(server: &MockServer, home: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["-R", "alice/demo"]);
    cmd
}

#[tokio::test]
async fn trigger_sends_ref_and_wait_returns_result() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/alice/demo/pipelines"))
        .and(body_json(json!({"ref_name":"refs/heads/feature"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id":42,"status":"pending"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/pipelines/42"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"pipeline":{"id":42,"status":"success"},"stages":[]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let result = command(&server, &home)
        .args([
            "--json",
            "ci",
            "run",
            "--ref",
            "refs/heads/feature",
            "--wait",
            "--timeout",
            "5",
        ])
        .assert()
        .success();
    let result: serde_json::Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
    assert_eq!(result["pipeline"]["status"], "success");
}

#[tokio::test]
async fn trigger_without_ref_or_wait_uses_server_default() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/alice/demo/pipelines"))
        .and(body_json(json!({})))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(json!({"id": 9, "status": "pending"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let output = command(&server, &home)
        .args(["ci", "run"])
        .assert()
        .success();
    let response: serde_json::Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert_eq!(response["id"], 9);
}

#[tokio::test]
async fn wait_fails_closed_for_terminal_failure_block_and_unknown() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    for (id, status) in [
        (1, "failed"),
        (2, "manual"),
        (3, "waiting_approval"),
        (4, "future_state"),
    ] {
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/repos/alice/demo/pipelines/{id}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"pipeline":{"id":id,"status":status},"stages":[]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        command(&server, &home)
            .args(["ci", "wait", &id.to_string(), "--timeout", "5"])
            .assert()
            .code(1);
    }
    command(&server, &home)
        .args(["ci", "wait", "1", "--timeout", "0"])
        .assert()
        .code(2);
}

#[tokio::test]
async fn wait_times_out_instead_of_hanging() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/pipelines/5"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"pipeline":{"id":5,"status":"pending"},"stages":[]})),
        )
        .mount(&server)
        .await;
    let output = command(&server, &home)
        .args(["ci", "wait", "5", "--timeout", "1"])
        .assert()
        .code(1);
    assert!(String::from_utf8_lossy(&output.get_output().stderr).contains("did not finish"));
}
