//! `ifg mcp config`: the snippet is JSON, carries the resolved host and the
//! `${IRONFORGE_PAT}` placeholder, never the token, and makes no request.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;
use wiremock::MockServer;

const TOKEN: &str = "ifp_mcptestsecretvalue0123456789abcdef";

fn ifg(home: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("XDG_CONFIG_HOME", home.path())
        .env_remove("IRONFORGE_URL")
        .env_remove("IRONFORGE_PAT");
    cmd
}

fn empty_home() -> TempDir {
    TempDir::new().unwrap()
}

/// A config whose default host carries [`TOKEN`] in the file itself.
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

fn stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout must be exactly one JSON document")
}

#[tokio::test]
async fn snippet_names_the_resolved_host_and_placeholder_token() {
    let server = MockServer::start().await;
    let home = home_with_stored_token(&server.uri());

    let assert = ifg(&home).args(["mcp", "config"]).assert().success();
    let output = assert.get_output();
    let value = stdout_json(output);

    let entry = &value["mcpServers"]["ironforge"];
    assert_eq!(entry["command"], "ironforge-mcp");
    assert_eq!(entry["env"]["IRONFORGE_URL"], server.uri().as_str());
    assert_eq!(entry["env"]["IRONFORGE_PAT"], "${IRONFORGE_PAT}");
    assert_eq!(value.as_object().unwrap().len(), 1);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("IRONFORGE_PAT"), "stderr: {stderr}");
    assert!(stderr.contains("ifg auth token"), "stderr: {stderr}");
}

#[tokio::test]
async fn every_client_emits_the_same_mcp_servers_shape() {
    let server = MockServer::start().await;
    let home = empty_home();
    let mut shapes = Vec::new();
    for client in ["claude-code", "cursor", "generic"] {
        let assert = ifg(&home)
            .args(["--host", &server.uri(), "mcp", "config", "--client", client])
            .assert()
            .success();
        shapes.push(stdout_json(assert.get_output()));
    }
    let default = ifg(&home)
        .args(["--host", &server.uri(), "mcp", "config"])
        .assert()
        .success();
    shapes.push(stdout_json(default.get_output()));
    assert!(shapes.windows(2).all(|pair| pair[0] == pair[1]));
}

#[tokio::test]
async fn unknown_client_is_a_usage_error() {
    let server = MockServer::start().await;
    let home = empty_home();
    ifg(&home)
        .args([
            "--host",
            &server.uri(),
            "mcp",
            "config",
            "--client",
            "vscode",
        ])
        .assert()
        .code(2);
}

#[tokio::test]
async fn name_and_command_flags_are_honoured() {
    let server = MockServer::start().await;
    let home = empty_home();
    let assert = ifg(&home)
        .args([
            "--host",
            &server.uri(),
            "mcp",
            "config",
            "--name",
            "forge-prod",
            "--command",
            "/opt/ifg/bin/ironforge-mcp",
        ])
        .assert()
        .success();
    let value = stdout_json(assert.get_output());
    let servers = value["mcpServers"].as_object().unwrap();
    assert_eq!(servers.len(), 1);
    let entry = &servers["forge-prod"];
    assert_eq!(entry["command"], "/opt/ifg/bin/ironforge-mcp");
    assert_eq!(entry["env"]["IRONFORGE_URL"], server.uri().as_str());
}

#[tokio::test]
async fn json_flag_prints_the_same_document() {
    let server = MockServer::start().await;
    let home = empty_home();
    let plain = ifg(&home)
        .args(["--host", &server.uri(), "mcp", "config"])
        .assert()
        .success();
    let json = ifg(&home)
        .args(["--json", "--host", &server.uri(), "mcp", "config"])
        .assert()
        .success();
    assert_eq!(plain.get_output().stdout, json.get_output().stdout);
}

#[tokio::test]
async fn template_output_is_rejected() {
    let server = MockServer::start().await;
    let home = empty_home();
    let assert = ifg(&home)
        .args(["-q", "{{.x}}", "--host", &server.uri(), "mcp", "config"])
        .assert()
        .code(2);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(
        stderr.contains("does not support --json or --template"),
        "stderr: {stderr}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn token_never_appears_in_stdout_or_stderr() {
    let server = MockServer::start().await;
    let stored = home_with_stored_token(&server.uri());
    let from_config = ifg(&stored)
        .args(["mcp", "config"])
        .assert()
        .success()
        .get_output()
        .clone();
    let from_env = ifg(&empty_home())
        .env("IRONFORGE_PAT", TOKEN)
        .args(["--json", "--host", &server.uri(), "mcp", "config"])
        .assert()
        .success()
        .get_output()
        .clone();
    for output in [from_config, from_env] {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stdout.contains(TOKEN), "stdout leaked the token");
        assert!(!stderr.contains(TOKEN), "stderr leaked the token");
        assert!(!stdout.contains("ifp_"), "stdout: {stdout}");
    }
}

#[tokio::test]
async fn no_request_is_made_and_no_file_is_written() {
    let server = MockServer::start().await;
    let home = home_with_stored_token(&server.uri());
    let config_dir = home.path().join("ironforge");
    let before = std::fs::read_dir(&config_dir).unwrap().count();
    ifg(&home).args(["mcp", "config"]).assert().success();
    assert!(server.received_requests().await.unwrap().is_empty());
    let after = std::fs::read_dir(&config_dir).unwrap().count();
    assert_eq!(before, after);
}

#[tokio::test]
async fn missing_host_fails_with_the_existing_host_error() {
    let home = empty_home();
    let assert = ifg(&home).args(["mcp", "config"]).assert().code(1);
    let output = assert.get_output();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no IronForge host configured"),
        "stderr: {stderr}"
    );
    assert!(output.stdout.is_empty());
}
