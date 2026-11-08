use assert_cmd::Command;
use serde_json::json;
use tempfile::TempDir;
use wiremock::{
    matchers::{body_json, method, path},
    Mock, MockServer, ResponseTemplate,
};

#[tokio::test]
async fn repo_edit_reads_version_and_sends_only_requested_fields() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let endpoint = "/api/v1/repos/a/r";
    Mock::given(method("GET")).and(path(endpoint)).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":1,"owner_id":2,"name":"r","description":"old","is_private":false,"default_branch":"main","stars_count":0,"forks_count":0,"settings_version":"opaque"}))).expect(1).mount(&server).await;
    Mock::given(method("PATCH")).and(path(endpoint)).and(body_json(json!({"expected_version":"opaque","description":null,"is_private":true,"default_branch":"develop"}))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"name":"r"}))).expect(1).mount(&server).await;
    Command::cargo_bin("ifg")
        .unwrap()
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .args([
            "-R",
            "a/r",
            "repo",
            "edit",
            "--clear-description",
            "--visibility",
            "private",
            "--default-branch",
            "develop",
        ])
        .assert()
        .success();
}
