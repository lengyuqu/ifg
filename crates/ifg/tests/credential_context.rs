use assert_cmd::Command;
use serde_json::json;
use tempfile::TempDir;
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};

fn config(server: &MockServer, url: &str) -> TempDir {
    let home = TempDir::new().unwrap();
    let dir = home.path().join("ironforge");
    std::fs::create_dir(&dir).unwrap();
    let key = server.uri().trim_start_matches("http://").to_string();
    let file = dir.join("hosts.toml");
    std::fs::write(&file, format!("default_host='{key}'\n[hosts.'{key}']\nurl='{url}'\nuser='alice'\ntoken='ifp_stored'\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    home
}

fn command(home: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("XDG_CONFIG_HOME", home.path())
        .env_remove("IRONFORGE_URL")
        .env_remove("IRONFORGE_PAT");
    cmd
}

#[tokio::test]
async fn stored_pat_is_not_reused_across_protocol_or_deployment_boundaries() {
    for (https, base_path) in [(true, ""), (false, "/another-deployment")] {
        for from_env in [false, true] {
            let server = MockServer::start().await;
            let stored_url = if https {
                server.uri().replace("http://", "https://")
            } else {
                server.uri()
            };
            let home = config(&server, &stored_url);
            let target = format!("{}{base_path}", server.uri());
            Mock::given(path(format!("{base_path}/api/v1/users/me")))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
                .expect(1)
                .mount(&server)
                .await;
            let mut cmd = command(&home);
            if from_env {
                cmd.env("IRONFORGE_URL", target);
            } else {
                cmd.args(["--host", &target]);
            }
            cmd.args(["api", "users/me"]).assert().success();
            let requests = server.received_requests().await.unwrap();
            assert!(!requests[0].headers.contains_key("authorization"));
        }
    }
}

#[tokio::test]
async fn matching_origin_still_reuses_the_stored_pat() {
    let server = MockServer::start().await;
    let home = config(&server, &server.uri());
    Mock::given(path("/api/v1/users/me"))
        .and(header("authorization", "Bearer ifp_stored"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
        .expect(1)
        .mount(&server)
        .await;
    command(&home)
        .args(["--host", &server.uri(), "api", "users/me"])
        .assert()
        .success();
}

#[tokio::test]
async fn repo_list_uses_the_account_of_the_overriding_pat() {
    let server = MockServer::start().await;
    let home = config(&server, &server.uri());
    Mock::given(path("/api/v1/users/me"))
        .and(header("authorization", "Bearer ifp_bob"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":2,"username":"bob"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/api/v1/repos/bob"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/api/v1/repos/alice"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(0)
        .mount(&server)
        .await;
    let result = command(&home)
        .env("IRONFORGE_PAT", "ifp_bob")
        .args(["repo", "list", "--json"])
        .assert()
        .success();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&result.get_output().stdout).unwrap(),
        json!([])
    );
}

#[tokio::test]
async fn an_explicit_owner_does_not_require_user_scope() {
    let server = MockServer::start().await;
    let home = config(&server, &server.uri());
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .respond_with(ResponseTemplate::new(403))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(path("/api/v1/repos/acme"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&server)
        .await;
    command(&home)
        .env("IRONFORGE_PAT", "ifp_repo_only")
        .args(["repo", "list", "acme", "--json"])
        .assert()
        .success();
}
