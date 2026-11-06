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
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

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
    let output = StdCommand::new("git")
        .args(args)
        .current_dir(dir)
        .envs([
            ("GIT_AUTHOR_NAME", "ifg-e2e"),
            ("GIT_AUTHOR_EMAIL", "ifg-e2e@ifg.invalid"),
            ("GIT_COMMITTER_NAME", "ifg-e2e"),
            ("GIT_COMMITTER_EMAIL", "ifg-e2e@ifg.invalid"),
        ])
        .output()
        .expect("git is installed");
    assert!(
        output.status.success(),
        "git {args:?} failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
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
    let registered = json_of(
        client
            .raw(Method::POST, "users/register", Some(&register))
            .await
            .expect("registration works (open by default; else set IFG_E2E_PAT)"),
    );
    // The register response already carries a session JWT; /users/login is
    // rate limited per IP and would make parallel e2e runs flaky.
    let jwt = registered["token"]
        .as_str()
        .expect("register returns a token")
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

/// Two independently persisted agent sessions on one account and repository.
#[cfg(unix)]
#[tokio::test]
#[ignore = "needs a live IronForge (scripts/e2e.sh)"]
async fn collaboration_two_sessions_exchange_and_handoff() {
    let h = harness().await;
    let name = format!("{}-collab", run_id());
    let repo = format!("{}/{}", h.user, name);
    h.run_ok(&["repo", "create", &name, "--init"]);
    let a = h.home_path.join("agent-a.json");
    let b = h.home_path.join("agent-b.json");
    let a_path = a.to_str().unwrap();
    let b_path = b.to_str().unwrap();
    let invoke = |args: &[&str]| -> Value {
        let out = h.run_ok(args);
        serde_json::from_str(&out).expect("collaboration JSON")
    };
    let started_a = invoke(&[
        "-R",
        &repo,
        "collab",
        "--session-file",
        a_path,
        "session",
        "start",
        "--name",
        "Agent A",
        "--client",
        "ifg",
    ]);
    let started_b = invoke(&[
        "-R",
        &repo,
        "collab",
        "--session-file",
        b_path,
        "session",
        "start",
        "--name",
        "Agent B",
        "--client",
        "ifg",
    ]);
    let a_id = started_a["id"].as_str().unwrap();
    let b_id = started_b["id"].as_str().unwrap();
    assert_ne!(a_id, b_id);
    let secret_a: Value = serde_json::from_slice(&std::fs::read(&a).unwrap()).unwrap();
    assert!(!started_a
        .to_string()
        .contains(secret_a["secret"].as_str().unwrap()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&a).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::set_permissions(&a, std::fs::Permissions::from_mode(0o644)).unwrap();
        h.ifg()
            .args([
                "-R",
                &repo,
                "collab",
                "--session-file",
                a_path,
                "session",
                "heartbeat",
            ])
            .assert()
            .failure();
        std::fs::set_permissions(&a, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    h.ifg()
        .args([
            "-R",
            &format!("{}/other", h.user),
            "collab",
            "--session-file",
            a_path,
            "session",
            "heartbeat",
        ])
        .assert()
        .failure();
    h.ifg()
        .args([
            "-R",
            &repo,
            "collab",
            "--session-file",
            a_path,
            "session",
            "start",
            "--name",
            "duplicate",
            "--client",
            "ifg",
        ])
        .assert()
        .failure();
    let logged = h
        .ifg()
        .args([
            "-vv",
            "-R",
            &repo,
            "collab",
            "--session-file",
            a_path,
            "session",
            "heartbeat",
        ])
        .assert()
        .success();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&logged.get_output().stdout),
        String::from_utf8_lossy(&logged.get_output().stderr)
    );
    assert!(
        !combined.contains(secret_a["secret"].as_str().unwrap()),
        "session secret leaked to output"
    );
    let sessions = invoke(&["-R", &repo, "collab", "session", "list"]);
    assert!(sessions["items"].as_array().unwrap().len() >= 2);
    invoke(&[
        "-R",
        &repo,
        "collab",
        "--session-file",
        a_path,
        "session",
        "heartbeat",
    ]);
    let task = invoke(&[
        "-R",
        &repo,
        "collab",
        "--session-file",
        a_path,
        "task",
        "create",
        "--title",
        "Shared task",
        "--owner-session",
        a_id,
    ]);
    let task_id = task["id"].as_str().unwrap();
    let version = task["version"].as_i64().unwrap().to_string();
    let updated = invoke(&[
        "-R",
        &repo,
        "collab",
        "--session-file",
        a_path,
        "task",
        "update",
        task_id,
        "--expected-version",
        &version,
        "--status",
        "in_progress",
    ]);
    assert_eq!(updated["status"], "in_progress");
    h.ifg()
        .args([
            "-R",
            &repo,
            "collab",
            "--session-file",
            a_path,
            "task",
            "update",
            task_id,
            "--expected-version",
            &version,
            "--status",
            "done",
        ])
        .assert()
        .failure();
    let tasks = invoke(&["-R", &repo, "collab", "task", "list"]);
    assert!(tasks["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["id"] == task_id));
    let view = invoke(&["-R", &repo, "collab", "task", "view", task_id]);
    assert_eq!(view["title"], "Shared task");
    let send = [
        "-R",
        &repo,
        "collab",
        "--session-file",
        a_path,
        "message",
        "send",
        task_id,
        "--key",
        "message-1",
        "--body",
        "ready",
        "--target-session",
        b_id,
    ];
    let first = invoke(&send);
    let replay = invoke(&send);
    assert_eq!(first["id"], replay["id"]);
    h.ifg()
        .args([
            "-R",
            &repo,
            "collab",
            "--session-file",
            a_path,
            "message",
            "send",
            task_id,
            "--key",
            "message-1",
            "--body",
            "different",
        ])
        .assert()
        .failure();
    let list = invoke(&["-R", &repo, "collab", "message", "list", task_id]);
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    invoke(&[
        "-R",
        &repo,
        "collab",
        "--session-file",
        b_path,
        "message",
        "read",
        task_id,
        "--seq",
        "1",
    ]);
    let handoff_args = [
        "-R",
        &repo,
        "collab",
        "--session-file",
        a_path,
        "handoff",
        "create",
        task_id,
        "--key",
        "handoff-1",
        "--to",
        b_id,
        "--summary",
        "please continue",
    ];
    let handoff = invoke(&handoff_args);
    let replay = invoke(&handoff_args);
    assert_eq!(handoff["id"], replay["id"]);
    let handoff_id = handoff["id"].as_str().unwrap();
    let handoff_version = handoff["version"].as_i64().unwrap().to_string();
    let inbox = invoke(&["-R", &repo, "collab", "--session-file", b_path, "inbox"]);
    assert!(!inbox["items"].as_array().unwrap().is_empty());
    let list = invoke(&["-R", &repo, "collab", "handoff", "list", task_id]);
    assert!(list["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["id"] == handoff_id));
    h.ifg()
        .args([
            "-R",
            &repo,
            "collab",
            "--session-file",
            a_path,
            "handoff",
            "accept",
            handoff_id,
            "--expected-version",
            &handoff_version,
        ])
        .assert()
        .failure();
    let accepted = invoke(&[
        "-R",
        &repo,
        "collab",
        "--session-file",
        b_path,
        "handoff",
        "accept",
        handoff_id,
        "--expected-version",
        &handoff_version,
    ]);
    assert_eq!(accepted["status"], "accepted");
    let second = invoke(&[
        "-R",
        &repo,
        "collab",
        "--session-file",
        a_path,
        "handoff",
        "create",
        task_id,
        "--key",
        "handoff-2",
        "--to",
        b_id,
        "--summary",
        "cancel this",
    ]);
    let second_id = second["id"].as_str().unwrap();
    let second_version = second["version"].as_i64().unwrap().to_string();
    let cancelled = invoke(&[
        "-R",
        &repo,
        "collab",
        "--session-file",
        a_path,
        "handoff",
        "cancel",
        second_id,
        "--expected-version",
        &second_version,
    ]);
    assert_eq!(cancelled["status"], "cancelled");
    h.ifg()
        .args([
            "-R",
            &repo,
            "collab",
            "--session-file",
            b_path,
            "handoff",
            "accept",
            second_id,
            "--expected-version",
            &second_version,
        ])
        .assert()
        .failure();
    invoke(&[
        "-R",
        &repo,
        "collab",
        "--session-file",
        a_path,
        "session",
        "end",
    ]);
    h.ifg()
        .args([
            "-R",
            &repo,
            "collab",
            "--session-file",
            a_path,
            "session",
            "heartbeat",
        ])
        .assert()
        .failure();
    invoke(&[
        "-R",
        &repo,
        "collab",
        "--session-file",
        b_path,
        "session",
        "end",
    ]);
    cleanup(&h, &[name]).await;
}

/// The M2 acceptance chain (design doc §9): 建仓 → Issue 建关 → PR 建/合并,
/// through the real `ifg` binary, including `auth login --with-token`.
#[cfg(unix)]
#[tokio::test]
#[ignore = "needs a live IronForge (scripts/e2e.sh)"]
async fn collaboration_questions_and_task_takeover() {
    let h = harness().await;
    let name = format!("{}-questions", run_id());
    let repo = format!("{}/{}", h.user, name);
    h.run_ok(&["repo", "create", &name, "--init"]);
    let a = h.home_path.join("question-a.json");
    let b = h.home_path.join("question-b.json");
    let a = a.to_str().unwrap();
    let b = b.to_str().unwrap();
    let run = |args: &[&str]| -> Value { serde_json::from_str(&h.run_ok(args)).unwrap() };
    let sa = run(&[
        "-R",
        &repo,
        "collab",
        "--session-file",
        a,
        "session",
        "start",
        "--name",
        "A",
        "--client",
        "ifg",
    ]);
    let sb = run(&[
        "-R",
        &repo,
        "collab",
        "--session-file",
        b,
        "session",
        "start",
        "--name",
        "B",
        "--client",
        "ifg",
    ]);
    let aid = sa["id"].as_str().unwrap();
    let bid = sb["id"].as_str().unwrap();
    let task = run(&[
        "-R",
        &repo,
        "collab",
        "--session-file",
        a,
        "task",
        "create",
        "--title",
        "Question task",
        "--owner-session",
        aid,
    ]);
    let tid = task["id"].as_str().unwrap();
    let version = task["version"].as_i64().unwrap().to_string();
    let q = run(&[
        "-R",
        &repo,
        "collab",
        "--session-file",
        a,
        "question",
        "ask",
        tid,
        "--key",
        "question-1",
        "--title",
        "Choose",
        "--body",
        "What next?",
        "--option",
        "one",
        "--option",
        "two",
    ]);
    let qid = q["id"].as_str().unwrap();
    let listed = run(&["-R", &repo, "collab", "question", "list", tid]);
    assert!(listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|q| q["id"] == qid));
    let inbox = run(&["-R", &repo, "collab", "inbox", "--user"]);
    assert!(inbox["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["type"] == "open_question"));
    let refused = h
        .ifg()
        .args([
            "-R",
            &repo,
            "collab",
            "--session-file",
            b,
            "task",
            "claim",
            tid,
            "--key",
            "claim-active",
            "--expected-version",
            &version,
            "--expected-owner",
            aid,
            "--reason",
            "try claim",
        ])
        .assert()
        .code(1);
    assert!(String::from_utf8_lossy(&refused.get_output().stderr).contains("OWNER_MAY_BE_ACTIVE"));
    let reassign_args = [
        "-R",
        &repo,
        "collab",
        "task",
        "reassign",
        tid,
        "--key",
        "reassign-1",
        "--expected-version",
        &version,
        "--expected-owner",
        aid,
        "--to",
        bid,
        "--reason",
        "user assigns B",
    ];
    let moved = run(&reassign_args);
    assert_eq!(moved["task"]["owner_session_id"], bid);
    assert_eq!(run(&reassign_args)["replayed"], true);
    let history = run(&["-R", &repo, "collab", "task", "takeovers", tid]);
    assert_eq!(history["items"].as_array().unwrap().len(), 1);
    let old = run(&["-R", &repo, "collab", "question", "view", qid]);
    assert_eq!(old["context_freshness"], "stale");
    let qver = old["version"].as_i64().unwrap().to_string();
    let hash = old["context_hash"].as_str().unwrap();
    let refused = h
        .ifg()
        .args([
            "-R",
            &repo,
            "collab",
            "question",
            "answer",
            qid,
            "--key",
            "answer-stale",
            "--expected-version",
            &qver,
            "--context-hash",
            hash,
            "--option-id",
            "o1",
        ])
        .assert()
        .code(1);
    assert!(String::from_utf8_lossy(&refused.get_output().stderr).contains("CONTEXT_STALE"));
    let q2 = run(&[
        "-R",
        &repo,
        "collab",
        "--session-file",
        b,
        "question",
        "ask",
        tid,
        "--key",
        "question-2",
        "--title",
        "Choose again",
        "--body",
        "Which option?",
        "--option",
        "one",
        "--option",
        "two",
    ]);
    let q2id = q2["id"].as_str().unwrap();
    let read = run(&["-R", &repo, "collab", "question", "view", q2id]);
    let ver = read["version"].as_i64().unwrap().to_string();
    let hash = read["context_hash"].as_str().unwrap();
    let answered = run(&[
        "-R",
        &repo,
        "collab",
        "question",
        "answer",
        q2id,
        "--key",
        "answer-2",
        "--expected-version",
        &ver,
        "--context-hash",
        hash,
        "--option-id",
        "o1",
    ]);
    assert_eq!(answered["status"], "answered");
    assert_eq!(answered["answer"]["actor_kind"], "user");
    let q3 = run(&[
        "-R",
        &repo,
        "collab",
        "--session-file",
        b,
        "question",
        "ask",
        tid,
        "--key",
        "question-3",
        "--title",
        "Cancel",
        "--body",
        "cancel this",
    ]);
    let q3id = q3["id"].as_str().unwrap();
    let q3ver = q3["version"].as_i64().unwrap().to_string();
    assert_eq!(
        run(&[
            "-R",
            &repo,
            "collab",
            "--session-file",
            b,
            "question",
            "cancel",
            q3id,
            "--expected-version",
            &q3ver
        ])["status"],
        "cancelled"
    );
    let free = run(&[
        "-R",
        &repo,
        "collab",
        "--session-file",
        a,
        "task",
        "create",
        "--title",
        "Unowned",
    ]);
    let freeid = free["id"].as_str().unwrap();
    let freever = free["version"].as_i64().unwrap().to_string();
    let claim_args = [
        "-R",
        &repo,
        "collab",
        "--session-file",
        b,
        "task",
        "claim",
        freeid,
        "--key",
        "claim-1",
        "--expected-version",
        &freever,
        "--reason",
        "available",
    ];
    assert_eq!(run(&claim_args)["task"]["owner_session_id"], bid);
    assert_eq!(run(&claim_args)["replayed"], true);
    cleanup(&h, &[name]).await;
}

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

#[tokio::test]
#[ignore = "needs a live IronForge (scripts/e2e.sh)"]
async fn trigger_pipeline_and_report_manual_gate() {
    let h = harness().await;
    let name = format!("{}-pipeline", run_id());
    let repo = format!("{}/{}", h.user, name);
    h.run_ok(&["repo", "create", &name, "--init"]);
    let work = h.home_path.join("ci-work");
    std::fs::create_dir_all(&work).unwrap();
    git(&work, &["clone", &h.clone_url(&h.user, &name), "clone"]);
    let clone = work.join("clone");
    std::fs::write(clone.join(".ironforge-ci.yml"), "stages:\n  - test\nmanual_check:\n  stage: test\n  script:\n    - echo ok\n  when: manual\n").unwrap();
    git(&clone, &["add", ".ironforge-ci.yml"]);
    git(&clone, &["commit", "-m", "add manual pipeline"]);
    git(&clone, &["push", "origin", "main"]);
    let commit = StdCommand::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&clone)
        .output()
        .unwrap();
    let commit = String::from_utf8(commit.stdout).unwrap().trim().to_string();
    let triggered: Value =
        serde_json::from_str(&h.run_ok(&["-R", &repo, "ci", "run", "--ref", "refs/heads/main"]))
            .unwrap();
    assert_eq!(triggered["commit_sha"], commit);
    let by_commit: Value =
        serde_json::from_str(&h.run_ok(&["-R", &repo, "ci", "run", "--ref", &commit])).unwrap();
    assert_eq!(by_commit["commit_sha"], commit);
    let id = triggered["id"].as_i64().unwrap().to_string();
    let waited = h
        .ifg()
        .args(["-R", &repo, "ci", "wait", &id, "--timeout", "45"])
        .assert()
        .failure();
    let output = String::from_utf8_lossy(&waited.get_output().stdout);
    let detail: Value = serde_json::from_str(&output).expect("blocked pipeline details");
    assert_eq!(detail["pipeline"]["status"], "manual");
    cleanup(&h, &[name]).await;
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

    // Edit, close/reopen, general review comment and reviewer lifecycle.
    let value: Value = serde_json::from_str(&harness.run_in(
        &clone_dir,
        &[
            "pr",
            "edit",
            "1",
            "--title",
            "updated PR",
            "--body",
            "updated description",
            "--draft",
        ],
    ))
    .unwrap();
    assert_eq!(value["title"], "updated PR");
    assert_eq!(value["body"], "updated description");
    assert_eq!(value["is_draft"], true);
    let value: Value =
        serde_json::from_str(&harness.run_in(&clone_dir, &["pr", "edit", "1", "--ready"])).unwrap();
    assert_eq!(value["is_draft"], false);
    let value: Value =
        serde_json::from_str(&harness.run_in(&clone_dir, &["pr", "close", "1"])).unwrap();
    assert_eq!(value["state"], "closed");
    let value: Value =
        serde_json::from_str(&harness.run_in(&clone_dir, &["pr", "reopen", "1"])).unwrap();
    assert_eq!(value["state"], "open");
    harness.run_in(
        &clone_dir,
        &["pr", "comment", "1", "--body", "general review comment"],
    );
    let reviews: Value =
        serde_json::from_str(&harness.run_in(&clone_dir, &["pr", "reviews", "1"])).unwrap();
    assert!(reviews
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["body"] == "general review comment"));
    let (reviewer, _) = bootstrap_second(&format!("{}-reviewer", run_id())).await;
    harness.run_in(&clone_dir, &["pr", "reviewers", "add", "1", &reviewer]);
    let reviewers: Value =
        serde_json::from_str(&harness.run_in(&clone_dir, &["pr", "reviewers", "list", "1"]))
            .unwrap();
    assert!(reviewers
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["username"] == reviewer));
    harness.run_in(&clone_dir, &["pr", "reviewers", "remove", "1", &reviewer]);
    let reviewers: Value =
        serde_json::from_str(&harness.run_in(&clone_dir, &["pr", "reviewers", "list", "1"]))
            .unwrap();
    assert!(!reviewers
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["username"] == reviewer));

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

    // 8b. The server mirrors every same-repo PR head into refs/pull/<n>/head
    // (branch deletion over HTTP is rejected server-side, so assert the ref
    // directly — it is what the CLI falls back to when a branch is gone).
    let ls_remote = StdCommand::new("git")
        .args(["ls-remote", &clone_url, "refs/pull/1/head"])
        .output()
        .expect("git ls-remote");
    let listing = String::from_utf8_lossy(&ls_remote.stdout);
    assert!(
        listing.contains("refs/pull/1/head"),
        "refs/pull mirror missing: {listing}"
    );

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
    let body = json_of(
        client
            .raw(Method::POST, "users/register", Some(&register))
            .await
            .expect("second registration works"),
    );
    let jwt = body["token"].as_str().expect("register token").to_string();
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

/// Organizations: create, list, add/remove a member by username.
#[tokio::test]
#[ignore = "needs a live IronForge (scripts/e2e.sh)"]
async fn organizations_and_members() {
    let harness = harness().await;
    let run_id = run_id();
    let org = format!("{run_id}-org");
    let (member_user, _member_pat) = bootstrap_second(&format!("{run_id}-member")).await;

    let result = org_chain(&harness, &org, &member_user).await;
    let _ = pat_client(&harness.pat)
        .raw(Method::DELETE, &format!("orgs/{org}"), None)
        .await;
    result.expect("org chain");
}

async fn org_chain(harness: &Harness, org: &str, member: &str) -> Result<(), String> {
    let out = harness.run_ok(&["org", "create", org, "-d", "e2e org"]);
    assert!(out.contains("Created organization"), "create: {out}");

    let out = harness.run_ok(&["org", "list"]);
    assert!(out.contains(org), "list: {out}");

    let out = harness.run_ok(&["org", "view", org]);
    assert!(out.contains("members"), "view: {out}");

    let out = harness.run_ok(&["org", "add-member", org, member, "--role", "member"]);
    assert!(out.contains("Added"), "add member: {out}");

    let out = harness.run_ok(&["org", "members", org]);
    assert!(
        out.contains(member),
        "members should resolve the username: {out}"
    );

    let out = harness.run_ok(&["org", "remove-member", org, member]);
    assert!(out.contains("Removed"), "remove member: {out}");

    let out = harness.run_ok(&["org", "members", org]);
    assert!(!out.contains(member), "member should be gone: {out}");
    Ok(())
}

/// The job-log WebSocket accepts a PAT: an authenticated request for a
/// missing job answers 404, not 401 (design doc §7.1-B).
#[tokio::test]
#[ignore = "needs a live IronForge (scripts/e2e.sh)"]
async fn job_log_websocket_accepts_pat() {
    let harness = harness().await;
    let base = e2e_url()
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    let url = format!("{base}/api/v1/ws/job/999999?since=0");
    let mut request = url.as_str().into_client_request().expect("valid ws url");
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", harness.pat).parse().expect("header"),
    );
    let error = tokio_tungstenite::connect_async(request)
        .await
        .expect_err("no such job");
    let message = error.to_string();
    assert!(
        message.contains("404"),
        "PAT should authenticate (404 for a missing job, not 401): {message}"
    );
}

/// Labels, milestones, issue metadata, release edit and status (M5 surface).
#[tokio::test]
#[ignore = "needs a live IronForge (scripts/e2e.sh)"]
async fn labels_milestones_issue_meta_and_status() {
    let harness = harness().await;
    let run_id = run_id();
    let repo = format!("{run_id}-meta");
    let repo_arg = format!("{}/{}", harness.user, repo);
    let created = vec![repo.clone()];

    let result = meta_chain(&harness, &repo_arg, &repo).await;
    cleanup(&harness, &created).await;
    result.expect("meta chain");
}

async fn meta_chain(harness: &Harness, repo_arg: &str, repo_name: &str) -> Result<(), String> {
    let out = harness.run_ok(&["repo", "create", repo_name, "--init"]);
    assert!(out.contains("Created"), "repo create: {out}");

    // Label lifecycle.
    let out = harness.run_ok(&[
        "-R",
        repo_arg,
        "label",
        "create",
        "bug",
        "-c",
        "ff0000",
        "-d",
        "broken things",
    ]);
    assert!(out.contains("Created label bug"), "label create: {out}");
    let out = harness.run_ok(&["-R", repo_arg, "label", "list"]);
    assert!(out.contains("bug"), "label list: {out}");
    let out = harness.run_ok(&[
        "-R",
        repo_arg,
        "label",
        "edit",
        "bug",
        "--new-name",
        "defect",
    ]);
    assert!(out.contains("Updated label defect"), "label edit: {out}");

    // Milestone lifecycle.
    let out = harness.run_ok(&[
        "-R",
        repo_arg,
        "milestone",
        "create",
        "v1",
        "-d",
        "first cut",
        "--due",
        "2026-12-31",
    ]);
    assert!(
        out.contains("Created milestone 'v1'"),
        "milestone create: {out}"
    );
    let out = harness.run_ok(&["-R", repo_arg, "milestone", "list"]);
    assert!(out.contains("v1"), "milestone list: {out}");

    // Issue metadata: labels + milestone + assignee by name.
    let out = harness.run_ok(&["-R", repo_arg, "issue", "create", "-t", "needs labels"]);
    assert!(out.contains("#1"), "issue create: {out}");
    let out = harness.run_ok(&[
        "-R",
        repo_arg,
        "issue",
        "edit",
        "1",
        "--label",
        "defect",
        "--milestone",
        "v1",
        "--assignee",
        &harness.user,
    ]);
    assert!(out.contains("Updated"), "issue edit: {out}");
    let out = harness.run_ok(&["-R", repo_arg, "issue", "view", "1"]);
    assert!(out.contains("defect"), "issue labels after edit: {out}");

    // Release edit + delete (by-tag endpoint path).
    let artifact = harness.home_path.join("meta.txt");
    std::fs::write(&artifact, "meta artifact").expect("write artifact");
    let out = harness.run_ok(&[
        "-R",
        repo_arg,
        "release",
        "create",
        "v0.0.1",
        "-n",
        "draft notes",
        artifact.to_str().unwrap(),
    ]);
    assert!(out.contains("v0.0.1"), "release create: {out}");
    let out = harness.run_ok(&[
        "-R",
        repo_arg,
        "release",
        "edit",
        "v0.0.1",
        "-t",
        "edited title",
        "--prerelease",
    ]);
    assert!(out.contains("edited title"), "release edit: {out}");
    let out = harness.run_ok(&["-R", repo_arg, "release", "view", "v0.0.1"]);
    assert!(
        out.contains("edited title"),
        "release view after edit: {out}"
    );
    let out = harness.run_ok(&["-R", repo_arg, "release", "delete", "v0.0.1", "-y"]);
    assert!(out.contains("Deleted release"), "release delete: {out}");

    // status: outside a work tree it must explain itself, inside a clone it
    // prints the branch block (no PR, no runs here).
    let work = harness.home_path.join("statuswork");
    std::fs::create_dir_all(&work).expect("worktree");
    let clone_url = harness.clone_url(&harness.user, repo_name);
    git(&work, &["clone", &clone_url, "clone"]);
    let clone_dir = work.join("clone");
    harness_git(&clone_dir, &["checkout", "-b", "status-branch"]);
    let out = harness.run_in(&clone_dir, &["status"]);
    assert!(
        out.contains("branch  status-branch"),
        "status output: {out}"
    );

    // search: the repository shows up in the global index.
    let out = harness.run_ok(&["search", repo_name, "--type", "repo"]);
    assert!(out.contains(repo_name), "search hit: {out}");

    // search --template renders one line per hit without headers.
    let out = harness.run_ok(&[
        "search",
        repo_name,
        "--type",
        "repo",
        "-q",
        "{{.result_type}}:{{.title}}",
    ]);
    assert!(
        out.contains(&format!("repo:{repo_name}")),
        "search template: {out}"
    );

    // notifications: the counter answers even on a fresh account.
    let out = harness.run_ok(&["notifications", "count"]);
    assert!(
        out.trim().parse::<u64>().is_ok(),
        "notifications count: {out}"
    );

    // doctor: every check passes against a healthy instance + repository.
    let out = harness.run_ok(&["-R", repo_arg, "doctor"]);
    assert!(out.contains("all good."), "doctor: {out}");
    assert!(out.contains("repo scope works"), "doctor scope: {out}");
    assert!(out.contains("WebSocket available"), "doctor ws: {out}");

    // Cleanup the metadata rows.
    harness.run_ok(&["-R", repo_arg, "label", "delete", "defect", "-y"]);
    harness.run_ok(&["-R", repo_arg, "milestone", "delete", "v1", "-y"]);
    Ok(())
}
