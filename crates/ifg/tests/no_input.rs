//! `--no-input` / `IFG_NO_INPUT=1`: every place that would ask a person must
//! fail at once with exit 2, name the flag to pass instead and write nothing
//! to the server. The switch never acts as `--yes`.

use std::time::Duration;

use assert_cmd::{assert::Assert, Command};
use serde_json::json;
use tempfile::TempDir;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

fn ifg(server: &MockServer, home: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .env_remove("IFG_NO_INPUT")
        .timeout(Duration::from_secs(20));
    cmd
}

/// `ifg <args>` with the switch on, as the flag or as `IFG_NO_INPUT=1`.
fn no_input(server: &MockServer, home: &TempDir, args: &[&str], via_env: bool) -> Assert {
    let mut cmd = ifg(server, home);
    if via_env {
        cmd.env("IFG_NO_INPUT", "1");
    } else {
        cmd.arg("--no-input");
    }
    cmd.args(args).assert()
}

/// Exit 2, a message that says why (`--no-input`), and a message that names
/// each argument the caller should pass instead.
fn assert_refused(assert: Assert, names: &[&str]) {
    let assert = assert.code(2);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(stderr.contains("--no-input"), "stderr: {stderr}");
    for name in names {
        assert!(stderr.contains(name), "stderr should name {name}: {stderr}");
    }
}

fn page(items: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "data": items,
        "pagination": {
            "page": 1, "per_page": 100, "total": 1, "total_pages": 1,
            "has_next": false, "has_prev": false,
        }
    }))
}

/// Each refusal runs as the flag and as the environment switch. The mocks
/// expect no write, and wiremock verifies that when the server is dropped.
#[tokio::test]
async fn repo_delete_refuses_without_yes_and_deletes_nothing() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("DELETE"))
        .and(path("/api/v1/repos/a/r"))
        .respond_with(ResponseTemplate::new(204))
        .expect(0)
        .mount(&server)
        .await;
    for via_env in [false, true] {
        assert_refused(
            no_input(&server, &home, &["repo", "delete", "a/r"], via_env),
            &["--yes"],
        );
    }
}

#[tokio::test]
async fn repo_delete_with_yes_still_deletes_under_no_input() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("DELETE"))
        .and(path("/api/v1/repos/a/r"))
        .respond_with(ResponseTemplate::new(204))
        .expect(2)
        .mount(&server)
        .await;
    for via_env in [false, true] {
        let out = no_input(&server, &home, &["repo", "delete", "a/r", "--yes"], via_env).success();
        let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
        assert!(stdout.contains("Deleted a/r."), "stdout: {stdout}");
    }
}

#[tokio::test]
async fn repo_collaborator_removal_refuses_without_yes() {
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
                .set_body_json(json!([{"id":987,"repo_id":2,"user_id":123,"permission":"write"}])),
        )
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/repos/a/r/collaborators/123"))
        .respond_with(ResponseTemplate::new(204))
        .expect(0)
        .mount(&server)
        .await;
    for via_env in [false, true] {
        assert_refused(
            no_input(
                &server,
                &home,
                &["-R", "a/r", "repo", "collaborators", "remove", "bob"],
                via_env,
            ),
            &["--yes"],
        );
    }
}

#[tokio::test]
async fn wiki_delete_refuses_without_yes() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("DELETE"))
        .and(path("/api/v1/repos/a/r/wiki/demo"))
        .respond_with(ResponseTemplate::new(204))
        .expect(0)
        .mount(&server)
        .await;
    for via_env in [false, true] {
        assert_refused(
            no_input(
                &server,
                &home,
                &["-R", "a/r", "wiki", "delete", "demo"],
                via_env,
            ),
            &["--yes"],
        );
    }
}

#[tokio::test]
async fn label_delete_refuses_without_yes() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/labels"))
        .respond_with(page(json!([{"id":5,"repo_id":2,"name":"bug"}])))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/repos/a/r/labels/5"))
        .respond_with(ResponseTemplate::new(204))
        .expect(0)
        .mount(&server)
        .await;
    for via_env in [false, true] {
        assert_refused(
            no_input(
                &server,
                &home,
                &["-R", "a/r", "label", "delete", "bug"],
                via_env,
            ),
            &["--yes"],
        );
    }
}

#[tokio::test]
async fn milestone_delete_refuses_without_yes() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/milestones"))
        .respond_with(page(json!([{"id":7,"repo_id":2,"title":"v1"}])))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/repos/a/r/milestones/7"))
        .respond_with(ResponseTemplate::new(204))
        .expect(0)
        .mount(&server)
        .await;
    for via_env in [false, true] {
        assert_refused(
            no_input(
                &server,
                &home,
                &["-R", "a/r", "milestone", "delete", "v1"],
                via_env,
            ),
            &["--yes"],
        );
    }
}

#[tokio::test]
async fn release_delete_refuses_without_yes() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/r/releases/tags/v1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "repo_id": 2, "tag_name": "v1", "target_commitish": "main",
            "title": "v1", "is_draft": false, "is_prerelease": false, "author_id": 1
        })))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/repos/a/r/releases/9"))
        .respond_with(ResponseTemplate::new(204))
        .expect(0)
        .mount(&server)
        .await;
    for via_env in [false, true] {
        assert_refused(
            no_input(
                &server,
                &home,
                &["-R", "a/r", "release", "delete", "v1"],
                via_env,
            ),
            &["--yes"],
        );
    }
}

#[tokio::test]
async fn auth_login_without_with_token_refuses_and_saves_nothing() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    for via_env in [false, true] {
        let mut cmd = ifg(&server, &home);
        if via_env {
            cmd.env("IFG_NO_INPUT", "1");
        } else {
            cmd.arg("--no-input");
        }
        // Piped stdin is the implicit read that is refused, not just a TTY prompt.
        let assert = cmd
            .args(["auth", "login"])
            .write_stdin("ifp_login_token\n")
            .assert();
        assert_refused(assert, &["--with-token"]);
    }
    assert!(!home.path().join("ironforge/hosts.toml").exists());
}

#[tokio::test]
async fn auth_login_with_token_still_reads_stdin_under_no_input() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 1, "username": "alice", "email": null, "display_name": null,
            "avatar_url": null, "is_admin": false, "created_at": "2026-01-01T00:00:00Z"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let out = ifg(&server, &home)
        .args(["--no-input", "auth", "login", "--with-token"])
        .write_stdin("ifp_login_token\n")
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(stdout.contains("Logged in"), "stdout: {stdout}");
}

#[cfg(unix)]
#[tokio::test]
async fn issue_create_editor_refuses_before_launching_the_editor() {
    use std::os::unix::fs::PermissionsExt;
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let scratch = TempDir::new().unwrap();
    let marker = scratch.path().join("editor-ran");
    let editor = scratch.path().join("fake-editor.sh");
    std::fs::write(
        &editor,
        format!(
            "#!/bin/sh\ntouch '{}'\nprintf 'T\\n' > \"$1\"\n",
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&editor, std::fs::Permissions::from_mode(0o755)).unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/alice/demo/issues"))
        .respond_with(ResponseTemplate::new(201))
        .expect(0)
        .mount(&server)
        .await;
    for via_env in [false, true] {
        let mut cmd = ifg(&server, &home);
        cmd.env("EDITOR", &editor);
        if via_env {
            cmd.env("IFG_NO_INPUT", "1");
        } else {
            cmd.arg("--no-input");
        }
        assert_refused(
            cmd.args(["-R", "alice/demo", "issue", "create", "--editor"])
                .assert(),
            &["--title", "--body"],
        );
    }
    assert!(!marker.exists(), "the editor must not be launched");
}

#[cfg(unix)]
#[tokio::test]
async fn pr_create_editor_refuses_before_launching_the_editor() {
    use std::os::unix::fs::PermissionsExt;
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let scratch = TempDir::new().unwrap();
    let marker = scratch.path().join("editor-ran");
    let editor = scratch.path().join("fake-editor.sh");
    std::fs::write(
        &editor,
        format!(
            "#!/bin/sh\ntouch '{}'\nprintf 'T\\n' > \"$1\"\n",
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&editor, std::fs::Permissions::from_mode(0o755)).unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/alice/demo/pulls"))
        .respond_with(ResponseTemplate::new(201))
        .expect(0)
        .mount(&server)
        .await;
    let mut cmd = ifg(&server, &home);
    cmd.env("EDITOR", &editor).arg("--no-input").args([
        "-R",
        "alice/demo",
        "pr",
        "create",
        "--editor",
    ]);
    assert_refused(cmd.assert(), &["--title", "--body"]);
    assert!(!marker.exists(), "the editor must not be launched");
}

/// A fake `git` that records the prompt settings it was started with. The
/// clone is the only call that matters; every other call reports no result.
#[cfg(unix)]
#[tokio::test]
async fn repo_clone_git_is_told_not_to_prompt_only_under_no_input() {
    use std::os::unix::fs::PermissionsExt;
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let bin = TempDir::new().unwrap();
    let record = bin.path().join("record");
    let git = bin.path().join("git");
    std::fs::write(
        &git,
        "#!/bin/sh\n\
         if [ \"$1\" = clone ]; then\n\
           printf 'prompt=%s askpass=%s\\n' \"${GIT_TERMINAL_PROMPT-unset}\" \"${GIT_ASKPASS-unset}\" > \"$IFG_TEST_RECORD\"\n\
           exit 0\n\
         fi\n\
         exit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        bin.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    for (switch_on, expected) in [
        (true, "prompt=0 askpass="),
        (false, "prompt=unset askpass=unset"),
    ] {
        let _ = std::fs::remove_file(&record);
        let mut cmd = ifg(&server, &home);
        cmd.env("PATH", &path)
            .env("IFG_TEST_RECORD", &record)
            .env_remove("GIT_TERMINAL_PROMPT")
            .env_remove("GIT_ASKPASS");
        if switch_on {
            cmd.arg("--no-input");
        }
        cmd.args(["repo", "clone", "a/r"]).assert().success();
        let seen = std::fs::read_to_string(&record).unwrap();
        assert_eq!(seen.trim(), expected, "switch_on={switch_on}");
    }
}
