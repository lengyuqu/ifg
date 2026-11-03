//! End-to-end tests for the `ifg` CLI against a live IronForge instance
//! (design doc §7.1). Every test is `#[ignore]`d — they need a server and
//! run via `scripts/e2e.sh` (`cargo test -p ifg-e2e -- --ignored`).
//!
//! Environment:
//! - `IFG_E2E_URL`: base URL of the instance (default `http://localhost:8080`)
//! - `IFG_E2E_PAT`: existing PAT (scope `repo,user`); skips bootstrap
//! - otherwise: register → login → create PAT (registration is open by
//!   default; verified against the IronForge baseline revision)
//!
//! All names carry a run ID so repeated runs against a persistent data
//! volume do not collide; created repositories are deleted at the end.

use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::Duration;

use assert_cmd::Command as IfgCommand;
use ifg_client::{Client, Method, Secret};
use serde_json::Value;
use tempfile::TempDir;

fn e2e_url() -> String {
    std::env::var("IFG_E2E_URL").unwrap_or_else(|_| "http://localhost:8080".into())
}

fn run_id() -> String {
    format!(
        "e2e-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_secs()
    )
}

fn json_of(bytes: Vec<u8>) -> Value {
    serde_json::from_slice(&bytes).expect("JSON response")
}

/// A temp XDG home, the real `ifg` binary, the instance URL and a PAT.
struct Harness {
    _home: TempDir,
    home_path: PathBuf,
    user: String,
    pat: String,
}

impl Harness {
    /// A fresh `ifg` invocation; assert_cmd accumulates arguments, so a new
    /// command is built per call.
    fn ifg(&self) -> IfgCommand {
        let mut ifg = IfgCommand::cargo_bin("ifg").unwrap();
        ifg.env("XDG_CONFIG_HOME", &self.home_path)
            .env("IRONFORGE_URL", e2e_url())
            .env("IRONFORGE_PAT", &self.pat);
        ifg
    }

    fn run_ok(&self, args: &[&str]) -> String {
        let output = self.ifg().args(args).assert().success();
        String::from_utf8(output.get_output().stdout.clone()).expect("utf-8 stdout")
    }

    fn run_in(&self, dir: &Path, args: &[&str]) -> String {
        let output = self.ifg().args(args).current_dir(dir).assert().success();
        String::from_utf8(output.get_output().stdout.clone()).expect("utf-8 stdout")
    }

    fn clone_url(&self, owner: &str, repo: &str) -> String {
        format!("{}/git/{}/{}.git", e2e_url(), owner, repo).replacen(
            "://",
            &format!("://{}:{}@", self.user, self.pat),
            1,
        )
    }
}

fn git(dir: &Path, args: &[&str]) {
    let status = StdCommand::new("git")
        .args(args)
        .current_dir(dir)
        .envs([
            ("GIT_AUTHOR_NAME", "ifg-e2e"),
            ("GIT_AUTHOR_EMAIL", "ifg-e2e@ifg.invalid"),
            ("GIT_COMMITTER_NAME", "ifg-e2e"),
            ("GIT_COMMITTER_EMAIL", "ifg-e2e@ifg.invalid"),
        ])
        .status()
        .expect("git is installed");
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

fn anonymous_client() -> Client {
    Client::builder(e2e_url())
        .user_agent(concat!("ifg-e2e/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client builds")
}

fn pat_client(pat: &str) -> Client {
    Client::builder(e2e_url())
        .user_agent(concat!("ifg-e2e/", env!("CARGO_PKG_VERSION")))
        .token(Secret::new(pat))
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client builds")
}

/// Wait for `/health` (30s), then obtain a user/PAT pair.
async fn harness() -> Harness {
    let probe = anonymous_client();
    let mut healthy = false;
    let mut last_error = String::new();
    for _ in 0..30 {
        match probe.health().await {
            Ok(_) => {
                healthy = true;
                break;
            }
            Err(error) => last_error = error.to_string(),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert!(
        healthy,
        "no IronForge at {} (last error: {last_error}); start one, e.g.: \
         docker compose -f deploy/docker-compose.yml up -d",
        e2e_url()
    );

    let (user, pat) = match std::env::var("IFG_E2E_PAT") {
        Ok(pat) if !pat.is_empty() => {
            let me = pat_client(&pat)
                .users()
                .me()
                .await
                .expect("IFG_E2E_PAT works");
            (me.username, pat)
        }
        _ => bootstrap().await,
    };

    let home = TempDir::new().expect("temp dir");

    Harness {
        home_path: home.path().to_path_buf(),
        _home: home,
        user,
        pat,
    }
}

/// register → login → create PAT (§7.3 bootstrap).
async fn bootstrap() -> (String, String) {
    let run_id = run_id();
    let client = anonymous_client();
    let username = format!("{run_id}-user");
    let password = format!("{run_id}-Pass!X9");
    let register = serde_json::json!({
        "username": username,
        "email": format!("{username}@ifg.invalid"),
        "password": password,
    });
    client
        .raw(Method::POST, "users/register", Some(&register))
        .await
        .expect("registration works (open by default; else set IFG_E2E_PAT)");

    let login = serde_json::json!({ "login": username, "password": password });
    let login_body = json_of(
        client
            .raw(Method::POST, "users/login", Some(&login))
            .await
            .expect("login works"),
    );
    let jwt = login_body["token"]
        .as_str()
        .expect("login returns a token")
        .to_string();

    let token_request = serde_json::json!({
        "name": format!("{run_id}-pat"),
        "scopes": "repo,user",
    });
    let body = json_of(
        pat_client(&jwt)
            .raw(Method::POST, "users/tokens", Some(&token_request))
            .await
            .expect("PAT creation works"),
    );
    let pat = body["token"]
        .as_str()
        .expect("token in response")
        .to_string();
    (username, pat)
}

/// Delete the repositories created by this run (data volumes persist).
async fn cleanup(harness: &Harness, repos: &[String]) {
    let client = pat_client(&harness.pat);
    for repo in repos {
        let path = format!("repos/{}/{}", harness.user, repo);
        if let Err(error) = client.raw(Method::DELETE, &path, None).await {
            eprintln!("cleanup failed for {path}: {error}");
        }
    }
    let _ = harness.home_path;
}

/// The M2 acceptance chain (design doc §9): 建仓 → Issue 建关 → PR 建/合并,
/// through the real `ifg` binary, including `auth login --with-token`.
#[tokio::test]
#[ignore = "needs a live IronForge (scripts/e2e.sh)"]
async fn full_chain_create_issue_pr_merge() {
    let harness = harness().await;
    let run_id = run_id();
    let repo = format!("{run_id}-repo");
    let repo_arg = format!("{}/{}", harness.user, repo);
    let created = vec![repo.clone()];

    let result = full_chain(&harness, &repo_arg).await;
    cleanup(&harness, &created).await;
    result.expect("M2 chain");
}

async fn full_chain(harness: &Harness, repo_arg: &str) -> Result<(), String> {
    let repo_name = repo_arg.split_once('/').expect("OWNER/NAME").1;

    // 1. auth login --with-token (writes the config from the env PAT).
    let output = harness
        .ifg()
        .args(["auth", "login", "--with-token"])
        .write_stdin(format!("{}\n", harness.pat))
        .assert()
        .success();
    let stdout = String::from_utf8(output.get_output().stdout.clone()).expect("utf-8");
    assert!(stdout.contains("Logged in"), "login output: {stdout}");

    // 2. repo create (auto-init so main exists as the PR base).
    let out = harness.run_ok(&["repo", "create", repo_name, "--init"]);
    assert!(out.contains("Created"), "repo create: {out}");
    let out = harness.run_ok(&["repo", "view", "-R", repo_arg]);
    assert!(out.contains(repo_name), "repo view: {out}");

    // 3. issue create / list / comment / close.
    let out = harness.run_ok(&[
        "-R",
        repo_arg,
        "issue",
        "create",
        "-t",
        "e2e issue",
        "-b",
        "body",
    ]);
    assert!(out.contains("#1"), "issue create: {out}");
    let out = harness.run_ok(&["-R", repo_arg, "issue", "list"]);
    assert!(out.contains("e2e issue"), "issue list: {out}");
    harness.run_ok(&["-R", repo_arg, "issue", "comment", "1", "-b", "e2e comment"]);
    let out = harness.run_ok(&["-R", repo_arg, "issue", "close", "1"]);
    assert!(out.contains("closed"), "issue close: {out}");

    // 4. clone (PAT embedded; `auth git-credential` lands in M3), branch,
    //    commit, push — with explicit git identity (no global config in CI).
    let worktree = harness.home_path.join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree dir");
    let clone_url = harness.clone_url(&harness.user, repo_name);
    git(&worktree, &["clone", &clone_url, "clone"]);
    let clone_dir = worktree.join("clone");
    git(&clone_dir, &["checkout", "-b", "feature"]);
    std::fs::write(clone_dir.join("e2e.txt"), "e2e").expect("write file");
    git(&clone_dir, &["add", "."]);
    git(&clone_dir, &["commit", "-m", "feat: e2e commit"]);
    git(&clone_dir, &["push", "-q", "origin", "feature"]);

    // 5. pr create from inside the clone (remote inference + current branch).
    let out = harness.run_in(&clone_dir, &["pr", "create", "-t", "e2e PR"]);
    assert!(out.contains("#1"), "pr create: {out}");
    let out = harness.run_in(&clone_dir, &["pr", "list"]);
    assert!(out.contains("e2e PR"), "pr list: {out}");

    // 6. merge (squash) and verify.
    let out = harness.run_in(&clone_dir, &["pr", "merge", "1", "--squash"]);
    assert!(out.contains("Merged"), "pr merge: {out}");
    let out = harness.run_in(&clone_dir, &["pr", "view", "1"]);
    assert!(out.contains("merged"), "pr view after merge: {out}");

    Ok(())
}
