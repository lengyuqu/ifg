//! Integration tests: the real `ifg` binary against a wiremock IronForge.
//! Each test gets its own `XDG_CONFIG_HOME` so config writes stay isolated.

use assert_cmd::Command;
use serde_json::json;
use tempfile::TempDir;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ifg() -> Command {
    Command::cargo_bin("ifg").unwrap()
}

fn me_body() -> serde_json::Value {
    json!({"id": 1, "username": "alice", "email": null, "display_name": null,
           "avatar_url": null, "is_admin": false, "created_at": "2026-01-01T00:00:00Z"})
}

fn error_body(code: &str, message: &str) -> serde_json::Value {
    json!({"error": {"code": code, "message": message, "request_id": null}})
}

fn page(items: serde_json::Value, page: u64, has_next: bool) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "data": items,
        "pagination": {
            "page": page,
            "per_page": 100,
            "total": 3,
            "total_pages": if has_next { page + 1 } else { page },
            "has_next": has_next,
            "has_prev": page > 1,
        }
    }))
}

#[test]
fn version_prints_crate_version() {
    let out = ifg().arg("--version").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert_eq!(stdout.trim(), concat!("ifg ", env!("CARGO_PKG_VERSION")));
}

#[test]
fn no_arguments_shows_help_with_usage_exit_code() {
    let out = ifg().assert().code(2);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("Usage: ifg"), "stderr was: {stderr}");
}

/// Login with a token piped from stdin, against a mock `/users/me`.
#[tokio::test]
async fn auth_login_with_token_saves_config() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .and(header("authorization", "Bearer ifp_login_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(me_body()))
        .expect(1)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let out = ifg()
        .arg("auth")
        .arg("login")
        .arg("--with-token")
        .env("IRONFORGE_URL", server.uri())
        .env("XDG_CONFIG_HOME", home.path())
        .write_stdin("ifp_login_token\n")
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("Logged in"), "stdout: {stdout}");
    assert!(stdout.contains("alice"), "stdout: {stdout}");

    let config_path = home.path().join("ironforge/hosts.toml");
    let text = std::fs::read_to_string(&config_path).unwrap();
    assert!(text.contains("ifp_login_token"), "config: {text}");
    assert!(text.contains("alice"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&config_path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "config must be 0600");
    }
}

/// `ifg api users/me` passes the server JSON through untouched.
#[tokio::test]
async fn api_prints_server_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .and(header("authorization", "Bearer ifp_env_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(me_body()))
        .expect(1)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let out = ifg()
        .arg("api")
        .arg("users/me")
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_env_token")
        .env("XDG_CONFIG_HOME", home.path())
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["username"], "alice");
    // D5: the server's own field names, no re-shaping.
    assert!(parsed.get("created_at").is_some());
}

#[tokio::test]
async fn api_fields_become_query_params_on_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos"))
        .and(query_param("state", "open"))
        .and(query_param("page", "2"))
        .respond_with(page(json!([{"id": 1}]), 2, false))
        .expect(1)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    ifg()
        .arg("api")
        .arg("repos")
        .arg("-f")
        .arg("state=open")
        .arg("-f")
        .arg("page=2")
        .env("IRONFORGE_URL", server.uri())
        .env("XDG_CONFIG_HOME", home.path())
        .assert()
        .success();
}

#[tokio::test]
async fn api_paginates_and_merges_data() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos"))
        .and(query_param("page", "1"))
        .respond_with(page(json!([{"id": 1}, {"id": 2}]), 1, true))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos"))
        .and(query_param("page", "2"))
        .respond_with(page(json!([{"id": 3}]), 2, false))
        .expect(1)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let out = ifg()
        .arg("api")
        .arg("repos")
        .arg("--paginate")
        .env("IRONFORGE_URL", server.uri())
        .env("XDG_CONFIG_HOME", home.path())
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let merged: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(merged.len(), 3);
}

#[tokio::test]
async fn api_not_found_exits_8() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/ghost"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_json(error_body("NOT_FOUND", "repository not found")),
        )
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let out = ifg()
        .arg("api")
        .arg("repos/ghost")
        .env("IRONFORGE_URL", server.uri())
        .env("XDG_CONFIG_HOME", home.path())
        .assert()
        .code(8);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("repository not found"), "stderr: {stderr}");
}

#[tokio::test]
async fn api_unauthorized_exits_4() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(error_body("UNAUTHORIZED", "missing or invalid token")),
        )
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    ifg()
        .arg("api")
        .arg("users/me")
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_bad")
        .env("XDG_CONFIG_HOME", home.path())
        .assert()
        .code(4);
}

#[tokio::test]
async fn api_paginate_on_post_is_a_usage_error() {
    let home = TempDir::new().unwrap();
    let out = ifg()
        .arg("api")
        .arg("search")
        .arg("-X")
        .arg("POST")
        .arg("--paginate")
        .env("IRONFORGE_URL", "http://127.0.0.1:9")
        .env("XDG_CONFIG_HOME", home.path())
        .assert()
        .code(2);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("--paginate requires GET"),
        "stderr: {stderr}"
    );
}

#[tokio::test]
async fn auth_token_prints_the_env_token() {
    let home = TempDir::new().unwrap();
    let out = ifg()
        .arg("auth")
        .arg("token")
        .env("IRONFORGE_URL", "http://forge.invalid")
        .env("IRONFORGE_PAT", "ifp_env_token")
        .env("XDG_CONFIG_HOME", home.path())
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert_eq!(stdout.trim(), "ifp_env_token");
}

#[tokio::test]
async fn auth_token_without_token_exits_4() {
    let home = TempDir::new().unwrap();
    ifg()
        .arg("auth")
        .arg("token")
        .env("IRONFORGE_URL", "http://forge.invalid")
        .env("XDG_CONFIG_HOME", home.path())
        .assert()
        .code(4);
}

/// Logout removes the entry and re-points `default_host`.
#[tokio::test]
async fn auth_logout_removes_the_host() {
    let home = TempDir::new().unwrap();
    let config_dir = home.path().join("ironforge");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("hosts.toml"),
        r#"
default_host = "a.example.com"

[hosts."a.example.com"]
url = "https://a.example.com"
user = "alice"
token = "ifp_a"

[hosts."b.example.com"]
url = "https://b.example.com"
user = "bob"
token = "ifp_b"
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            config_dir.join("hosts.toml"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }

    let out = ifg()
        .arg("auth")
        .arg("logout")
        .arg("--host")
        .arg("a.example.com")
        .env("XDG_CONFIG_HOME", home.path())
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("a.example.com"), "stdout: {stdout}");

    let text = std::fs::read_to_string(config_dir.join("hosts.toml")).unwrap();
    assert!(!text.contains("ifp_a"), "token still present: {text}");
    assert!(text.contains("ifp_b"));
    assert!(
        text.contains("b.example.com"),
        "default_host not re-pointed: {text}"
    );
}

/// A config file with group/other read bits loses its tokens on load.
#[tokio::test]
async fn insecure_config_tokens_are_refused() {
    let home = TempDir::new().unwrap();
    let config_dir = home.path().join("ironforge");
    std::fs::create_dir_all(&config_dir).unwrap();
    let config_path = config_dir.join("hosts.toml");
    std::fs::write(
        &config_path,
        r#"
default_host = "a.example.com"

[hosts."a.example.com"]
url = "http://forge.invalid"
user = "alice"
token = "ifp_a"
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }

    // `auth token` resolves the host but must not hand out the token.
    let out = ifg()
        .arg("auth")
        .arg("token")
        .env("XDG_CONFIG_HOME", home.path())
        .assert()
        .code(4);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("chmod 600"), "stderr: {stderr}");
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(!stdout.contains("ifp_a"), "token leaked: {stdout}");
}

/// Retries keep the CLI usable behind a flaky reverse proxy, and are fast
/// enough for tests.
#[tokio::test]
async fn api_gets_retried_after_503() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(me_body()))
        .expect(1)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    ifg()
        .arg("api")
        .arg("users/me")
        .env("IRONFORGE_URL", server.uri())
        .env("XDG_CONFIG_HOME", home.path())
        .assert()
        .success();
}
