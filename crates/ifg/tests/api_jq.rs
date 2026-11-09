use assert_cmd::Command;
use serde_json::json;
use tempfile::TempDir;
use wiremock::{
    matchers::{method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};

fn command(server: &MockServer, home: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path());
    cmd
}

#[tokio::test]
async fn filters_json_and_reports_invalid_expression() {
    if !std::process::Command::new("jq")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("jq unavailable; real jq integration test skipped");
        return;
    }
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/example"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"items":[{"name":"alpha"},{"name":"beta"}]})),
        )
        .expect(2)
        .mount(&server)
        .await;
    for (page, name, has_next) in [(1, "alpha", true), (2, "beta", false)] {
        Mock::given(method("GET")).and(path("/api/v1/list"))
            .and(query_param("page", page.to_string()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"name":name}],"pagination":{"page":page,"per_page":100,"total":2,"total_pages":2,"has_next":has_next,"has_prev":page>1}})))
            .expect(1).mount(&server).await;
    }
    let result = command(&server, &home)
        .args([
            "api",
            "example",
            "--jq",
            ".items[] | select(.name == \"beta\") | .name",
            "--jq-raw",
        ])
        .assert()
        .success();
    assert_eq!(
        String::from_utf8_lossy(&result.get_output().stdout).replace("\r\n", "\n"),
        "beta\n"
    );
    let result = command(&server, &home)
        .args(["api", "list", "--paginate", "--jq", "map(.name)"])
        .assert()
        .success();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&result.get_output().stdout).unwrap(),
        json!(["alpha", "beta"])
    );
    command(&server, &home)
        .args(["api", "example", "--jq", ".[ "])
        .assert()
        .failure();
}

#[tokio::test]
async fn rejects_non_json_and_non_get_requests() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/example"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>not json</html>"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/example"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let result = command(&server, &home)
        .args(["api", "example", "--jq", "."])
        .assert()
        .failure();
    assert!(
        String::from_utf8_lossy(&result.get_output().stderr).contains("requires a JSON response")
    );
    command(&server, &home)
        .args(["api", "example", "-X", "POST", "--jq", "."])
        .assert()
        .code(2);
}

#[cfg(unix)]
#[tokio::test]
async fn reports_missing_jq_without_running_a_shell() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/example"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
        .expect(1)
        .mount(&server)
        .await;
    let result = command(&server, &home)
        .env("PATH", home.path())
        .args(["api", "example", "--jq", "."])
        .assert()
        .failure();
    assert!(String::from_utf8_lossy(&result.get_output().stderr).contains("could not start jq"));
}
