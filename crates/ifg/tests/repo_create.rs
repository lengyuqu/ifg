//! `ifg repo create`: a personal repository belongs to the account, not to
//! the host name. The old fallback printed `Created forge.example.com/demo`.

use assert_cmd::Command;
use serde_json::json;
use tempfile::TempDir;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

fn create_response() -> ResponseTemplate {
    ResponseTemplate::new(201).set_body_json(json!({
        "id": 1, "owner_id": 1, "name": "demo", "description": null,
        "is_private": true, "default_branch": "main", "settings_version": null,
        "stars_count": 0, "forks_count": 0,
        "created_at": "2026-11-09T10:00:00Z", "updated_at": null
    }))
}

fn profile() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "id": 1, "username": "alice", "email": null, "display_name": null,
        "avatar_url": null, "is_admin": false, "created_at": "2026-01-01T00:00:00Z"
    }))
}

/// A config with a cached login name for `server`.
fn config_with_user(server: &MockServer, user: &str) -> TempDir {
    let home = TempDir::new().unwrap();
    let dir = home.path().join("ironforge");
    std::fs::create_dir_all(&dir).unwrap();
    let key = server.uri().trim_start_matches("http://").to_string();
    let text = format!(
        "default_host = \"{key}\"\n\n[hosts.\"{key}\"]\nurl = \"{}\"\nuser = \"{user}\"\ntoken = \"ifp_test\"\n",
        server.uri()
    );
    std::fs::write(dir.join("hosts.toml"), text).unwrap();
    home
}

fn ifg(home: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("XDG_CONFIG_HOME", home.path())
        .env_remove("IRONFORGE_URL")
        .env_remove("IRONFORGE_PAT")
        .args(["repo", "create", "demo", "--private"]);
    cmd
}

#[tokio::test]
async fn personal_repository_uses_the_cached_account() {
    let server = MockServer::start().await;
    let home = config_with_user(&server, "alice");
    Mock::given(method("POST"))
        .and(path("/api/v1/repos"))
        .respond_with(create_response())
        .expect(1)
        .mount(&server)
        .await;
    // The cached name must be used instead of asking the server again.
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .respond_with(profile())
        .expect(0)
        .mount(&server)
        .await;

    let output = ifg(&home).assert().success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).into_owned();
    assert!(stdout.contains("Created alice/demo"), "stdout: {stdout}");
    assert!(
        stdout.contains(&format!("{}/git/alice/demo.git", server.uri())),
        "stdout: {stdout}"
    );
    assert!(
        !stdout.contains("Created 127.0.0.1"),
        "host name leaked into the owner: {stdout}"
    );
}

#[tokio::test]
async fn organization_repository_keeps_the_org_name() {
    let server = MockServer::start().await;
    let home = config_with_user(&server, "alice");
    Mock::given(method("POST"))
        .and(path("/api/v1/repos"))
        .respond_with(create_response())
        .expect(1)
        .mount(&server)
        .await;

    let output = ifg(&home).args(["--org", "acme"]).assert().success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).into_owned();
    assert!(stdout.contains("Created acme/demo"), "stdout: {stdout}");
    assert!(
        stdout.contains(&format!("{}/git/acme/demo.git", server.uri())),
        "stdout: {stdout}"
    );
}

/// No config at all (`IRONFORGE_URL` + `IRONFORGE_PAT`): the account name has
/// to come from `/users/me`.
#[tokio::test]
async fn env_only_auth_resolves_the_account_from_the_api() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .respond_with(profile())
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/repos"))
        .respond_with(create_response())
        .expect(1)
        .mount(&server)
        .await;

    let mut cmd = Command::cargo_bin("ifg").unwrap();
    let output = cmd
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["repo", "create", "demo", "--private"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).into_owned();
    assert!(stdout.contains("Created alice/demo"), "stdout: {stdout}");
}

/// A token with `repo` but not `user` scope: `GET /users/me` answers 403 and
/// the create must still succeed. The old code called `/users/me` first and
/// aborted, even though `--json` worked with the same token.
#[tokio::test]
async fn forbidden_profile_does_not_block_the_create() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "error": {"code": "FORBIDDEN", "message": "missing user scope"}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/repos"))
        .respond_with(create_response())
        .expect(1)
        .mount(&server)
        .await;

    let output = Command::cargo_bin("ifg")
        .unwrap()
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_repo_only")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["repo", "create", "demo", "--private"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr).into_owned();
    assert!(stdout.contains("Created demo"), "stdout: {stdout}");
    assert!(
        !stdout.contains("Created 127.0.0.1"),
        "host name must not become the owner: {stdout}"
    );
    assert!(
        stdout.contains("/git/OWNER/demo.git"),
        "the remote example must not invent a name: {stdout}"
    );
    assert!(
        stderr.contains("could not read the account name"),
        "stderr: {stderr}"
    );
}

/// `IRONFORGE_PAT` can carry a different account than the one cached for the
/// host; the cached name must not be reported for the new token.
#[tokio::test]
async fn env_token_overrides_the_cached_user_name() {
    let server = MockServer::start().await;
    let home = config_with_user(&server, "alice");
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 2, "username": "bob", "email": null, "display_name": null,
            "avatar_url": null, "is_admin": false,
            "created_at": "2026-01-01T00:00:00Z"
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/repos"))
        .respond_with(create_response())
        .expect(1)
        .mount(&server)
        .await;

    let output = Command::cargo_bin("ifg")
        .unwrap()
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_bob")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["repo", "create", "demo", "--private"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).into_owned();
    assert!(stdout.contains("Created bob/demo"), "stdout: {stdout}");
    assert!(
        !stdout.contains("alice"),
        "cached name leaked with a different token: {stdout}"
    );
}
