use assert_cmd::Command;
use serde_json::json;
use tempfile::TempDir;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

#[tokio::test]
async fn version_reports_provenance_and_unknown_fallback() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"version":"0.2.0"})))
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/api/v1/collaboration/capabilities")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"build_commit":"1234567890abcdef1234567890abcdef12345678", "build_dirty":true, "api_capabilities":["wiki.concurrent_update.v1"]}))).expect(1).mount(&server).await;
    let result = Command::cargo_bin("ifg")
        .unwrap()
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .arg("version")
        .assert()
        .success();
    let out = String::from_utf8_lossy(&result.get_output().stdout);
    assert!(out.contains("1234567890abcdef1234567890abcdef12345678 (modified source)"));
    assert!(out.contains("wiki.concurrent_update.v1"));
}

#[tokio::test]
async fn version_uses_repo_scoped_discovery_when_global_scope_is_denied() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"version":"0.2.0"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/collaboration/capabilities"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(json!({"error":{"code":"FORBIDDEN","message":"denied"}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/api/v1/repos/a/r/collaboration/capabilities")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"build_commit":"1234567890abcdef1234567890abcdef12345678","build_dirty":false,"api_capabilities":["wiki.concurrent_update.v1"]}))).expect(1).mount(&server).await;
    let result = Command::cargo_bin("ifg")
        .unwrap()
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["-R", "a/r", "version"])
        .assert()
        .success();
    let out = String::from_utf8_lossy(&result.get_output().stdout);
    assert!(out.contains("1234567890abcdef1234567890abcdef12345678"));
    assert!(out.contains("wiki.concurrent_update.v1"));
}
