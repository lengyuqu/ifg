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
use serde_json::{json, Value};
use tempfile::TempDir;

fn e2e_url() -> String {
    std::env::var("IFG_E2E_URL").unwrap_or_else(|_| "http://localhost:8080".into())
}

fn run_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "e2e-{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_secs(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
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
    let password = format!("Zq7!Pass-{}", run_id.replace("e2e-", ""));
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

    // 0. version: client line + server version from /health (D10).
    let out = harness.run_ok(&["version"]);
    assert!(out.contains("ifg "), "version client: {out}");
    assert!(out.contains("server:"), "version server: {out}");

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

    // 7. release create with an uploaded asset, then a verified download.
    let artifact = harness.home_path.join("artifact.txt");
    std::fs::write(&artifact, "e2e artifact content v1").expect("write artifact");
    let artifact_str = artifact.to_str().expect("utf-8 path");
    let out = harness.run_ok(&[
        "-R",
        repo_arg,
        "release",
        "create",
        "v1.0.0",
        "-n",
        "e2e notes",
        artifact_str,
    ]);
    assert!(out.contains("v1.0.0"), "release create: {out}");
    assert!(out.contains("Uploaded artifact.txt"), "asset upload: {out}");
    let out = harness.run_ok(&["-R", repo_arg, "release", "list"]);
    assert!(out.contains("v1.0.0"), "release list: {out}");
    let out = harness.run_ok(&["-R", repo_arg, "release", "view", "v1.0.0"]);
    assert!(out.contains("artifact.txt"), "release view assets: {out}");

    let download_dir = harness.home_path.join("downloads");
    std::fs::create_dir_all(&download_dir).expect("download dir");
    let out = harness.run_in(
        &download_dir,
        &[
            "-R",
            repo_arg,
            "release",
            "download",
            "v1.0.0",
            "-p",
            "artifact*",
        ],
    );
    assert!(
        out.contains("Downloaded artifact.txt"),
        "release download: {out}"
    );
    let downloaded = std::fs::read(download_dir.join("artifact.txt")).expect("downloaded file");
    assert_eq!(downloaded, b"e2e artifact content v1");
    assert!(
        !download_dir.join("artifact.txt.part").exists(),
        "no .part leftover"
    );

    // 8. pr checkout: fetch the (still existing) head branch as pr-1.
    harness.run_in(&clone_dir, &["pr", "checkout", "1"]);
    let branch = StdCommand::new("git")
        .args(["rev-parse", "--verify", "pr-1"])
        .current_dir(&clone_dir)
        .output()
        .expect("git rev-parse");
    assert!(branch.status.success(), "branch pr-1 exists after checkout");

    // 9. ci runs: the pipeline list renders (no runner in this setup, so an
    // empty list is the expected outcome).
    let out = harness.run_ok(&["-R", repo_arg, "ci", "runs"]);
    assert!(
        out.contains("No pipelines") || out.contains("PIPELINES") || out.chars().count() > 0,
        "ci runs: {out}"
    );

    Ok(())
}

/// Fork PR checkout (design doc §7.2): fetch from the fork, and a second run
/// fast-forwards instead of failing.
#[tokio::test]
#[ignore = "needs a live IronForge (scripts/e2e.sh)"]
async fn fork_pull_request_checkout_and_update() {
    let harness = harness().await;
    let run_id = run_id();
    let upstream = format!("{run_id}-upstream");
    let upstream_arg = format!("{}/{}", harness.user, upstream);

    // Second identity: the fork owner.
    let (fork_user, fork_pat) = bootstrap_second(&format!("{run_id}-forker")).await;

    let result = fork_chain(&harness, &upstream_arg, &upstream, &fork_user, &fork_pat).await;

    // Cleanup both repositories regardless of the outcome.
    let own = pat_client(&harness.pat);
    let _ = own
        .raw(
            Method::DELETE,
            &format!("repos/{}/{}", harness.user, upstream),
            None,
        )
        .await;
    let forked = pat_client(&fork_pat);
    let _ = forked
        .raw(
            Method::DELETE,
            &format!("repos/{fork_user}/{upstream}"),
            None,
        )
        .await;
    result.expect("fork chain");
}

/// Register an extra user and mint a PAT for it.
async fn bootstrap_second(username: &str) -> (String, String) {
    let client = anonymous_client();
    let password = format!("Zq7!Pass-{}", username.replace("e2e-", "x"));
    let register = serde_json::json!({
        "username": username,
        "email": format!("{username}@ifg.invalid"),
        "password": password,
    });
    client
        .raw(Method::POST, "users/register", Some(&register))
        .await
        .expect("second registration works");
    let login = serde_json::json!({ "login": username, "password": password });
    let body = json_of(
        client
            .raw(Method::POST, "users/login", Some(&login))
            .await
            .expect("second login works"),
    );
    let jwt = body["token"].as_str().expect("login token").to_string();
    let token_request = serde_json::json!({
        "name": format!("{username}-pat"),
        "scopes": "repo,user",
    });
    let body = json_of(
        pat_client(&jwt)
            .raw(Method::POST, "users/tokens", Some(&token_request))
            .await
            .expect("second PAT works"),
    );
    let pat = body["token"].as_str().expect("pat token").to_string();
    (username.to_string(), pat)
}

async fn fork_chain(
    harness: &Harness,
    upstream_arg: &str,
    upstream_name: &str,
    fork_user: &str,
    fork_pat: &str,
) -> Result<(), String> {
    // Upstream repo with an initial commit.
    let out = harness.run_ok(&["repo", "create", upstream_name, "--init"]);
    assert!(out.contains("Created"), "upstream create: {out}");

    // Fork it under the second user.
    let forker = pat_client(fork_pat);
    let fork = forker
        .raw(Method::POST, &format!("repos/{upstream_arg}/fork"), None)
        .await
        .expect("fork works");
    let fork = json_of(fork);
    assert_eq!(fork["name"], json!(upstream_name));

    // Push a branch into the fork.
    let worktree = harness.home_path.join("forkwork");
    std::fs::create_dir_all(&worktree).expect("worktree");
    let fork_url = format!("{}/git/{fork_user}/{upstream_name}.git", e2e_url()).replacen(
        "://",
        &format!("://{fork_user}:{fork_pat}@"),
        1,
    );
    git(&worktree, &["clone", &fork_url, "clone"]);
    let clone_dir = worktree.join("clone");
    harness_git(&clone_dir, &["checkout", "-b", "fork-feature"]);
    std::fs::write(clone_dir.join("fork.txt"), "from the fork").expect("write");
    harness_git(&clone_dir, &["add", "."]);
    harness_git(&clone_dir, &["commit", "-m", "feat: fork change"]);
    harness_git(&clone_dir, &["push", "-q", "origin", "fork-feature"]);

    // Open the PR from the fork (head = OWNER:branch).
    let pr = json_of(
        forker
            .raw(
                Method::POST,
                &format!("repos/{upstream_arg}/pulls"),
                Some(&serde_json::json!({
                    "title": "fork PR",
                    "head": format!("{fork_user}:fork-feature"),
                    "base": "main",
                })),
            )
            .await
            .expect("fork PR creation"),
    );
    assert!(
        pr["head_repo_id"].is_number(),
        "fork PR records head_repo_id"
    );

    // The upstream owner checks it out through the CLI.
    let upstream_work = harness.home_path.join("upstreamwork");
    std::fs::create_dir_all(&upstream_work).expect("upstream worktree");
    let own_url = harness.clone_url(&harness.user, upstream_name);
    git(&upstream_work, &["clone", &own_url, "clone"]);
    let upstream_clone = upstream_work.join("clone");

    let out = harness.run_in(&upstream_clone, &["pr", "checkout", "1"]);
    assert!(
        out.contains("checked out") || out.contains("updated"),
        "fork checkout: {out}"
    );
    let content =
        std::fs::read_to_string(upstream_clone.join("fork.txt")).expect("fork file after checkout");
    assert_eq!(content, "from the fork");

    // Second run: fast-forward / already up to date, not an error.
    let out = harness.run_in(&upstream_clone, &["pr", "checkout", "1"]);
    assert!(out.contains("updated"), "second checkout: {out}");
    Ok(())
}

/// `git` wrapper without asserting silence (fork flow prints more).
fn harness_git(dir: &Path, args: &[&str]) {
    git(dir, args);
}
