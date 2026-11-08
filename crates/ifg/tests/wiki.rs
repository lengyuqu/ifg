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
        .args(["-R", "a/r", "wiki"]);
    cmd
}

#[tokio::test]
async fn wiki_encodes_titles_and_reads_content_from_stdin() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/a/r/wiki"))
        .and(body_json(
            json!({"title":"Hello world", "content":"content", "message":null}),
        ))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({"title":"Hello world","content":"content"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/wiki/Hello%20world"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"title":"Hello world","content":"content"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    command(&server, &home)
        .args(["create", "Hello world", "--file", "-"])
        .write_stdin("content")
        .assert()
        .success();
    let out = command(&server, &home)
        .args(["view", "Hello world"])
        .assert()
        .success();
    assert_eq!(
        String::from_utf8_lossy(&out.get_output().stdout),
        "content\n"
    );
}

#[tokio::test]
async fn wiki_delete_requires_confirmation_and_body_arguments_are_exclusive() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("DELETE"))
        .and(path("/api/v1/repos/a/r/wiki/demo"))
        .respond_with(ResponseTemplate::new(204))
        .expect(0)
        .mount(&server)
        .await;
    command(&server, &home)
        .args(["delete", "demo"])
        .assert()
        .failure();
    command(&server, &home)
        .args(["create", "demo", "--body", "x", "--file", "-"])
        .assert()
        .code(2);
}

#[tokio::test]
async fn wiki_edit_sends_the_observed_version_and_rejects_old_servers() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/collaboration/capabilities"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"api_capabilities":["wiki.concurrent_update.v1"]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let url = "/api/v1/repos/a/r/wiki/editable";
    Mock::given(method("GET"))
        .and(path(url))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"content":"old", "version":"10:21"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(url))
        .and(body_json(
            json!({"content":"new", "message":null, "expected_version":"10:21"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"content":"new"})))
        .expect(1)
        .mount(&server)
        .await;
    command(&server, &home)
        .args(["edit", "editable", "--body", "new"])
        .assert()
        .success();

    let old = "/api/v1/repos/a/r/wiki/old";
    Mock::given(method("GET"))
        .and(path(old))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"content":"old"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(old))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let result = command(&server, &home)
        .args(["edit", "old", "--body", "new"])
        .assert()
        .failure();
    assert!(String::from_utf8_lossy(&result.get_output().stderr)
        .contains("refusing an unprotected edit"));
}

#[tokio::test]
async fn explicit_version_cannot_bypass_missing_server_capability() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/collaboration/capabilities"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"capabilities":[]})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/api/v1/repos/a/r/wiki/old"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let result = command(&server, &home)
        .args(["edit", "old", "--body", "new", "--expected-version", "1:0"])
        .assert()
        .failure();
    assert!(String::from_utf8_lossy(&result.get_output().stderr)
        .contains("refusing an unprotected edit"));
}
