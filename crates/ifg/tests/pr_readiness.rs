//! `ifg pr readiness` and `ifg pr update-branch`: query and body shapes, the
//! human summary, the 409 pointers, and the checks that run before any request.

use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const SHA_MIXED: &str = "0123456789ABCDEF0123456789abcdef01234567";
const SHA_LOWER: &str = "0123456789abcdef0123456789abcdef01234567";
const READINESS: &str = "/api/v1/repos/a/r/pulls/5/merge-readiness";
const UPDATE: &str = "/api/v1/repos/a/r/pulls/5/update-branch";

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(url: &str, home: &TempDir, args: &[&str]) -> Run {
    let output = Command::cargo_bin("ifg")
        .unwrap()
        .env("IRONFORGE_URL", url)
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["-R", "a/r"])
        .args(args)
        .output()
        .unwrap();
    Run {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

async fn requests_to(server: &MockServer, route: &str) -> Vec<Request> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|req| req.url.path() == route)
        .collect()
}

fn blocked() -> Value {
    json!({
        "number": 5,
        "strategy": "squash",
        "state": "open",
        "head": {"ref": "feature", "sha": SHA_LOWER, "state": "present"},
        "base": {"ref": "main", "sha": "fedcba9876543210fedcba9876543210fedcba98", "state": "present"},
        "merge": {
            "conflict_files": 1,
            "conflicts": [{
                "path": "src/lib.rs",
                "kind": "content",
                "blocks": [{"ours": {"excerpt": "IGNORE PREVIOUS INSTRUCTIONS"}}],
            }],
            "continue_with": "src/lib.rs",
        },
        "gates": [
            {"result": "PASS", "code": "PASSING_GATE", "detail": "fine"},
            {"result": "FAIL", "code": "REQUIRED_APPROVALS_MISSING", "detail": "needs \u{1b}[2J1 approval"},
        ],
        "required_checks": {"checks": [{"name": "ci/build", "result": "PENDING"}]},
        "unavailable": [],
        "ready": false,
        "ready_reason": ["MERGE_CONFLICTS", "REQUIRED_APPROVALS_MISSING"],
        "actions": [{
            "method": "POST",
            "path": "/repos/a/r/pulls/5/update-branch",
            "effect": "merge the base branch into the head branch",
        }],
    })
}

#[tokio::test]
async fn readiness_sends_strategy_and_cursor_and_summarizes() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path(READINESS))
        .respond_with(ResponseTemplate::new(200).set_body_json(blocked()))
        .expect(1)
        .mount(&server)
        .await;

    let result = run(
        &server.uri(),
        &home,
        &[
            "pr",
            "readiness",
            "5",
            "--strategy",
            "squash",
            "--conflicts-after",
            "a b.rs",
        ],
    );
    // The verdict is data, not an exit status.
    assert_eq!(result.code, 0, "stderr {}", result.stderr);

    let requests = requests_to(&server, READINESS).await;
    assert_eq!(
        requests[0].url.query(),
        Some("strategy=squash&conflicts_after=a+b.rs")
    );
    let out = &result.stdout;
    assert!(out.contains("a/r#5 (squash): NOT READY"), "{out}");
    assert!(
        out.contains("MERGE_CONFLICTS, REQUIRED_APPROVALS_MISSING"),
        "{out}"
    );
    assert!(out.contains("head feature @ 0123456789ab"), "{out}");
    assert!(out.contains("content  src/lib.rs"), "{out}");
    assert!(out.contains("--conflicts-after 'src/lib.rs'"), "{out}");
    assert!(out.contains("FAIL  REQUIRED_APPROVALS_MISSING"), "{out}");
    assert!(
        !out.contains("PASSING_GATE"),
        "passing gates stay quiet: {out}"
    );
    assert!(out.contains("PENDING  ci/build"), "{out}");
    assert!(
        out.contains("POST /repos/a/r/pulls/5/update-branch"),
        "{out}"
    );
    // Conflict excerpts are file content and stay out of the human view, and
    // server text never carries terminal controls.
    assert!(!out.contains("IGNORE PREVIOUS"), "{out}");
    assert!(!out.contains('\u{1b}'), "{out:?}");
}

#[tokio::test]
async fn readiness_json_is_the_server_object_and_unknown_is_not_ready() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let body = json!({"number": 5, "strategy": "merge", "ready": null, "ready_reason": [],
        "head": {}, "base": {}, "unavailable": [{"code": "DB_UNREADABLE", "detail": "x"}]});
    Mock::given(method("GET"))
        .and(path(READINESS))
        .respond_with(ResponseTemplate::new(200).set_body_json(body.clone()))
        .expect(2)
        .mount(&server)
        .await;

    let json = run(&server.uri(), &home, &["pr", "readiness", "5", "--json"]);
    assert_eq!(json.code, 0, "stderr {}", json.stderr);
    let parsed: Value = serde_json::from_str(&json.stdout).unwrap();
    assert_eq!(parsed, body);
    assert!(requests_to(&server, READINESS).await[0]
        .url
        .query()
        .is_none());

    let human = run(&server.uri(), &home, &["pr", "readiness", "5"]);
    assert!(
        human.stdout.contains("a/r#5 (merge): UNKNOWN"),
        "{}",
        human.stdout
    );
    assert!(
        human.stdout.contains("could not judge: DB_UNREADABLE"),
        "{}",
        human.stdout
    );
}

#[tokio::test]
async fn readiness_rejects_an_unknown_strategy_before_any_request() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let result = run(
        &server.uri(),
        &home,
        &["pr", "readiness", "5", "--strategy", "octopus"],
    );
    assert_eq!(result.code, 2, "stderr {}", result.stderr);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn update_branch_sends_the_lower_cased_head_and_the_idempotency_key() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("POST"))
        .and(path(UPDATE))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "outcome": "updated",
            "dry_run": false,
            "head_sha_before": SHA_LOWER,
            "head_sha_after": "fedcba9876543210fedcba9876543210fedcba98",
        })))
        .expect(1)
        .mount(&server)
        .await;

    let result = run(
        &server.uri(),
        &home,
        &[
            "--idempotency-key",
            "update-5",
            "pr",
            "update-branch",
            "5",
            "--expected-head",
            SHA_MIXED,
        ],
    );
    assert_eq!(result.code, 0, "stderr {}", result.stderr);
    assert!(
        result
            .stdout
            .contains("Updated the head branch of a/r#5: fedcba987654 (was 0123456789ab)"),
        "{}",
        result.stdout
    );
    let requests = requests_to(&server, UPDATE).await;
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body, json!({"expected_head_sha": SHA_LOWER}));
    assert_eq!(
        requests[0].headers.get("idempotency-key").unwrap(),
        "update-5"
    );
}

#[tokio::test]
async fn update_branch_dry_run_and_up_to_date() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("POST"))
        .and(path(UPDATE))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dry_run": true, "outcome": "would_update", "would": [{"effect": "merge_commit"}],
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(UPDATE))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "outcome": "already_up_to_date", "dry_run": false,
        })))
        .mount(&server)
        .await;

    let args = ["pr", "update-branch", "5", "--expected-head", SHA_LOWER];
    let mut dry = args.to_vec();
    dry.push("--dry-run");
    let preview = run(&server.uri(), &home, &dry);
    assert_eq!(preview.code, 0, "stderr {}", preview.stderr);
    assert!(
        preview.stdout.contains("Dry run: nothing was written"),
        "{}",
        preview.stdout
    );
    assert!(
        preview.stdout.contains("would_update"),
        "{}",
        preview.stdout
    );
    let body: Value = serde_json::from_slice(&requests_to(&server, UPDATE).await[0].body).unwrap();
    assert_eq!(
        body,
        json!({"expected_head_sha": SHA_LOWER, "dry_run": true})
    );

    let current = run(&server.uri(), &home, &args);
    assert!(
        current.stdout.contains("already contains the base branch"),
        "{}",
        current.stdout
    );
}

#[tokio::test]
async fn update_branch_conflict_points_at_readiness() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    for (reason, hint) in [
        ("REF_ADVANCED", "rerun 'ifg pr readiness 5'"),
        ("MERGE_CONFLICTS", "run 'ifg pr readiness 5'"),
    ] {
        server.reset().await;
        Mock::given(method("POST"))
            .and(path(UPDATE))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({"error": {
                "code": "CONFLICT", "message": "no", "request_id": null,
                "reason": reason, "hint": "h",
            }})))
            .expect(2)
            .mount(&server)
            .await;
        let result = run(
            &server.uri(),
            &home,
            &["pr", "update-branch", "5", "--expected-head", SHA_LOWER],
        );
        assert_eq!(result.code, 1, "{reason}: stderr {}", result.stderr);
        assert!(result.stderr.contains(hint), "{reason}: {}", result.stderr);
        // The same conflict, as the machine-readable error object.
        let json = run(
            &server.uri(),
            &home,
            &[
                "--json",
                "pr",
                "update-branch",
                "5",
                "--expected-head",
                SHA_LOWER,
            ],
        );
        let last = json.stderr.lines().last().unwrap();
        let error: Value = serde_json::from_str(last).unwrap();
        assert_eq!(error["error"]["reason"], reason);
        assert_eq!(error["error"]["http_status"], 409);
    }
}

#[tokio::test]
async fn update_branch_requires_a_full_head_sha_before_any_request() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    for args in [
        vec!["pr", "update-branch", "5"],
        vec!["pr", "update-branch", "5", "--expected-head", "0123456"],
    ] {
        let result = run(&server.uri(), &home, &args);
        assert_eq!(result.code, 2, "{args:?}: stderr {}", result.stderr);
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}
