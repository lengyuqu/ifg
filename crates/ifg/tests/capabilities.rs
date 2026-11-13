use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

const INSTANCE: &str = "/api/v1/collaboration/capabilities";
const REPO: &str = "/api/v1/repos/alice/demo/collaboration/capabilities";

fn instance_body() -> Value {
    json!({
        "contract_version": "0.2",
        "server_version": "0.9.1",
        "build_commit": "abc123def",
        "build_dirty": false,
        "api_capabilities": ["wiki.concurrent_update.v1"],
        "capabilities": [
            {"name": "wiki", "version": "1.0", "status": "stable",
             "requires": ["repo_collab_read"], "mcp_tools": ["collab_wiki_read"]},
            {"name": "collab.tasks", "version": "0.1", "status": "experimental",
             "requires": ["account"], "mcp_tools": []}
        ],
        "limits": {"page_size": 100},
        "auth_modes": [{"name": "pat", "description": "personal token", "headers": ["Authorization"]}],
        "mcp_transport": {"transports": ["stdio"], "binary": "ironforge-mcp",
                          "tool_count": 1, "discovery": "tools/list"},
        "deprecations": [],
        "future_field": {"kept": true}
    })
}

fn repo_body() -> Value {
    let mut body = instance_body();
    body["repo"] = json!("alice/demo");
    body["caller"] = json!({"repo_collab_read": true, "repo_write": false});
    body
}

fn command(server: &MockServer, home: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path());
    cmd
}

async fn mount_json(server: &MockServer, route: &str, body: Value, expect: u64) {
    Mock::given(method("GET"))
        .and(path(route))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(expect)
        .mount(server)
        .await;
}

fn stdout(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stdout).into_owned()
}

fn stderr(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stderr).into_owned()
}

#[tokio::test]
async fn without_repo_flag_uses_the_instance_endpoint_only() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_json(&server, INSTANCE, instance_body(), 1).await;
    mount_json(&server, REPO, repo_body(), 0).await;
    command(&server, &home)
        .args(["capabilities", "--json"])
        .assert()
        .success();
}

#[tokio::test]
async fn inside_a_checkout_without_r_still_uses_the_instance_endpoint() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let checkout = TempDir::new().unwrap();
    for args in [
        vec!["init", "-q"],
        vec![
            "remote",
            "add",
            "origin",
            "https://forge.invalid/git/alice/demo",
        ],
    ] {
        let status = std::process::Command::new("git")
            .args(&args)
            .current_dir(checkout.path())
            .status()
            .unwrap();
        assert!(status.success());
    }
    mount_json(&server, INSTANCE, instance_body(), 1).await;
    mount_json(&server, REPO, repo_body(), 0).await;
    command(&server, &home)
        .current_dir(checkout.path())
        .args(["capabilities", "--json"])
        .assert()
        .success();
}

#[tokio::test]
async fn repo_flag_uses_the_repository_endpoint() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_json(&server, INSTANCE, instance_body(), 0).await;
    mount_json(&server, REPO, repo_body(), 1).await;
    let assert = command(&server, &home)
        .args(["-R", "alice/demo", "capabilities", "--json"])
        .assert()
        .success();
    let value: Value = serde_json::from_str(&stdout(&assert)).unwrap();
    assert_eq!(value, repo_body());
}

#[tokio::test]
async fn json_prints_the_server_response_unchanged() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_json(&server, INSTANCE, instance_body(), 1).await;
    let assert = command(&server, &home)
        .args(["capabilities", "--json"])
        .assert()
        .success();
    let value: Value = serde_json::from_str(&stdout(&assert)).unwrap();
    assert_eq!(value, instance_body());
    assert_eq!(value["future_field"]["kept"], json!(true));
}

#[tokio::test]
async fn template_reads_the_same_object() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_json(&server, INSTANCE, instance_body(), 1).await;
    let assert = command(&server, &home)
        .args([
            "capabilities",
            "-q",
            "{{.server_version}} {{.contract_version}}",
        ])
        .assert()
        .success();
    assert_eq!(stdout(&assert).trim(), "0.9.1 0.2");
}

#[tokio::test]
async fn human_output_shows_versions_and_a_capability_table() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_json(&server, INSTANCE, instance_body(), 1).await;
    let assert = command(&server, &home)
        .arg("capabilities")
        .assert()
        .success();
    let out = stdout(&assert);
    assert!(out.contains("server:") && out.contains("0.9.1"), "{out}");
    assert!(out.contains("contract:") && out.contains("0.2"), "{out}");
    assert!(out.contains("build:") && out.contains("abc123def"), "{out}");
    assert!(out.contains("wiki.concurrent_update.v1"), "{out}");
    for header in ["NAME", "VERSION", "STATUS"] {
        assert!(out.contains(header), "{out}");
    }
    assert!(out.contains("collab.tasks"), "{out}");
    assert!(out.contains("experimental"), "{out}");
    assert!(
        !out.contains("repository:"),
        "instance output has no repo: {out}"
    );
}

#[tokio::test]
async fn repo_human_output_reports_the_callers_access() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_json(&server, REPO, repo_body(), 1).await;
    let assert = command(&server, &home)
        .args(["-R", "alice/demo", "capabilities"])
        .assert()
        .success();
    let out = stdout(&assert);
    assert!(
        out.contains("repository:") && out.contains("alice/demo"),
        "{out}"
    );
    assert!(out.contains("repo_collab_read yes, repo_write no"), "{out}");
}

#[tokio::test]
async fn human_output_strips_terminal_controls_from_server_text() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let mut body = instance_body();
    body["capabilities"][0]["name"] = json!("wiki\u{1b}[2J.evil");
    mount_json(&server, INSTANCE, body, 1).await;
    let assert = command(&server, &home)
        .arg("capabilities")
        .assert()
        .success();
    let out = stdout(&assert);
    assert!(!out.contains('\u{1b}'), "{out:?}");
    assert!(out.contains("wiki[2J.evil"), "{out}");
}

#[tokio::test]
async fn has_present_exits_zero_with_empty_stdout() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_json(&server, INSTANCE, instance_body(), 1).await;
    let assert = command(&server, &home)
        .args(["capabilities", "--has", "wiki"])
        .assert()
        .success();
    assert_eq!(stdout(&assert), "");
    assert_eq!(stderr(&assert), "");
}

#[tokio::test]
async fn has_absent_exits_one_with_a_one_line_message() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_json(&server, INSTANCE, instance_body(), 1).await;
    let assert = command(&server, &home)
        .args(["capabilities", "--has", "packages"])
        .assert()
        .code(1);
    assert_eq!(stdout(&assert), "");
    let err = stderr(&assert);
    assert_eq!(err.lines().count(), 1, "{err}");
    assert!(
        err.contains("does not advertise capability `packages`"),
        "{err}"
    );
}

#[tokio::test]
async fn has_matches_names_exactly_not_api_behaviours_or_prefixes() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    // `wiki.concurrent_update.v1` is an api behaviour, not a capability name.
    mount_json(&server, INSTANCE, instance_body(), 2).await;
    for name in ["wiki.concurrent_update.v1", "wik"] {
        let assert = command(&server, &home)
            .args(["capabilities", "--has", name])
            .assert()
            .code(1);
        assert_eq!(stdout(&assert), "", "{name}");
    }
}

#[tokio::test]
async fn has_with_json_still_prints_nothing_on_stdout() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_json(&server, INSTANCE, instance_body(), 1).await;
    let assert = command(&server, &home)
        .args(["capabilities", "--json", "--has", "wiki"])
        .assert()
        .success();
    assert_eq!(stdout(&assert), "");
}

#[tokio::test]
async fn old_server_without_the_endpoint_fails_with_not_found() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path(INSTANCE))
        .respond_with(ResponseTemplate::new(404).set_body_string("404 page not found"))
        .expect(2)
        .mount(&server)
        .await;
    let assert = command(&server, &home)
        .args(["capabilities"])
        .assert()
        .code(8);
    assert_eq!(stdout(&assert), "");
    let err = stderr(&assert);
    assert!(err.contains("does not expose capabilities"), "{err}");

    // An empty list would look like success to a script, so `--has` fails too.
    let assert = command(&server, &home)
        .args(["capabilities", "--has", "wiki"])
        .assert()
        .code(8);
    assert_eq!(stdout(&assert), "");
}

#[tokio::test]
async fn old_server_404_under_json_is_an_error_not_an_empty_list() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path(INSTANCE))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(
                json!({"error": {"code": "NOT_FOUND", "message": "route not found"}}),
            ),
        )
        .expect(1)
        .mount(&server)
        .await;
    let assert = command(&server, &home)
        .args(["capabilities", "--json"])
        .assert()
        .code(8);
    assert_eq!(stdout(&assert), "");
    assert!(stderr(&assert).contains("does not expose capabilities"));
}

#[tokio::test]
async fn repo_404_mentions_the_repository_and_capabilities() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path(REPO))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let assert = command(&server, &home)
        .args(["-R", "alice/demo", "capabilities"])
        .assert()
        .code(8);
    let err = stderr(&assert);
    assert!(
        err.contains("does not expose capabilities for alice/demo"),
        "{err}"
    );
}

#[tokio::test]
async fn every_run_makes_exactly_one_request_and_writes_nothing() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_json(&server, INSTANCE, instance_body(), 2).await;
    for run in 1..=2 {
        command(&server, &home)
            .args(["capabilities", "--json"])
            .assert()
            .success();
        assert_eq!(server.received_requests().await.unwrap().len(), run);
    }
    assert!(
        std::fs::read_dir(home.path()).unwrap().next().is_none(),
        "capabilities must not write to disk"
    );
}
