//! `ifg mcp doctor`: the checks run in order and print `doctor`-style items.
//! A missing stdio declaration, a tool-count mismatch and an unset
//! `IRONFORGE_PAT` only warn. A server too old to declare MCP support, a
//! missing server binary and a rejected host fail the run. The token never
//! reaches stdout or stderr.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

const TOKEN: &str = "ifp_mcpdoctorsecretvalue0123456789abcdef";
const CAPABILITIES: &str = "/api/v1/collaboration/capabilities";
const CHECKS_WHEN_COMPLETE: [&str; 9] = [
    "host",
    "token",
    "server",
    "account",
    "capabilities",
    "transport",
    "tools",
    "binary",
    "IRONFORGE_PAT",
];

fn ifg(home: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("XDG_CONFIG_HOME", home.path())
        .env_remove("IRONFORGE_URL")
        .env_remove("IRONFORGE_PAT");
    cmd
}

/// `ifg mcp doctor` against `server`, with the token in the environment.
fn doctor(server: &MockServer, home: &TempDir, extra: &[&str]) -> Command {
    let mut cmd = ifg(home);
    cmd.env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", TOKEN)
        .args(["mcp", "doctor"])
        .args(extra);
    cmd
}

/// A file the tests never run: it only has to exist, with an executable bit
/// where the platform has one.
fn fake_binary(dir: &Path, name: &str) -> PathBuf {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let path = dir.join(file);
    std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

async fn mount(server: &MockServer, route: &str, status: u16, body: Value) {
    Mock::given(method("GET"))
        .and(path(route))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

/// A server that declares MCP support: two tools, both named by the
/// capabilities list, and stdio as its only transport.
fn capabilities_body() -> Value {
    json!({
        "contract_version": "0.2",
        "server_version": "0.9.1",
        "api_capabilities": [],
        "capabilities": [
            {"name": "collab.wiki", "version": "1.0", "status": "stable",
             "requires": ["repo_collab_read"],
             "mcp_tools": ["collab_wiki_read", "collab_wiki_write"]},
            {"name": "collab.tasks", "version": "0.1", "status": "experimental",
             "requires": ["account"], "mcp_tools": []}
        ],
        "limits": {},
        "auth_modes": [],
        "mcp_transport": {"transports": ["stdio"], "binary": "ironforge-mcp",
                          "tool_count": 2, "discovery": "tools/list"},
        "deprecations": []
    })
}

/// Health and account routes, plus capabilities with `edit` applied.
async fn server_with(edit: impl FnOnce(&mut Value)) -> MockServer {
    let server = MockServer::start().await;
    mount(
        &server,
        "/health",
        200,
        json!({"version": "0.9.1", "phase": 3}),
    )
    .await;
    mount(
        &server,
        "/api/v1/users/me",
        200,
        json!({"id": 1, "username": "alice"}),
    )
    .await;
    let mut body = capabilities_body();
    edit(&mut body);
    mount(&server, CAPABILITIES, 200, body).await;
    server
}

/// Health and account routes only: the capabilities route answers 404.
async fn server_without_capabilities() -> MockServer {
    let server = MockServer::start().await;
    mount(
        &server,
        "/health",
        200,
        json!({"version": "0.9.1", "phase": 3}),
    )
    .await;
    mount(
        &server,
        "/api/v1/users/me",
        200,
        json!({"id": 1, "username": "alice"}),
    )
    .await;
    mount(
        &server,
        CAPABILITIES,
        404,
        json!({"error": {"message": "not found"}}),
    )
    .await;
    server
}

/// A config file whose default host carries [`TOKEN`] (no environment token).
fn home_with_stored_token(url: &str) -> TempDir {
    let home = TempDir::new().unwrap();
    let dir = home.path().join("ironforge");
    std::fs::create_dir(&dir).unwrap();
    let key = url.trim_start_matches("http://");
    let file = dir.join("hosts.toml");
    std::fs::write(
        &file,
        format!(
            "default_host='{key}'\n[hosts.'{key}']\nurl='{url}'\nuser='alice'\ntoken='{TOKEN}'\n"
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    home
}

fn items(output: &std::process::Output) -> Vec<Value> {
    let value: Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be exactly one JSON array");
    value.as_array().expect("array of check items").clone()
}

fn find<'a>(items: &'a [Value], name: &str) -> Option<&'a Value> {
    items.iter().find(|item| item["check"] == name)
}

fn check_names(items: &[Value]) -> Vec<&str> {
    items
        .iter()
        .map(|item| item["check"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn every_check_passes_when_the_setup_is_complete() {
    let server = server_with(|_| {}).await;
    let home = TempDir::new().unwrap();
    let bin = TempDir::new().unwrap();
    let binary = fake_binary(bin.path(), "ironforge-mcp");

    let assert = doctor(
        &server,
        &home,
        &["--json", "--command", binary.to_str().unwrap()],
    )
    .assert()
    .success();
    let items = items(assert.get_output());

    assert_eq!(check_names(&items), CHECKS_WHEN_COMPLETE);
    assert!(items.iter().all(|item| item["status"] == "ok"), "{items:?}");
    assert!(find(&items, "binary").unwrap()["detail"]
        .as_str()
        .unwrap()
        .contains("--command"));
}

#[tokio::test]
async fn binary_declared_by_the_server_is_looked_up_on_path() {
    let server = server_with(|_| {}).await;
    let home = TempDir::new().unwrap();
    let bin = TempDir::new().unwrap();
    fake_binary(bin.path(), "ironforge-mcp");

    let assert = doctor(&server, &home, &["--json"])
        .env("PATH", bin.path())
        .assert()
        .success();
    let items = items(assert.get_output());
    let binary = find(&items, "binary").unwrap();
    assert_eq!(binary["status"], "ok", "{binary}");
    assert!(binary["detail"]
        .as_str()
        .unwrap()
        .contains("declared by the server"));
}

#[tokio::test]
async fn server_without_capabilities_is_too_old_and_skips_transport_and_tools() {
    let server = server_without_capabilities().await;
    let home = TempDir::new().unwrap();
    let bin = TempDir::new().unwrap();
    let binary = fake_binary(bin.path(), "ironforge-mcp");

    let assert = doctor(
        &server,
        &home,
        &["--json", "--command", binary.to_str().unwrap()],
    )
    .assert()
    .code(1);
    let items = items(assert.get_output());

    let capabilities = find(&items, "capabilities").unwrap();
    assert_eq!(capabilities["status"], "fail");
    assert!(capabilities["hint"].as_str().unwrap().contains("too old"));
    assert!(find(&items, "transport").is_none(), "{items:?}");
    assert!(find(&items, "tools").is_none(), "{items:?}");
    // The binary and environment checks do not depend on the server.
    assert_eq!(find(&items, "binary").unwrap()["status"], "ok");
    assert_eq!(find(&items, "IRONFORGE_PAT").unwrap()["status"], "ok");
}

#[tokio::test]
async fn missing_stdio_is_a_warning_that_shows_what_the_server_declares() {
    let server = server_with(|body| {
        body["mcp_transport"]["transports"] = json!(["http"]);
    })
    .await;
    let home = TempDir::new().unwrap();
    let bin = TempDir::new().unwrap();
    let binary = fake_binary(bin.path(), "ironforge-mcp");

    let assert = doctor(
        &server,
        &home,
        &["--json", "--command", binary.to_str().unwrap()],
    )
    .assert()
    .success();
    let items = items(assert.get_output());
    let transport = find(&items, "transport").unwrap();
    assert_eq!(transport["status"], "warn");
    assert!(transport["detail"].as_str().unwrap().contains("http"));
    assert!(transport["hint"].is_string());
}

#[tokio::test]
async fn tool_count_mismatch_is_a_warning_and_keeps_the_exit_code() {
    let server = server_with(|body| {
        body["mcp_transport"]["tool_count"] = json!(5);
    })
    .await;
    let home = TempDir::new().unwrap();
    let bin = TempDir::new().unwrap();
    let binary = fake_binary(bin.path(), "ironforge-mcp");

    let assert = doctor(
        &server,
        &home,
        &["--json", "--command", binary.to_str().unwrap()],
    )
    .assert()
    .success();
    let items = items(assert.get_output());
    let tools = find(&items, "tools").unwrap();
    assert_eq!(tools["status"], "warn");
    assert!(tools["detail"].as_str().unwrap().contains("tools/list"));
    assert!(
        items.iter().all(|item| item["status"] != "fail"),
        "{items:?}"
    );
}

#[tokio::test]
async fn missing_binary_fails_with_an_install_hint() {
    let server = server_with(|_| {}).await;
    let home = TempDir::new().unwrap();
    let bin = TempDir::new().unwrap();
    let missing = bin.path().join("no-such-ironforge-mcp");

    let assert = doctor(
        &server,
        &home,
        &["--json", "--command", missing.to_str().unwrap()],
    )
    .assert()
    .code(1);
    let items = items(assert.get_output());
    let binary = find(&items, "binary").unwrap();
    assert_eq!(binary["status"], "fail");
    assert!(binary["hint"].as_str().unwrap().contains("--command"));

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(stderr.contains("mcp doctor check(s) failed"), "{stderr}");
}

#[tokio::test]
async fn declared_binary_absent_from_path_fails() {
    let server = server_with(|_| {}).await;
    let home = TempDir::new().unwrap();
    let empty = TempDir::new().unwrap();

    let assert = doctor(&server, &home, &["--json"])
        .env("PATH", empty.path())
        .assert()
        .code(1);
    let items = items(assert.get_output());
    let binary = find(&items, "binary").unwrap();
    assert_eq!(binary["status"], "fail");
    assert!(binary["detail"]
        .as_str()
        .unwrap()
        .contains("declared by the server"));
}

#[cfg(unix)]
#[tokio::test]
async fn binary_without_an_executable_bit_fails() {
    let server = server_with(|_| {}).await;
    let home = TempDir::new().unwrap();
    let bin = TempDir::new().unwrap();
    let file = bin.path().join("ironforge-mcp");
    std::fs::write(&file, "not executable\n").unwrap();

    let assert = doctor(
        &server,
        &home,
        &["--json", "--command", file.to_str().unwrap()],
    )
    .assert()
    .code(1);
    let items = items(assert.get_output());
    let binary = find(&items, "binary").unwrap();
    assert_eq!(binary["status"], "fail");
    assert!(binary["detail"]
        .as_str()
        .unwrap()
        .contains("not executable"));
}

#[tokio::test]
async fn unset_pat_warns_with_the_export_hint() {
    let server = server_with(|_| {}).await;
    // The token comes from the config file, so IRONFORGE_PAT is genuinely unset.
    let home = home_with_stored_token(&server.uri());
    let bin = TempDir::new().unwrap();
    let binary = fake_binary(bin.path(), "ironforge-mcp");

    let assert = ifg(&home)
        .args([
            "mcp",
            "doctor",
            "--json",
            "--command",
            binary.to_str().unwrap(),
        ])
        .assert()
        .success();
    let items = items(assert.get_output());
    let pat = find(&items, "IRONFORGE_PAT").unwrap();
    assert_eq!(pat["status"], "warn");
    #[cfg(windows)]
    let expected = "$env:IRONFORGE_PAT = (ifg auth token)";
    #[cfg(not(windows))]
    let expected = "export IRONFORGE_PAT=\"$(ifg auth token)\"";
    assert_eq!(pat["hint"], expected);
}

#[tokio::test]
async fn token_value_never_reaches_stdout_or_stderr() {
    let bin = TempDir::new().unwrap();
    let binary = fake_binary(bin.path(), "ironforge-mcp");
    let binary = binary.to_str().unwrap();
    let home = TempDir::new().unwrap();

    let complete = server_with(|_| {}).await;
    let outcomes = [
        doctor(&complete, &home, &["--json", "--command", binary]).assert(),
        doctor(&complete, &home, &["--command", binary]).assert(),
        doctor(
            &server_without_capabilities().await,
            &home,
            &["--json", "--command", binary],
        )
        .assert(),
        doctor(
            &server_without_capabilities().await,
            &home,
            &["--command", binary],
        )
        .assert(),
    ];
    for assert in &outcomes {
        let output = assert.get_output();
        assert!(!String::from_utf8_lossy(&output.stdout).contains(TOKEN));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(TOKEN));
    }

    // A token that exists only in the config file must stay out too.
    let stored = server_with(|_| {}).await;
    let home = home_with_stored_token(&stored.uri());
    let assert = ifg(&home)
        .args(["mcp", "doctor", "--command", binary])
        .assert()
        .success();
    let output = assert.get_output();
    assert!(!String::from_utf8_lossy(&output.stdout).contains(TOKEN));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(TOKEN));
}

#[tokio::test]
async fn json_output_is_an_array_of_doctor_check_items() {
    let bin = TempDir::new().unwrap();
    let binary = fake_binary(bin.path(), "ironforge-mcp");
    let binary = binary.to_str().unwrap();
    let home = TempDir::new().unwrap();

    let scenarios = [
        server_with(|body| body["mcp_transport"]["tool_count"] = json!(9)).await,
        server_without_capabilities().await,
    ];
    for server in &scenarios {
        let assert = doctor(server, &home, &["--json", "--command", binary]).assert();
        let items = items(assert.get_output());
        assert!(!items.is_empty());
        for item in &items {
            let object = item.as_object().unwrap();
            let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
            keys.sort_unstable();
            assert_eq!(keys, ["check", "detail", "hint", "status"], "{item}");
            assert!(item["check"].is_string(), "{item}");
            assert!(item["detail"].is_string(), "{item}");
            assert!(item["hint"].is_string() || item["hint"].is_null(), "{item}");
            assert!(
                matches!(item["status"].as_str(), Some("ok" | "warn" | "fail")),
                "{item}"
            );
        }
    }
}

#[tokio::test]
async fn template_renders_one_line_per_check() {
    let server = server_with(|_| {}).await;
    let home = TempDir::new().unwrap();
    let bin = TempDir::new().unwrap();
    let binary = fake_binary(bin.path(), "ironforge-mcp");

    let assert = doctor(
        &server,
        &home,
        &[
            "-q",
            "{{.check}}={{.status}}",
            "--command",
            binary.to_str().unwrap(),
        ],
    )
    .assert()
    .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(stdout.contains("capabilities=ok"), "{stdout}");
    assert!(stdout.contains("IRONFORGE_PAT="), "{stdout}");
}
