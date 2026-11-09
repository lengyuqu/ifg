use assert_cmd::Command;
use serde_json::json;
use std::path::Path;
use tempfile::TempDir;
use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};

fn git(dir: &Path, args: &[&str]) -> String {
    let result = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .envs([
            ("GIT_AUTHOR_NAME", "ifg-test"),
            ("GIT_AUTHOR_EMAIL", "test@example.invalid"),
            ("GIT_COMMITTER_NAME", "ifg-test"),
            ("GIT_COMMITTER_EMAIL", "test@example.invalid"),
            (
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            ),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap().trim().to_string()
}

async fn checkout_case(deleted: bool, enriched: bool, mirror: bool) {
    let server = MockServer::start().await;
    let root = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let work = root.path().join("work");
    let target = root.path().join("target.git");
    let fork = root.path().join("fork.git");
    std::fs::create_dir(&work).unwrap();
    git(root.path(), &["init", "--bare", target.to_str().unwrap()]);
    git(root.path(), &["init", "--bare", fork.to_str().unwrap()]);
    git(&work, &["init", "-b", "main"]);
    git(&work, &["commit", "--allow-empty", "-m", "base"]);
    git(
        &work,
        &["remote", "add", "target", target.to_str().unwrap()],
    );
    git(&work, &["remote", "add", "fork", fork.to_str().unwrap()]);
    git(&work, &["push", "target", "main"]);
    git(&work, &["checkout", "-b", "feature"]);
    git(&work, &["commit", "--allow-empty", "-m", "head"]);
    git(&work, &["push", "fork", "feature"]);
    if mirror {
        git(&work, &["push", "target", "HEAD:refs/pull/1/head"]);
    }
    if deleted {
        git(&work, &["push", "fork", ":feature"]);
    } else {
        // The source branch is fresher than the mirror and must win.
        git(&work, &["commit", "--allow-empty", "-m", "new source head"]);
        git(&work, &["push", "fork", "feature"]);
    }
    let expected = git(&work, &["rev-parse", "HEAD"]);
    git(&work, &["checkout", "main"]);
    if !enriched {
        std::fs::remove_dir_all(&fork).unwrap();
    }
    for (local, remote) in [(&target, "alice/demo"), (&fork, "bob/fork")] {
        git(
            &work,
            &[
                "config",
                &format!("url.{}.insteadOf", local.display()),
                &format!("{}/git/{remote}.git", server.uri()),
            ],
        );
    }
    Mock::given(path("/api/v1/repos/alice/demo/pulls/1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id":1,"repo_id":1,"number":1,"title":"Fork PR","state":"open","author_id":1,
            "head_branch":"feature","base_branch":"main","head_repo_id":2,
            "head_repo_owner":if enriched {Some("bob")} else {None},
            "head_repo_name":if enriched {Some("fork")} else {None}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.current_dir(&work)
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_fixture")
        .env("XDG_CONFIG_HOME", home.path())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .args(["-R", "alice/demo", "pr", "checkout", "1"]);
    if mirror || !deleted {
        cmd.assert().success();
        assert_eq!(git(&work, &["rev-parse", "HEAD"]), expected);
        assert_eq!(git(&work, &["branch", "--show-current"]), "pr-1");
    } else {
        let result = cmd.assert().failure();
        assert!(String::from_utf8_lossy(&result.get_output().stderr)
            .contains("target repository mirror is unavailable"));
    }
}

#[tokio::test]
async fn deleted_fork_branch_uses_the_target_mirror() {
    checkout_case(true, true, true).await;
}

#[tokio::test]
async fn deleted_fork_repository_does_not_require_head_enrichment() {
    checkout_case(true, false, true).await;
}

#[tokio::test]
async fn live_fork_branch_is_preferred_over_a_stale_mirror() {
    checkout_case(false, true, true).await;
}

#[tokio::test]
async fn older_servers_without_a_fork_mirror_fail_explicitly() {
    checkout_case(true, true, false).await;
}
