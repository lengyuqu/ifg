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
