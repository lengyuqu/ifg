//! Integration tests: the real `ifg` binary against a wiremock IronForge.
//! Each test gets its own `XDG_CONFIG_HOME` so config writes stay isolated.

use assert_cmd::Command;
use serde_json::json;
use tempfile::TempDir;
use wiremock::matchers::{body_json, header, method, path, query_param};
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

/// `repo create` prints the created repository (table and --json forms).
#[tokio::test]
async fn repo_create_table_and_json() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/repos"))
        .and(header("authorization", "Bearer ifp_env_token"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": 7, "owner_id": 1, "name": "demo", "description": null,
            "is_private": false, "default_branch": "main", "stars_count": 0,
            "forks_count": 0, "created_at": "2026-11-03T10:00:00Z",
            "updated_at": "2026-11-03T10:00:00Z"
        })))
        .expect(2)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let env = |home: &TempDir| {
        let mut cmd = ifg();
        cmd.env("IRONFORGE_URL", server.uri())
            .env("IRONFORGE_PAT", "ifp_env_token")
            .env("XDG_CONFIG_HOME", home.path());
        cmd
    };

    let out = env(&home)
        .args(["repo", "create", "demo"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("Created"), "table: {stdout}");

    let out = env(&home)
        .args(["repo", "create", "demo", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["name"], "demo");
    assert_eq!(value["id"], 7);
}

/// `issue list` renders the table and `--json` the raw array.
#[tokio::test]
async fn issue_list_table_and_json() {
    let issue = json!({
        "id": 1, "repo_id": 1, "number": 1, "title": "broken build",
        "body": null, "state": "open", "author_id": 1, "labels": "[\"bug\"]",
        "author": "alice", "assignees": [],
        "created_at": "2026-11-03T10:00:00Z", "updated_at": "2026-11-03T10:00:00Z",
        "closed_at": null
    });
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/issues"))
        .and(query_param("state", "open"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [issue],
            "pagination": {"page": 1, "per_page": 100, "total": 1,
                           "total_pages": 1, "has_next": false, "has_prev": false}
        })))
        .expect(2)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let run = |json: bool| {
        let mut cmd = ifg();
        cmd.env("IRONFORGE_URL", server.uri())
            .env("IRONFORGE_PAT", "ifp_env_token")
            .env("XDG_CONFIG_HOME", home.path())
            .args(["-R", "alice/demo", "issue", "list"]);
        if json {
            cmd.arg("--json");
        }
        cmd
    };

    let out = run(false).assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("broken build"), "table: {stdout}");
    assert!(stdout.contains("bug"), "labels decoded: {stdout}");

    let out = run(true).assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    // D5: labels stays the server's raw JSON string, not the decoded list.
    assert_eq!(items[0]["labels"], json!("[\"bug\"]"));
}

/// `pr merge --squash` prints the merge result.
#[tokio::test]
async fn pr_merge_reports_the_merge_commit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/alice/demo/pulls/1/merge"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "merge_commit_sha": "f891045e1cb0d07121d6f0651131ef87566d20f6",
            "strategy": "squash"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let out = ifg()
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_env_token")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["-R", "alice/demo", "pr", "merge", "1", "--squash"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("Merged"), "stdout: {stdout}");
    assert!(stdout.contains("f891045e1cb0"), "merge commit: {stdout}");
}

/// `pr create` refuses to run when the branch has no upstream.
#[tokio::test]
async fn pr_create_requires_pushed_branch() {
    let home = TempDir::new().unwrap();
    // IFG_E2E-style URL that resolves nowhere; the failure must happen
    // before any request — locally, in the push check.
    let out = ifg()
        .env("IRONFORGE_URL", "http://127.0.0.1:9")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["-R", "alice/demo", "pr", "create", "-t", "t"])
        .current_dir("/tmp")
        .assert()
        .failure();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("no repository context") || stderr.contains("not inside a git work tree"),
        "stderr: {stderr}"
    );
}

fn write_config(home: &TempDir, hosts_toml: &str) {
    let dir = home.path().join("ironforge");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("hosts.toml"), hosts_toml).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            dir.join("hosts.toml"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
}

/// `auth git-credential get` answers for a configured host, stays silent
/// otherwise (the PAT must never leak to another site).
#[tokio::test]
async fn git_credential_answers_only_for_configured_hosts() {
    let home = TempDir::new().unwrap();
    write_config(
        &home,
        r#"
[hosts."forge.example.com"]
url = "https://forge.example.com"
user = "alice"
token = "ifp_secret_token"
"#,
    );

    // Matching host: full credential answer.
    let out = ifg()
        .args(["auth", "git-credential", "get"])
        .env("XDG_CONFIG_HOME", home.path())
        .write_stdin("protocol=https\nhost=forge.example.com\npath=git/alice/demo.git\n\n")
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("protocol=https"), "stdout: {stdout}");
    assert!(
        stdout.contains("host=forge.example.com"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("username=alice"), "stdout: {stdout}");
    assert!(
        stdout.contains("password=ifp_secret_token"),
        "stdout: {stdout}"
    );

    // Different host: silence.
    let out = ifg()
        .args(["auth", "git-credential", "get"])
        .env("XDG_CONFIG_HOME", home.path())
        .write_stdin("protocol=https\nhost=evil.example.com\n\n")
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.is_empty(),
        "credentials leaked to another host: {stdout}"
    );

    // store/erase: no-op, no output.
    let out = ifg()
        .args(["auth", "git-credential", "store"])
        .env("XDG_CONFIG_HOME", home.path())
        .write_stdin("protocol=https\nhost=forge.example.com\n\n")
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.is_empty(), "stdout: {stdout}");
}

/// `ci logs` prints a job log without polling.
#[tokio::test]
async fn ci_logs_prints_the_job_log() {
    let server = MockServer::start().await;
    let detail = json!({
        "pipeline": {"id": 1, "repo_id": 1, "commit_sha": "abc123", "ref_name": "main",
                     "status": "running", "trigger_type": "push", "triggered_by": "alice",
                     "started_at": "2026-11-03T10:00:00Z", "finished_at": null,
                     "created_at": "2026-11-03T10:00:00Z"},
        "stages": [{"stage": {"id": 1, "pipeline_id": 1, "name": "build", "stage_order": 1,
                              "status": "running", "started_at": null, "finished_at": null},
                    "jobs": [{"id": 9, "stage_id": 1, "name": "compile", "status": "running",
                              "log": "compiling...\ndone 42%\n", "exit_code": null}]}]
    });
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/pipelines/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(detail))
        .expect(1)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let out = ifg()
        .env("IRONFORGE_URL", server.uri())
        .env("XDG_CONFIG_HOME", home.path())
        .args(["-R", "alice/demo", "ci", "logs", "1"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("compiling..."), "stdout: {stdout}");
}

/// `release download` verifies the SHA-256 and deletes the partial file on
/// mismatch.
#[tokio::test]
async fn release_download_verifies_checksum() {
    use sha2::{Digest, Sha256};

    let content = b"release payload bytes";
    let checksum = format!("{:x}", Sha256::digest(content));

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/releases"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id": 1, "repo_id": 1, "tag_name": "v1", "target_commitish": "main",
                      "title": "v1", "body": null, "is_draft": false, "is_prerelease": false,
                      "author_id": 1, "created_at": "2026-11-03T10:00:00Z",
                      "updated_at": "2026-11-03T10:00:00Z"}],
            "pagination": {"page": 1, "per_page": 100, "total": 1, "total_pages": 1,
                           "has_next": false, "has_prev": false}
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/releases/1/assets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "id": 5, "release_id": 1, "filename": "pkg.bin", "size": content.len() as i64,
            "content_type": "application/octet-stream", "download_count": 0,
            "uploader_id": 1, "sha256": checksum, "created_at": "2026-11-03T10:00:00Z"
        }])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/releases/assets/5/download"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(content.to_vec())
                .insert_header("x-checksum-sha256", checksum.as_str()),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/releases/assets/6/download"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(b"tampered".to_vec())
                .insert_header("x-checksum-sha256", checksum.as_str()),
        )
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let download_dir = TempDir::new().unwrap();

    // Good checksum: file lands with the asset name.
    ifg()
        .env("IRONFORGE_URL", server.uri())
        .env("XDG_CONFIG_HOME", home.path())
        .current_dir(download_dir.path())
        .args(["-R", "alice/demo", "release", "download", "v1"])
        .assert()
        .success();
    let downloaded = std::fs::read(download_dir.path().join("pkg.bin")).unwrap();
    assert_eq!(downloaded, content);
}

/// A tampered download is detected: exit 1, partial file deleted.
#[tokio::test]
async fn release_download_rejects_a_tampered_asset() {
    use sha2::{Digest, Sha256};

    let content = b"release payload bytes";
    let checksum = format!("{:x}", Sha256::digest(content));

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/releases"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id": 1, "repo_id": 1, "tag_name": "v1", "target_commitish": "main",
                      "title": "v1", "body": null, "is_draft": false, "is_prerelease": false,
                      "author_id": 1, "created_at": "2026-11-03T10:00:00Z",
                      "updated_at": "2026-11-03T10:00:00Z"}],
            "pagination": {"page": 1, "per_page": 100, "total": 1, "total_pages": 1,
                           "has_next": false, "has_prev": false}
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/releases/1/assets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "id": 6, "release_id": 1, "filename": "pkg.bin", "size": 8,
            "content_type": "application/octet-stream", "download_count": 0,
            "uploader_id": 1, "sha256": checksum, "created_at": "2026-11-03T10:00:00Z"
        }])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/releases/assets/6/download"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(b"tampered".to_vec())
                .insert_header("x-checksum-sha256", checksum.as_str()),
        )
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let download_dir = TempDir::new().unwrap();
    let out = ifg()
        .env("IRONFORGE_URL", server.uri())
        .env("XDG_CONFIG_HOME", home.path())
        .current_dir(download_dir.path())
        .args(["-R", "alice/demo", "release", "download", "v1"])
        .assert()
        .failure();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("SHA-256 mismatch"), "stderr: {stderr}");
    assert!(!download_dir.path().join("pkg.bin").exists());
    assert!(!download_dir.path().join("pkg.bin.part").exists());
}

/// `ci logs --follow` on a job waiting for approval stops right away with a
/// hint instead of polling forever.
#[tokio::test]
async fn ci_logs_follow_stops_on_a_blocked_job() {
    let server = MockServer::start().await;
    let detail = json!({
        "pipeline": {"id": 1, "repo_id": 1, "commit_sha": "abc123", "ref_name": "main",
                     "status": "running", "trigger_type": "push", "triggered_by": "alice",
                     "started_at": "2026-11-03T10:00:00Z", "finished_at": null,
                     "created_at": "2026-11-03T10:00:00Z"},
        "stages": [{"stage": {"id": 1, "pipeline_id": 1, "name": "deploy", "stage_order": 1,
                              "status": "waiting_approval", "started_at": null, "finished_at": null},
                    "jobs": [{"id": 9, "stage_id": 1, "name": "prod", "status": "waiting_approval",
                              "log": null, "exit_code": null}]}]
    });
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/pipelines/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(detail))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/pipelines/1/jobs/9"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let out = ifg()
        .env("IRONFORGE_URL", server.uri())
        .env("XDG_CONFIG_HOME", home.path())
        .args(["-R", "alice/demo", "ci", "logs", "1", "--follow"])
        .timeout(std::time::Duration::from_secs(10))
        .assert()
        .success();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("ifg ci approve 1 9"), "stderr: {stderr}");
}

/// Mount a release `v1` with one asset named `filename` (id 7) whose
/// download returns `content`.
async fn mount_single_asset_release(server: &MockServer, filename: &str, content: &[u8]) {
    use sha2::{Digest, Sha256};
    let checksum = format!("{:x}", Sha256::digest(content));
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/releases"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id": 1, "repo_id": 1, "tag_name": "v1", "target_commitish": "main",
                      "title": "v1", "body": null, "is_draft": false, "is_prerelease": false,
                      "author_id": 1, "created_at": "2026-11-03T10:00:00Z",
                      "updated_at": "2026-11-03T10:00:00Z"}],
            "pagination": {"page": 1, "per_page": 100, "total": 1, "total_pages": 1,
                           "has_next": false, "has_prev": false}
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/releases/1/assets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "id": 7, "release_id": 1, "filename": filename, "size": content.len() as i64,
            "content_type": "application/octet-stream", "download_count": 0,
            "uploader_id": 1, "sha256": checksum, "created_at": "2026-11-03T10:00:00Z"
        }])))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/releases/assets/7/download"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(content.to_vec())
                .insert_header("x-checksum-sha256", checksum.as_str()),
        )
        .mount(server)
        .await;
}

/// A server-supplied asset name with path components is refused before any
/// byte is downloaded, so nothing lands outside the current directory.
#[tokio::test]
async fn release_download_refuses_path_traversal_names() {
    let server = MockServer::start().await;
    mount_single_asset_release(&server, "../escaped.txt", b"payload").await;

    let home = TempDir::new().unwrap();
    let parent = TempDir::new().unwrap();
    let download_dir = parent.path().join("downloads");
    std::fs::create_dir(&download_dir).unwrap();
    let out = ifg()
        .env("IRONFORGE_URL", server.uri())
        .env("XDG_CONFIG_HOME", home.path())
        .current_dir(&download_dir)
        .args(["-R", "alice/demo", "release", "download", "v1"])
        .assert()
        .failure();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("unsafe filename"), "stderr: {stderr}");
    assert!(!parent.path().join("escaped.txt").exists());
    assert!(!parent.path().join("escaped.txt.part").exists());
}

/// Existing files are kept unless `--clobber` is passed.
#[tokio::test]
async fn release_download_needs_clobber_to_overwrite() {
    let server = MockServer::start().await;
    mount_single_asset_release(&server, "pkg.bin", b"new bytes").await;

    let home = TempDir::new().unwrap();
    let download_dir = TempDir::new().unwrap();
    let target = download_dir.path().join("pkg.bin");
    std::fs::write(&target, b"old bytes").unwrap();

    let out = ifg()
        .env("IRONFORGE_URL", server.uri())
        .env("XDG_CONFIG_HOME", home.path())
        .current_dir(download_dir.path())
        .args(["-R", "alice/demo", "release", "download", "v1"])
        .assert()
        .failure();
    let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("--clobber"), "stderr: {stderr}");
    assert_eq!(std::fs::read(&target).unwrap(), b"old bytes");

    ifg()
        .env("IRONFORGE_URL", server.uri())
        .env("XDG_CONFIG_HOME", home.path())
        .current_dir(download_dir.path())
        .args(["-R", "alice/demo", "release", "download", "v1", "--clobber"])
        .assert()
        .success();
    assert_eq!(std::fs::read(&target).unwrap(), b"new bytes");
}

#[tokio::test]
async fn search_table_and_template() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/search"))
        .and(query_param("q", "fix bug"))
        .and(query_param("type", "all"))
        .and(query_param("page", "1"))
        .and(query_param("per_page", "30"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [
                {"result_type": "repo", "id": 1, "title": "alice/demo",
                 "repo_owner": "alice", "repo_name": "demo"},
                {"result_type": "issue", "id": 2, "title": "fix the bug",
                 "repo_owner": "alice", "repo_name": "demo", "state": "open", "number": 7}
            ],
            "total": 2, "page": 1, "per_page": 30
        })))
        .expect(2)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let mut table = ifg();
    let output = table
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_env_token")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["search", "fix bug"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("repo"), "table: {text}");
    assert!(text.contains("fix the bug"), "table: {text}");

    let mut templated = ifg();
    let output = templated
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_env_token")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["search", "fix bug", "-q", "{{.result_type}} {{.title}}"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("repo alice/demo"), "template: {text}");
    assert!(text.contains("issue fix the bug"), "template: {text}");
}

#[tokio::test]
async fn notifications_list_and_count() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/notifications"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 3, "user_id": 1, "event_type": "pipeline", "title": "build failed",
             "body": null, "repo_id": 1, "is_read": false, "created_at": "2026-11-03T10:00:00Z"}
        ])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/notifications/unread-count"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"unread_count": 1})))
        .expect(2)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let mut cmd = ifg();
    let output = cmd
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_env_token")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["notifications"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("build failed"), "list: {text}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("1 unread"),
        "count hint"
    );

    let mut count = ifg();
    let output = count
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_env_token")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["notifications", "count"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "1");
}

#[tokio::test]
async fn org_list_members_and_add() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/orgs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 4, "name": "acme", "display_name": "Acme Corp", "description": null,
             "owner_id": 1, "visibility": "public",
             "created_at": "2026-11-01T00:00:00Z", "updated_at": "2026-11-01T00:00:00Z"}
        ])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/orgs/acme/members"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 1, "org_id": 4, "user_id": 1, "username": "alice",
             "display_name": "Alice", "role": "owner", "created_at": "2026-11-01T00:00:00Z"},
            {"id": 2, "org_id": 4, "user_id": 7, "role": "member",
             "created_at": "2026-11-02T00:00:00Z"}
        ])))
        .expect(1)
        .mount(&server)
        .await;
    // --add-member resolves the username through /users/{username}
    Mock::given(method("GET"))
        .and(path("/api/v1/users/bob"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "username": "bob", "display_name": null, "avatar_url": null,
            "created_at": "2026-11-01T00:00:00Z"
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/orgs/acme/members"))
        .and(body_json(json!({"user_id": 9, "role": "admin"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;

    let home = TempDir::new().unwrap();
    let run = |args: &[&str]| {
        let mut cmd = ifg();
        cmd.env("IRONFORGE_URL", server.uri())
            .env("IRONFORGE_PAT", "ifp_env_token")
            .env("XDG_CONFIG_HOME", home.path())
            .args(args)
            .output()
            .unwrap()
    };

    let output = run(&["org", "list"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("acme"));

    let output = run(&["org", "members", "acme"]);
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("alice"), "resolved username: {text}");
    // A member the server could not resolve falls back to the numeric id.
    assert!(text.contains("#7"), "fallback label: {text}");

    let output = run(&["org", "add-member", "acme", "bob", "--role", "admin"]);
    assert!(
        output.status.success(),
        "add-member: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains("Added bob to acme as admin"),
        "add-member: {text}"
    );
}

#[test]
fn help_carries_examples_and_long_descriptions() {
    let top = ifg().arg("--help").output().unwrap();
    assert!(top.status.success());
    let text = String::from_utf8_lossy(&top.stdout);
    assert!(text.contains("EXAMPLES:"), "examples block: {text}");
    assert!(text.contains("ifg pr checkout 12"), "example line: {text}");
    assert!(text.contains("man ifg"), "man page hint: {text}");

    // `ifg help <group>` shows the long description, `-h` the short one.
    let long = ifg().args(["help", "pr"]).output().unwrap();
    let text = String::from_utf8_lossy(&long.stdout);
    assert!(text.contains("pr-<number>"), "long about: {text}");
    assert!(
        text.contains("refs/pull/<n>/head"),
        "long about detail: {text}"
    );

    let short = ifg().args(["pr", "-h"]).output().unwrap();
    let text = String::from_utf8_lossy(&short.stdout);
    assert!(
        !text.contains("refs/pull/<n>/head"),
        "short help stays short"
    );
}
