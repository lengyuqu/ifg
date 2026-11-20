//! `ifg agent`, `ifg draft` and `ifg notifications prefs`: request shapes, the
//! checks made before a request, human views with sanitized server text, the
//! token secret staying out of every output, and confirmation handling.

use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SECRET: &str = "ifp_supersecretvalue0123456789abcdef";

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(server: &MockServer, args: &[&str]) -> Run {
    let home = TempDir::new().unwrap();
    let output = Command::cargo_bin("ifg")
        .unwrap()
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .args(args)
        .output()
        .unwrap();
    Run {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

async fn mount(server: &MockServer, verb: &str, route: &str, status: u16, body: Value) {
    Mock::given(method(verb))
        .and(path(route))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

fn agent() -> Value {
    json!({
        "id": 7, "username": "alice-agent-bot", "kind": "agent",
        "owner_user_id": 1, "description": "does \u{1b}[2Jthings",
        "is_active": true, "created_at": "2026-11-19T10:00:00Z"
    })
}

fn token_reply() -> Value {
    json!({
        "id": 42, "name": "ci", "token": SECRET, "scopes": "repo:write",
        "repos": ["alice/demo"], "refs": ["agent/*"],
        "expires_at": null, "created_at": "2026-11-19T10:00:00Z"
    })
}

fn bodies(requests: &[wiremock::Request]) -> Vec<Value> {
    requests
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap_or(Value::Null))
        .collect()
}

#[tokio::test]
async fn agent_list_and_view_sanitize_and_pass_json_through() {
    let server = MockServer::start().await;
    mount(&server, "GET", "/api/v1/agents", 200, json!([agent()])).await;
    mount(
        &server,
        "GET",
        "/api/v1/agents/alice-agent-bot",
        200,
        agent(),
    )
    .await;

    let list = run(&server, &["agent", "list"]);
    assert_eq!(list.code, 0, "{}", list.stderr);
    assert!(list.stdout.contains("alice-agent-bot"));
    assert!(!list.stdout.contains('\u{1b}'), "{:?}", list.stdout);
    let json_list = run(&server, &["agent", "list", "--json"]);
    assert_eq!(
        serde_json::from_str::<Value>(&json_list.stdout).unwrap(),
        json!([agent()])
    );
    let view = run(&server, &["agent", "view", "alice-agent-bot"]);
    assert!(view.stdout.contains("- active: yes"), "{}", view.stdout);
    assert!(!view.stdout.contains('\u{1b}'));
}

#[tokio::test]
async fn agent_create_sends_name_and_description() {
    let server = MockServer::start().await;
    mount(&server, "POST", "/api/v1/agents", 201, agent()).await;
    let plain = run(&server, &["agent", "create", "bot"]);
    assert_eq!(plain.code, 0, "{}", plain.stderr);
    assert!(plain.stdout.contains("Created agent alice-agent-bot."));
    let described = run(&server, &["agent", "create", "bot", "--description", "d"]);
    assert_eq!(described.code, 0);
    let sent = bodies(&server.received_requests().await.unwrap());
    assert_eq!(sent[0], json!({"name": "bot"}));
    assert_eq!(sent[1], json!({"name": "bot", "description": "d"}));
}

#[tokio::test]
async fn agent_edit_sends_only_what_was_asked_and_needs_a_change() {
    let server = MockServer::start().await;
    mount(
        &server,
        "PATCH",
        "/api/v1/agents/alice-agent-bot",
        200,
        agent(),
    )
    .await;
    for args in [
        vec!["agent", "edit", "alice-agent-bot", "--deactivate"],
        vec!["agent", "edit", "alice-agent-bot", "--activate"],
        vec!["agent", "edit", "alice-agent-bot", "--description", ""],
    ] {
        assert_eq!(run(&server, &args).code, 0, "{args:?}");
    }
    let sent = bodies(&server.received_requests().await.unwrap());
    assert_eq!(sent[0], json!({"is_active": false}));
    assert_eq!(sent[1], json!({"is_active": true}));
    assert_eq!(sent[2], json!({"description": ""}), "empty clears");

    for args in [
        vec!["agent", "edit", "alice-agent-bot"],
        vec!["agent", "edit", "x", "--activate", "--deactivate"],
    ] {
        assert_eq!(run(&server, &args).code, 2, "{args:?}");
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
}

/// Without owner-only file permissions (non-unix) the secret is never written
/// anywhere: the command refuses before any request, so no token is created.
#[cfg(not(unix))]
#[tokio::test]
async fn token_create_is_refused_where_private_files_are_unsupported() {
    let server = MockServer::start().await;
    mount(
        &server,
        "POST",
        "/api/v1/agents/alice-agent-bot/tokens",
        201,
        token_reply(),
    )
    .await;
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("tok");
    let result = run(
        &server,
        &[
            "agent",
            "token",
            "create",
            "alice-agent-bot",
            "--name",
            "ci",
            "--token-file",
            file.to_str().unwrap(),
        ],
    );
    assert_ne!(result.code, 0);
    assert!(
        result.stderr.contains("not yet supported on this platform"),
        "{}",
        result.stderr
    );
    assert!(!result.stdout.contains(SECRET) && !result.stderr.contains(SECRET));
    assert!(!file.exists());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn token_create_saves_the_secret_privately_and_never_prints_it() {
    let server = MockServer::start().await;
    mount(
        &server,
        "POST",
        "/api/v1/agents/alice-agent-bot/tokens",
        201,
        token_reply(),
    )
    .await;
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("tok");
    let file_arg = file.to_str().unwrap();
    let args = [
        "agent",
        "token",
        "create",
        "alice-agent-bot",
        "--name",
        "ci",
        "--allow-repo",
        "alice/demo",
        "--allow-ref",
        "agent/*",
        "--token-file",
        file_arg,
    ];

    let plain = run(&server, &args);
    assert_eq!(plain.code, 0, "{}", plain.stderr);
    assert!(!plain.stdout.contains(SECRET) && !plain.stderr.contains(SECRET));
    assert!(
        plain.stdout.contains("Created token 42 (ci)"),
        "{}",
        plain.stdout
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap().trim(), SECRET);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let sent = bodies(&server.received_requests().await.unwrap());
    assert_eq!(
        sent[0],
        json!({"name": "ci", "repos": ["alice/demo"], "refs": ["agent/*"]})
    );

    // The file exists now: a second run is refused before any request.
    let again = run(&server, &args);
    assert_ne!(again.code, 0);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert_eq!(std::fs::read_to_string(&file).unwrap().trim(), SECRET);

    // --json drops the secret and names the file instead.
    let other = dir.path().join("tok2");
    let other_arg = other.to_str().unwrap();
    let json_run = run(
        &server,
        &[
            "agent",
            "token",
            "create",
            "alice-agent-bot",
            "--name",
            "ci",
            "--token-file",
            other_arg,
            "--json",
        ],
    );
    assert_eq!(json_run.code, 0, "{}", json_run.stderr);
    assert!(!json_run.stdout.contains(SECRET));
    let value: Value = serde_json::from_str(&json_run.stdout).unwrap();
    assert!(value.get("token").is_none());
    assert_eq!(value["token_file"], other_arg);
    assert_eq!(value["id"], 42);
}

#[tokio::test]
async fn token_create_failure_leaves_no_file_behind() {
    let server = MockServer::start().await;
    mount(
        &server,
        "POST",
        "/api/v1/agents/alice-agent-bot/tokens",
        409,
        json!({"message": "the agent is deactivated"}),
    )
    .await;
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("tok");
    let result = run(
        &server,
        &[
            "agent",
            "token",
            "create",
            "alice-agent-bot",
            "--name",
            "ci",
            "--token-file",
            file.to_str().unwrap(),
        ],
    );
    assert_ne!(result.code, 0);
    assert!(
        !file.exists(),
        "a refused request must not leave an empty token file"
    );
}

#[tokio::test]
async fn token_create_flag_rules_are_checked_before_any_request() {
    let server = MockServer::start().await;
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("tok");
    for args in [
        // --allow-ref needs --allow-repo
        vec![
            "agent",
            "token",
            "create",
            "a",
            "--name",
            "n",
            "--allow-ref",
            "x/*",
            "--token-file",
            file.to_str().unwrap(),
        ],
        // the secret must go somewhere
        vec!["agent", "token", "create", "a", "--name", "n"],
    ] {
        assert_eq!(run(&server, &args).code, 2, "{args:?}");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
    assert!(!file.exists());
}

#[tokio::test]
async fn token_list_shows_metadata_and_revoke_needs_confirmation() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/api/v1/agents/alice-agent-bot/tokens",
        200,
        json!([{"id": 42, "name": "ci", "scopes": "repo:write", "repos": ["alice/demo"],
                "expires_at": null, "last_used_at": null, "created_at": "2026-11-19T10:00:00Z"}]),
    )
    .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/agents/alice-agent-bot/tokens/42"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let list = run(&server, &["agent", "token", "list", "alice-agent-bot"]);
    assert_eq!(list.code, 0, "{}", list.stderr);
    assert!(list.stdout.contains("alice/demo") && list.stdout.contains("any"));

    // Not a terminal, no --yes: refused, nothing deleted.
    let refused = run(
        &server,
        &["agent", "token", "revoke", "alice-agent-bot", "42"],
    );
    assert_eq!(refused.code, 2, "{}", refused.stderr);
    let revoked = run(
        &server,
        &["agent", "token", "revoke", "alice-agent-bot", "42", "--yes"],
    );
    assert_eq!(revoked.code, 0, "{}", revoked.stderr);
    assert!(revoked.stdout.contains("Revoked token 42"));
}

fn draft(state: &str) -> Value {
    json!({
        "id": 3, "repo_id": 1, "prefix": "agent/7/\u{1b}[2J", "owner_user_id": 1,
        "ttl_secs": 7200, "expires_at": "2026-11-19T12:00:00Z",
        "delete_due_at": "2026-11-20T12:00:00Z", "grace_secs": 86400,
        "state": state, "extended_count": 0, "created_at": "2026-11-19T10:00:00Z"
    })
}

#[tokio::test]
async fn draft_commands_send_seconds_and_print_deadlines() {
    let server = MockServer::start().await;
    mount(
        &server,
        "POST",
        "/api/v1/repos/a/r/agent-drafts",
        201,
        draft("active"),
    )
    .await;
    mount(
        &server,
        "GET",
        "/api/v1/repos/a/r/agent-drafts",
        200,
        json!([draft("active")]),
    )
    .await;
    mount(
        &server,
        "PATCH",
        "/api/v1/repos/a/r/agent-drafts/3",
        200,
        draft("active"),
    )
    .await;
    mount(
        &server,
        "DELETE",
        "/api/v1/repos/a/r/agent-drafts/3",
        202,
        draft("expiring"),
    )
    .await;

    let created = run(
        &server,
        &[
            "-R", "a/r", "draft", "create", "agent/7/", "--ttl", "2h", "--grace", "90m",
        ],
    );
    assert_eq!(created.code, 0, "{}", created.stderr);
    assert!(created
        .stdout
        .contains("pushes accepted until: 2026-11-19T12:00:00Z"));
    assert!(!created.stdout.contains('\u{1b}'));
    let plain = run(
        &server,
        &["-R", "a/r", "draft", "create", "agent/7/", "--ttl", "3600"],
    );
    assert_eq!(plain.code, 0);
    let extended = run(
        &server,
        &["-R", "a/r", "draft", "extend", "3", "--by", "1d"],
    );
    assert_eq!(extended.code, 0, "{}", extended.stderr);
    let listed = run(&server, &["-R", "a/r", "draft", "list"]);
    assert!(listed.stdout.contains("active"), "{}", listed.stdout);
    let json_run = run(&server, &["-R", "a/r", "draft", "list", "--json"]);
    assert_eq!(json_run.code, 0);
    let refused = run(&server, &["-R", "a/r", "draft", "close", "3"]);
    assert_eq!(refused.code, 2, "{}", refused.stderr);
    let closed = run(&server, &["-R", "a/r", "draft", "close", "3", "--yes"]);
    assert_eq!(closed.code, 0, "{}", closed.stderr);
    assert!(closed.stdout.contains("branches are deleted at"));

    let sent = bodies(&server.received_requests().await.unwrap());
    assert_eq!(
        sent[0],
        json!({"prefix": "agent/7/", "ttl_secs": 7200, "grace_secs": 5400})
    );
    assert_eq!(sent[1], json!({"prefix": "agent/7/", "ttl_secs": 3600}));
    assert_eq!(sent[2], json!({"ttl_secs": 86400}));
}

#[tokio::test]
async fn draft_durations_are_validated_before_any_request() {
    let server = MockServer::start().await;
    for args in [
        vec!["draft", "create", "agent/7/", "--ttl", "59s"],
        vec!["draft", "create", "agent/7/", "--ttl", "31d"],
        vec!["draft", "create", "agent/7/", "--ttl", "soon"],
        vec![
            "draft", "create", "agent/7/", "--ttl", "1h", "--grace", "-5m",
        ],
        vec!["draft", "extend", "3", "--by", "10s"],
        vec!["draft", "create", "agent/7/"],
    ] {
        let mut full = vec!["-R", "a/r"];
        full.extend(args.iter());
        assert_eq!(run(&server, &full).code, 2, "{args:?}");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

fn prefs() -> Value {
    json!({
        "default_enabled": true,
        "preferences": [
            {"repo_id": null, "event_type": "push", "channel": "email", "enabled": false, "explicit": true},
            {"repo_id": 9, "event_type": "push", "channel": "email", "enabled": true, "explicit": true},
            {"repo_id": null, "event_type": "issue_assigned", "channel": "in_app", "enabled": true, "explicit": false}
        ]
    })
}

#[tokio::test]
async fn prefs_list_shows_scope_state_and_source() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/api/v1/user/notification-preferences",
        200,
        prefs(),
    )
    .await;
    let result = run(&server, &["notifications", "prefs", "list"]);
    assert_eq!(result.code, 0, "{}", result.stderr);
    assert!(
        result.stdout.contains("have not changed are on"),
        "{}",
        result.stdout
    );
    assert!(result.stdout.contains("repo #9"));
    assert!(result.stdout.contains("default"));
    let json_run = run(&server, &["notifications", "prefs", "list", "--json"]);
    assert_eq!(
        serde_json::from_str::<Value>(&json_run.stdout).unwrap(),
        prefs()
    );
}

#[tokio::test]
async fn prefs_set_is_global_unless_dash_r_is_given_and_checks_the_event_name() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/api/v1/user/notification-preferences",
        200,
        prefs(),
    )
    .await;
    mount(
        &server,
        "PUT",
        "/api/v1/user/notification-preferences",
        200,
        prefs(),
    )
    .await;
    mount(
        &server,
        "GET",
        "/api/v1/repos/a/r",
        200,
        json!({"id": 9, "owner_id": 1, "name": "r", "is_private": false, "default_branch": "main"}),
    )
    .await;

    let global = run(
        &server,
        &[
            "notifications",
            "prefs",
            "set",
            "--event",
            "push",
            "--channel",
            "email",
            "--off",
        ],
    );
    assert_eq!(global.code, 0, "{}", global.stderr);
    assert!(global
        .stdout
        .contains("Turned off push for email: all repositories."));
    let scoped = run(
        &server,
        &[
            "-R",
            "a/r",
            "notifications",
            "prefs",
            "set",
            "--event",
            "push",
            "--channel",
            "email",
            "--on",
        ],
    );
    assert_eq!(scoped.code, 0, "{}", scoped.stderr);
    assert!(scoped.stdout.contains("repository a/r"));

    let puts: Vec<Value> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "PUT")
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    assert_eq!(
        puts[0],
        json!([{"repo_id": null, "event_type": "push", "channel": "email", "enabled": false}])
    );
    assert_eq!(
        puts[1],
        json!([{"repo_id": 9, "event_type": "push", "channel": "email", "enabled": true}])
    );

    let before = server.received_requests().await.unwrap().len();
    let typo = run(
        &server,
        &[
            "notifications",
            "prefs",
            "set",
            "--event",
            "pussh",
            "--channel",
            "email",
            "--off",
        ],
    );
    assert_eq!(typo.code, 2, "{}", typo.stderr);
    assert!(
        typo.stderr.contains("known types: push, issue_assigned"),
        "{}",
        typo.stderr
    );
    let after = server.received_requests().await.unwrap();
    assert!(
        after[before..].iter().all(|r| r.method.as_str() != "PUT"),
        "a misspelled event must never be stored"
    );
}

#[tokio::test]
async fn prefs_set_needs_exactly_one_state_and_a_known_channel() {
    let server = MockServer::start().await;
    for args in [
        vec![
            "notifications",
            "prefs",
            "set",
            "--event",
            "push",
            "--channel",
            "email",
        ],
        vec![
            "notifications",
            "prefs",
            "set",
            "--event",
            "push",
            "--channel",
            "email",
            "--on",
            "--off",
        ],
        vec![
            "notifications",
            "prefs",
            "set",
            "--event",
            "push",
            "--channel",
            "sms",
            "--on",
        ],
    ] {
        assert_eq!(run(&server, &args).code, 2, "{args:?}");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}
