//! `ifg pr create`: the "is my branch pushed?" guard must depend on where
//! `head` came from and must be bound to the remote that corresponds to the
//! target repository — not to any remote that happens to have the branch.

use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;

use assert_cmd::Command;
use serde_json::json;
use tempfile::TempDir;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

fn git(dir: &Path, args: &[&str]) {
    let output = StdCommand::new("git")
        .args(args)
        .current_dir(dir)
        .envs([
            ("GIT_AUTHOR_NAME", "ifg-test"),
            ("GIT_AUTHOR_EMAIL", "ifg-test@ifg.invalid"),
            ("GIT_COMMITTER_NAME", "ifg-test"),
            ("GIT_COMMITTER_EMAIL", "ifg-test@ifg.invalid"),
        ])
        .output()
        .expect("git is installed");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
}

fn ifg(server: &MockServer, home: &TempDir, dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .current_dir(dir)
        .args(["-R", "a/r", "pr", "create"]);
    cmd
}

fn created_pr() -> ResponseTemplate {
    ResponseTemplate::new(201).set_body_json(json!({
        "id": 1, "repo_id": 1, "number": 5, "title": "title", "body": null,
        "state": "open", "is_draft": false, "author_id": 1,
        "head_branch": "feature", "base_branch": "main",
        "head_sha": null, "merge_commit_sha": null, "head_repo_id": null,
        "created_at": "2026-11-09T10:00:00Z"
    }))
}

/// A checkout whose `origin` **looks like** `{server}/git/a/r.git` to the CLI
/// but pushes to a local bare repository via `url.<path>.insteadOf`, so pushes
/// are real without touching the mock server.
struct Repos {
    clone: PathBuf,
}

fn ironforge_clone(work: &Path, server: &MockServer) -> Repos {
    let origin = work.join("origin.git");
    let backup = work.join("backup.git");
    let clone = work.join("clone");
    std::fs::create_dir(&origin).unwrap();
    std::fs::create_dir(&backup).unwrap();
    git(&origin, &["init", "--bare", "-b", "main"]);
    git(&backup, &["init", "--bare", "-b", "main"]);
    git(
        work,
        &["clone", origin.to_str().unwrap(), clone.to_str().unwrap()],
    );
    git(&clone, &["checkout", "-b", "main"]);
    git(&clone, &["commit", "--allow-empty", "-m", "init"]);
    git(&clone, &["push", "-q", "origin", "main"]);

    // From here on the CLI sees an IronForge remote; git still reaches the
    // local bare repository.
    let forge_url = format!("{}/git/a/r.git", server.uri());
    git(&clone, &["remote", "set-url", "origin", &forge_url]);
    git(
        &clone,
        &[
            "config",
            &format!("url.{}.insteadOf", origin.display()),
            &forge_url,
        ],
    );
    git(
        &clone,
        &["remote", "add", "backup", backup.to_str().unwrap()],
    );

    Repos { clone }
}

fn feature_with_commit(repo: &Repos, message: &str) {
    git(&repo.clone, &["checkout", "-b", "feature"]);
    git(&repo.clone, &["commit", "--allow-empty", "-m", message]);
}

/// A branch with no upstream and a `-t` title: the old code sent the request
/// anyway.
#[tokio::test]
async fn explicit_title_does_not_bypass_the_unpushed_check() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let repos = ironforge_clone(work.path(), &server);
    feature_with_commit(&repos, "one");

    Mock::given(method("POST"))
        .and(path("/api/v1/repos/a/r/pulls"))
        .respond_with(created_pr())
        .expect(0)
        .mount(&server)
        .await;

    let assert = ifg(&server, &home, &repos.clone)
        .args(["-t", "title", "--base", "main"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(stderr.contains("not pushed"), "stderr: {stderr}");
}

/// Pushed to the right remote without `-u`: no upstream, but `origin/feature`
/// is HEAD, which is a normal workflow.
#[tokio::test]
async fn pushed_without_upstream_is_accepted() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let repos = ironforge_clone(work.path(), &server);
    feature_with_commit(&repos, "one");
    git(&repos.clone, &["push", "-q", "origin", "feature"]);

    Mock::given(method("POST"))
        .and(path("/api/v1/repos/a/r/pulls"))
        .respond_with(created_pr())
        .expect(1)
        .mount(&server)
        .await;

    ifg(&server, &home, &repos.clone)
        .args(["-t", "title", "--base", "main"])
        .assert()
        .success();
}

/// The new commit only went to `backup`; `origin/feature` still points at the
/// previous commit, so the target repository lacks it.
#[tokio::test]
async fn push_to_another_remote_does_not_count() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let repos = ironforge_clone(work.path(), &server);
    feature_with_commit(&repos, "one");
    git(&repos.clone, &["push", "-q", "origin", "feature"]);
    git(&repos.clone, &["commit", "--allow-empty", "-m", "two"]);
    git(&repos.clone, &["push", "-q", "backup", "feature"]);

    Mock::given(method("POST"))
        .and(path("/api/v1/repos/a/r/pulls"))
        .respond_with(created_pr())
        .expect(0)
        .mount(&server)
        .await;

    let assert = ifg(&server, &home, &repos.clone)
        .args(["-t", "title", "--base", "main"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        stderr.contains("not pushed") || stderr.contains("not a/r"),
        "stderr: {stderr}"
    );
}

/// An upstream on a non-target remote must be refused even when it is current.
#[tokio::test]
async fn upstream_on_another_remote_is_refused() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let repos = ironforge_clone(work.path(), &server);
    feature_with_commit(&repos, "one");
    git(&repos.clone, &["push", "-q", "backup", "feature"]);
    git(
        &repos.clone,
        &["branch", "--set-upstream-to", "backup/feature", "feature"],
    );

    Mock::given(method("POST"))
        .and(path("/api/v1/repos/a/r/pulls"))
        .respond_with(created_pr())
        .expect(0)
        .mount(&server)
        .await;

    let assert = ifg(&server, &home, &repos.clone)
        .args(["-t", "title", "--base", "main"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(stderr.contains("tracks backup/feature"), "stderr: {stderr}");
}

/// Local commits ahead of a *correct* upstream are still refused.
#[tokio::test]
async fn local_commits_ahead_of_upstream_are_refused_with_a_title() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let repos = ironforge_clone(work.path(), &server);
    feature_with_commit(&repos, "one");
    git(&repos.clone, &["push", "-u", "-q", "origin", "feature"]);
    git(&repos.clone, &["commit", "--allow-empty", "-m", "unpushed"]);

    Mock::given(method("POST"))
        .and(path("/api/v1/repos/a/r/pulls"))
        .respond_with(created_pr())
        .expect(0)
        .mount(&server)
        .await;

    let assert = ifg(&server, &home, &repos.clone)
        .args(["-t", "title", "--base", "main"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(stderr.contains("local commit(s)"), "stderr: {stderr}");
}

/// Publishing HEAD under another name does not update the branch sent as the
/// PR's `head`, even when both names live on the same target remote.
#[tokio::test]
async fn another_upstream_branch_does_not_prove_the_pr_head_is_pushed() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let repos = ironforge_clone(work.path(), &server);
    feature_with_commit(&repos, "one");
    git(&repos.clone, &["push", "-q", "origin", "feature"]);
    git(&repos.clone, &["commit", "--allow-empty", "-m", "two"]);
    git(&repos.clone, &["push", "-q", "origin", "HEAD:published"]);
    git(
        &repos.clone,
        &["branch", "--set-upstream-to", "origin/published", "feature"],
    );

    Mock::given(method("POST"))
        .and(path("/api/v1/repos/a/r/pulls"))
        .respond_with(created_pr())
        .expect(0)
        .mount(&server)
        .await;

    let output = ifg(&server, &home, &repos.clone)
        .args(["-t", "title", "--base", "main"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr);
    assert!(stderr.contains("origin/feature"), "stderr: {stderr}");
}

#[tokio::test]
async fn current_pr_head_is_accepted_with_a_differently_named_upstream() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let repos = ironforge_clone(work.path(), &server);
    feature_with_commit(&repos, "one");
    git(&repos.clone, &["push", "-q", "origin", "feature"]);
    git(&repos.clone, &["push", "-q", "origin", "HEAD:published"]);
    git(
        &repos.clone,
        &["branch", "--set-upstream-to", "origin/published", "feature"],
    );

    Mock::given(method("POST"))
        .and(path("/api/v1/repos/a/r/pulls"))
        .respond_with(created_pr())
        .expect(1)
        .mount(&server)
        .await;

    ifg(&server, &home, &repos.clone)
        .args(["-t", "title", "--base", "main"])
        .assert()
        .success();
}

/// An explicit `--head` names a branch the server may already know: no local
/// push check, so this keeps working outside a checkout.
#[tokio::test]
async fn explicit_head_skips_the_local_push_check() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let empty = TempDir::new().unwrap();

    Mock::given(method("POST"))
        .and(path("/api/v1/repos/a/r/pulls"))
        .respond_with(created_pr())
        .expect(1)
        .mount(&server)
        .await;

    ifg(&server, &home, empty.path())
        .args(["--head", "feature", "-t", "title", "--base", "main"])
        .assert()
        .success();
}
