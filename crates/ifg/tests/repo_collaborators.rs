use assert_cmd::Command;
use serde_json::json;
use tempfile::TempDir;
use wiremock::{
    matchers::{body_json, method, path},
    Mock, MockServer, ResponseTemplate,
};

#[tokio::test]
async fn update_uses_collaborator_id_and_remove_uses_user_id() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/users/bob"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":123,"username":"bob"})))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/collaborators"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"id":987,"repo_id":2,"user_id":123,"permission":"write"}])),
        )
        .expect(3)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/api/v1/repos/a/r/collaborators/987"))
        .and(body_json(json!({"permission":"write"})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":987,"user_id":123,"permission":"write"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/repos/a/r/collaborators/123"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let command = |args: &[&str]| {
        let mut cmd = Command::cargo_bin("ifg").unwrap();
        cmd.env("IRONFORGE_URL", server.uri())
            .env("IRONFORGE_PAT", "ifp_test")
            .env("XDG_CONFIG_HOME", home.path())
            .args(["-R", "a/r", "repo", "collaborators"])
            .args(args);
        cmd
    };
    command(&["set", "bob", "--permission", "write"])
        .assert()
        .success();
    command(&["remove", "bob", "--yes"]).assert().success();
}

#[tokio::test]
async fn rejected_when_server_does_not_persist_permission() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/users/bob"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":123,"username":"bob"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/collaborators"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([{"id":987,"repo_id":2,"user_id":123,"permission":"read"}])),
        )
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/api/v1/repos/a/r/collaborators/987"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":987,"user_id":123,"permission":"write"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let out = Command::cargo_bin("ifg")
        .unwrap()
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .args([
            "-R",
            "a/r",
            "repo",
            "collaborators",
            "set",
            "bob",
            "--permission",
            "write",
        ])
        .assert()
        .code(1);
    assert!(String::from_utf8_lossy(&out.get_output().stderr).contains("did not persist"));
}
