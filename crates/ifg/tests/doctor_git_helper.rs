use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn doctor_hint_configures_a_working_host_scoped_helper() {
    let server = MockServer::start().await;
    for (route, body) in [
        ("/health", json!({"version":"0.2.0"})),
        ("/api/v1/users/me", json!({"id":1,"username":"alice"})),
        ("/api/v1/repos/alice", json!([])),
    ] {
        Mock::given(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
    }
    let home = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let dir = home.path().join("ironforge");
    std::fs::create_dir(&dir).unwrap();
    let key = server.uri().trim_start_matches("http://").to_string();
    let config = dir.join("hosts.toml");
    std::fs::write(
        &config,
        format!(
            "default_host='{key}'\n[hosts.'{key}']\nurl='{}'\nuser='alice'\ntoken='ifp_fixture'\n",
            server.uri()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let git_config = home.path().join("gitconfig");
    let doctor = || {
        Command::cargo_bin("ifg")
            .unwrap()
            .current_dir(work.path())
            .env("XDG_CONFIG_HOME", home.path())
            .env_remove("IRONFORGE_URL")
            .env_remove("IRONFORGE_PAT")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &git_config)
            .args(["doctor", "--json"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone()
    };
    let checks: Value = serde_json::from_slice(&doctor()).unwrap();
    let git = checks
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["check"] == "git")
        .unwrap();
    let hint = git["hint"].as_str().unwrap();
    let helper = if cfg!(windows) {
        "!ifg.exe auth git-credential"
    } else {
        "!ifg auth git-credential"
    };
    assert!(hint.contains(helper), "{hint}");
    assert!(
        hint.contains(&format!("credential.{}.helper", server.uri())),
        "{hint}"
    );
    // Execute exactly the emitted hint against an isolated global config.
    let mut shell = if cfg!(windows) {
        let mut shell = std::process::Command::new("pwsh");
        shell.args(["-NoProfile", "-NonInteractive", "-Command", hint]);
        shell
    } else {
        let mut shell = std::process::Command::new("sh");
        shell.args(["-c", hint]);
        shell
    };
    let result = shell
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", &git_config)
        .current_dir(work.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let checks: Value = serde_json::from_slice(&doctor()).unwrap();
    assert_eq!(
        checks
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["check"] == "git")
            .unwrap()["status"],
        "ok"
    );

    let binary = assert_cmd::cargo::cargo_bin("ifg");
    let task_path = std::env::join_paths(
        std::iter::once(binary.parent().unwrap().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let result = assert_cmd::Command::new("git")
        .current_dir(work.path())
        .env("PATH", task_path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", &git_config)
        .env("XDG_CONFIG_HOME", home.path())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("IRONFORGE_URL")
        .env_remove("IRONFORGE_PAT")
        .args(["credential", "fill"])
        .write_stdin(format!("protocol=http\nhost={key}\n\n"))
        .assert()
        .success();
    assert!(String::from_utf8_lossy(&result.get_output().stdout).contains("password=ifp_fixture"));
}
