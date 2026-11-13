//! `ifg schema` runs as a plain local command: no config, no token, no
//! network, and it creates nothing on disk.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

/// An `ifg` with an empty config home, no token and an unreachable server.
/// Running from an empty directory keeps git remote inference out of it.
fn bare_ifg(home: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.current_dir(home.path())
        .env("XDG_CONFIG_HOME", home.path())
        .env("HOME", home.path())
        .env("IRONFORGE_URL", "http://127.0.0.1:9")
        .env_remove("IRONFORGE_PAT")
        .env_remove("IRONFORGE_TOKEN")
        .env_remove("IRONFORGE_HOST");
    cmd
}

fn stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout is one JSON document")
}

#[test]
fn schema_needs_no_config_token_or_network() {
    let home = TempDir::new().unwrap();
    let assert = bare_ifg(&home).arg("schema").assert().success();
    let output = assert.get_output().clone();
    assert!(output.stderr.is_empty(), "stderr: {:?}", output.stderr);

    let doc = stdout_json(&output);
    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["ifg_version"], env!("CARGO_PKG_VERSION"));
    assert!(doc["commands"].as_array().unwrap().len() > 10);

    // Nothing was written under the config home.
    let entries: Vec<_> = std::fs::read_dir(home.path()).unwrap().collect();
    assert!(entries.is_empty(), "schema created files: {entries:?}");
}

#[test]
fn schema_with_a_path_prints_only_that_subtree() {
    let home = TempDir::new().unwrap();
    let output = bare_ifg(&home)
        .args(["schema", "pr", "merge"])
        .assert()
        .success()
        .get_output()
        .clone();
    let doc = stdout_json(&output);
    let commands = doc["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0]["path"], serde_json::json!(["pr", "merge"]));
    assert_eq!(commands[0]["effect"], "write");
    assert_eq!(commands[0]["json_supported"], true);
}

#[test]
fn schema_unknown_path_is_a_usage_error() {
    let home = TempDir::new().unwrap();
    let result = bare_ifg(&home)
        .args(["schema", "pr", "nosuch"])
        .assert()
        .code(2);
    let stderr = String::from_utf8_lossy(&result.get_output().stderr).into_owned();
    assert!(stderr.contains("no command `pr nosuch`"), "{stderr}");
}
