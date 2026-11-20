//! `IRONFORGE_CONFIG_DIR` and env-only credentials: where ifg stores and reads
//! its files, which token a request carries, and whether the keychain or the
//! stored config is touched at all.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::{
    matchers::{any, header, path},
    Mock, MockServer, ResponseTemplate,
};

const USER_PATH: &str = "/api/v1/users/me";

/// `ifg` with every ifg-related variable removed, so the developer's shell
/// cannot leak a host, token or config directory into the test.
fn ifg(home: &Path, xdg: &Path) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("HOME", home)
        .env("XDG_CONFIG_HOME", xdg)
        .env_remove("IRONFORGE_URL")
        .env_remove("IRONFORGE_PAT")
        .env_remove("IRONFORGE_CONFIG_DIR")
        .env_remove("IFG_NO_INPUT");
    cmd
}

/// Host key and URL for a mock server, as `hosts.toml` spells them.
fn host_of(server: &MockServer) -> (String, String) {
    let url = server.uri();
    let key = url.trim_start_matches("http://").to_string();
    (key, url)
}

/// Write a `hosts.toml` with one host. `mode` is applied on Unix.
fn write_hosts(
    dir: &Path,
    key: &str,
    url: &str,
    token: Option<&str>,
    token_store: Option<&str>,
    mode: u32,
) {
    std::fs::create_dir_all(dir).unwrap();
    let mut text = String::new();
    if let Some(store) = token_store {
        text.push_str(&format!("token_store = \"{store}\"\n"));
    }
    text.push_str(&format!(
        "[hosts.\"{key}\"]\nurl = \"{url}\"\nuser = \"agent\"\n"
    ));
    if let Some(token) = token {
        text.push_str(&format!("token = \"{token}\"\n"));
    }
    let file = dir.join("hosts.toml");
    std::fs::write(&file, text).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
    }
    #[cfg(not(unix))]
    let _ = mode;
}

/// Every file and directory below `root`, relative and sorted. Empty when
/// `root` does not exist.
fn tree(root: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            out.push(
                path.strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            );
            if path.is_dir() {
                walk(base, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

/// The `Authorization` header of every request the server received, in order.
async fn authorizations(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| {
            request
                .headers
                .get("authorization")
                .map(|value| value.to_str().unwrap().to_string())
                .unwrap_or_default()
        })
        .collect()
}

fn user_json(username: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "id": 1, "username": username }))
}

/// `ifg doctor --json` stdout, parsed. Doctor exits non-zero when checks fail,
/// so the status is not asserted here.
fn doctor_checks(mut cmd: Command) -> Vec<Value> {
    let output = cmd.args(["doctor", "--json"]).output().unwrap();
    serde_json::from_slice(&output.stdout).expect("doctor --json prints an array")
}

fn check<'a>(checks: &'a [Value], name: &str) -> Option<&'a Value> {
    checks.iter().find(|item| item["check"] == name)
}

/// Make `dir` unwritable for the test and restore it on drop, so the temporary
/// directory can still be removed.
// The path is only restored on drop under unix; elsewhere the guard is a no-op.
#[cfg_attr(not(unix), allow(dead_code))]
struct ReadOnly(PathBuf);

impl ReadOnly {
    fn new(dir: &Path) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        }
        Self(dir.to_path_buf())
    }
}

impl Drop for ReadOnly {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
    }
}

/// Login with `IRONFORGE_CONFIG_DIR` writes only below that directory: nothing
/// appears under HOME or XDG_CONFIG_HOME.
#[tokio::test]
async fn login_with_config_dir_writes_only_under_that_dir() {
    let server = MockServer::start().await;
    let (_, url) = host_of(&server);
    Mock::given(path(USER_PATH))
        .and(header("authorization", "Bearer ifp_login_secret"))
        .respond_with(user_json("agent-a"))
        .expect(1)
        .mount(&server)
        .await;
    let work = TempDir::new().unwrap();
    let home = work.path().join("home");
    let xdg = work.path().join("xdg");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&xdg).unwrap();
    let config_dir = work.path().join("agent-a");

    let output = ifg(&home, &xdg)
        .env("IRONFORGE_CONFIG_DIR", &config_dir)
        .args(["--host", &url, "auth", "login", "--with-token"])
        .write_stdin("ifp_login_secret\n")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");

    let stored = std::fs::read_to_string(config_dir.join("hosts.toml")).unwrap();
    assert!(stored.contains("ifp_login_secret"), "{stored}");
    assert!(
        tree(&home).is_empty(),
        "HOME was written: {:?}",
        tree(&home)
    );
    assert!(tree(&xdg).is_empty(), "XDG was written: {:?}", tree(&xdg));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(config_dir.join("hosts.toml"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

/// The isolation test: two config directories hold two tokens for the same
/// host, and each request carries the token of the directory it ran with.
/// Checked on the Authorization header the server actually received.
#[tokio::test]
async fn two_config_dirs_send_their_own_token_to_the_same_host() {
    let server = MockServer::start().await;
    let (key, url) = host_of(&server);
    for token in ["ifp_agent_a", "ifp_agent_b"] {
        Mock::given(path(USER_PATH))
            .and(header("authorization", format!("Bearer {token}").as_str()))
            .respond_with(user_json(token))
            .expect(1)
            .mount(&server)
            .await;
    }
    let work = TempDir::new().unwrap();
    let home = work.path().join("home");
    let xdg = work.path().join("xdg");
    let dir_a = work.path().join("agent-a");
    let dir_b = work.path().join("agent-b");
    write_hosts(&dir_a, &key, &url, Some("ifp_agent_a"), None, 0o600);
    write_hosts(&dir_b, &key, &url, Some("ifp_agent_b"), None, 0o600);

    for dir in [&dir_a, &dir_b] {
        ifg(&home, &xdg)
            .env("IRONFORGE_CONFIG_DIR", dir)
            .args(["--host", &url, "api", "users/me"])
            .assert()
            .success();
    }

    assert_eq!(
        authorizations(&server).await,
        ["Bearer ifp_agent_a", "Bearer ifp_agent_b"],
        "each directory must send only its own token"
    );
}

/// With both env variables set, the stored file (here insecure, so it would
/// also print a warning if read) is not read, the keychain is not probed, and
/// nothing is created. HOME does not exist; XDG points at a read-only directory.
#[tokio::test]
async fn env_only_run_reads_and_creates_nothing() {
    let server = MockServer::start().await;
    let (key, url) = host_of(&server);
    Mock::given(path(USER_PATH))
        .and(header("authorization", "Bearer ifp_env_token"))
        .respond_with(user_json("env-user"))
        .mount(&server)
        .await;
    let work = TempDir::new().unwrap();
    let home = work.path().join("no-such-home");
    let xdg = work.path().join("xdg");
    write_hosts(
        &xdg.join("ironforge"),
        &key,
        &url,
        Some("ifp_stored_token"),
        None,
        0o644,
    );
    let _guard = ReadOnly::new(&xdg.join("ironforge"));
    let _guard_xdg = ReadOnly::new(&xdg);
    let before = tree(work.path());

    let env_run = |args: &[&str]| {
        let mut cmd = ifg(&home, &xdg);
        cmd.env("IRONFORGE_URL", &url)
            .env("IRONFORGE_PAT", "ifp_env_token")
            .args(args);
        cmd
    };
    let api = env_run(&["api", "users/me"]).output().unwrap();
    assert!(api.status.success(), "{api:?}");
    let stderr = String::from_utf8_lossy(&api.stderr);
    assert!(!stderr.contains("readable by others"), "{stderr}");

    let checks = doctor_checks(env_run(&[]));
    assert!(check(&checks, "config-dir").is_some(), "{checks:?}");
    assert!(check(&checks, "config").is_none(), "{checks:?}");

    assert_eq!(tree(work.path()), before, "env-only run changed the disk");
    assert!(!home.exists(), "HOME was created");
    // Doctor makes more requests than `api`, and its unauthenticated health
    // probe sends no header. Every request that carries a credential must
    // carry the env token, and the stored one must never appear.
    let sent = authorizations(&server).await;
    let credentialed: Vec<&String> = sent.iter().filter(|value| !value.is_empty()).collect();
    assert!(!credentialed.is_empty());
    assert!(
        credentialed
            .iter()
            .all(|value| *value == "Bearer ifp_env_token"),
        "the stored token must not be used: {sent:?}"
    );
}

/// `auth status` in env-only mode names the source `env`, lists the env host,
/// has no config directory and never prints the token.
#[tokio::test]
async fn status_reports_env_source_without_the_token() {
    let server = MockServer::start().await;
    let (_, url) = host_of(&server);
    Mock::given(any())
        .respond_with(user_json("env-user"))
        .mount(&server)
        .await;
    let work = TempDir::new().unwrap();
    let home = work.path().join("home");
    let xdg = work.path().join("xdg");
    let token = "ifp_very_secret_env_token_value";

    let output = ifg(&home, &xdg)
        .env("IRONFORGE_URL", &url)
        .env("IRONFORGE_PAT", token)
        .args(["auth", "status", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(token), "token printed: {stdout}");
    let rows: Value = serde_json::from_str(&stdout).unwrap();
    let row = &rows[0];
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(row["source"], "env");
    assert!(row["config_dir"].is_null());
    assert_eq!(row["url"], url.as_str());
}

/// Without env credentials, `auth status` names `config-dir` and the directory
/// the file was read from.
#[tokio::test]
async fn status_reports_config_dir_source_and_directory() {
    let server = MockServer::start().await;
    let (key, url) = host_of(&server);
    Mock::given(any())
        .respond_with(user_json("agent-a"))
        .mount(&server)
        .await;
    let work = TempDir::new().unwrap();
    let home = work.path().join("home");
    let xdg = work.path().join("xdg");
    let dir_a = work.path().join("agent-a");
    write_hosts(&dir_a, &key, &url, Some("ifp_agent_a"), None, 0o600);

    let output = ifg(&home, &xdg)
        .env("IRONFORGE_CONFIG_DIR", &dir_a)
        .args(["auth", "status", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let rows: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rows[0]["source"], "config-dir");
    assert_eq!(rows[0]["config_dir"], dir_a.to_string_lossy().as_ref());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("ifp_agent_a"));
}

/// The config directory precedence, seen through `doctor`: the explicit
/// override beats XDG_CONFIG_HOME, which beats HOME. An empty override is
/// ignored. Doctor fails here (no host) but still prints its checks.
fn config_dir_detail(home: &Path, xdg: Option<&Path>, override_dir: Option<&str>) -> String {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("HOME", home)
        .env_remove("IRONFORGE_URL")
        .env_remove("IRONFORGE_PAT")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("IRONFORGE_CONFIG_DIR")
        .env_remove("IFG_NO_INPUT");
    if let Some(xdg) = xdg {
        cmd.env("XDG_CONFIG_HOME", xdg);
    }
    if let Some(dir) = override_dir {
        cmd.env("IRONFORGE_CONFIG_DIR", dir);
    }
    let checks = doctor_checks(cmd);
    let item = check(&checks, "config-dir").expect("config-dir check");
    item["detail"].as_str().unwrap().to_string()
}

#[test]
fn config_dir_precedence_override_then_xdg_then_home() {
    let work = TempDir::new().unwrap();
    let home = work.path().join("home");
    let xdg = work.path().join("xdg");
    let explicit = work.path().join("explicit");

    let detail = config_dir_detail(&home, Some(&xdg), Some(explicit.to_str().unwrap()));
    assert!(
        detail.starts_with(&format!("{} (IRONFORGE_CONFIG_DIR)", explicit.display())),
        "{detail}"
    );

    let detail = config_dir_detail(&home, Some(&xdg), Some(""));
    assert!(
        detail.starts_with(&format!(
            "{} (XDG_CONFIG_HOME)",
            xdg.join("ironforge").display()
        )),
        "empty override must be ignored: {detail}"
    );

    // The platform default is HOME-based on Unix only.
    #[cfg(not(windows))]
    {
        let detail = config_dir_detail(&home, None, None);
        assert!(
            detail.starts_with(&format!(
                "{} (default)",
                home.join(".config").join("ironforge").display()
            )),
            "{detail}"
        );
    }
}

/// A non-writable directory is only a warning, and only the env-only run says
/// that the environment credentials do not need it.
#[cfg(unix)]
#[tokio::test]
async fn read_only_config_dir_warns_in_env_only_runs() {
    let work = TempDir::new().unwrap();
    let home = work.path().join("home");
    let locked = work.path().join("locked");
    std::fs::create_dir_all(&locked).unwrap();
    let _guard = ReadOnly::new(&locked);
    // A writer in this sandbox could still write to a 0555 directory; skip the
    // check then rather than report a false pass.
    let writable = std::process::Command::new("sh")
        .args(["-c", "test -w \"$1\"", "probe"])
        .arg(&locked)
        .status()
        .unwrap()
        .success();
    if writable {
        return;
    }

    let mut cmd = ifg(&home, &work.path().join("xdg"));
    cmd.env("IRONFORGE_CONFIG_DIR", &locked)
        .env("IRONFORGE_URL", "http://127.0.0.1:9")
        .env("IRONFORGE_PAT", "ifp_env_token");
    let checks = doctor_checks(cmd);
    let item = check(&checks, "config-dir").expect("config-dir check");
    assert_eq!(item["status"], "warn", "{item}");
    assert!(item["hint"].as_str().unwrap().contains("env credentials"));
}

/// Keychain isolation. The keyring backend is observed through a fake
/// `secret-tool` on PATH that records each call; the macOS and Windows backends
/// run in-process and cannot be observed this way (see the tests in keyring.rs).
#[cfg(all(unix, not(target_os = "macos")))]
mod keychain {
    use super::*;

    struct Fake {
        path: String,
        record: PathBuf,
    }

    fn fake_secret_tool(work: &Path) -> Fake {
        use std::os::unix::fs::PermissionsExt;
        let bin = work.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let tool = bin.join("secret-tool");
        std::fs::write(
            &tool,
            "#!/bin/sh\necho \"$*\" >> \"$IFG_TEST_KEYCHAIN_RECORD\"\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let existing = std::env::var("PATH").unwrap_or_default();
        Fake {
            path: format!("{}:{existing}", bin.display()),
            record: work.join("keychain-calls"),
        }
    }

    fn with_fake(cmd: &mut Command, fake: &Fake) {
        cmd.env("PATH", &fake.path)
            .env("IFG_TEST_KEYCHAIN_RECORD", &fake.record);
    }

    /// Control: without the override the fake sees a lookup, so the harness
    /// really does observe keychain use.
    #[tokio::test]
    async fn fake_keychain_is_observed_without_the_override() {
        let server = MockServer::start().await;
        let (key, url) = host_of(&server);
        Mock::given(any())
            .respond_with(user_json("x"))
            .mount(&server)
            .await;
        let work = TempDir::new().unwrap();
        let home = work.path().join("home");
        let xdg = work.path().join("xdg");
        write_hosts(
            &xdg.join("ironforge"),
            &key,
            &url,
            None,
            Some("keyring"),
            0o600,
        );
        let fake = fake_secret_tool(work.path());
        let mut cmd = ifg(&home, &xdg);
        with_fake(&mut cmd, &fake);
        cmd.args(["--host", &url, "api", "users/me"])
            .assert()
            .success();
        assert!(
            fake.record.exists(),
            "the control run must reach the keychain"
        );
    }

    /// With `IRONFORGE_CONFIG_DIR` the keychain is never consulted: neither a
    /// read for a `token_store = "keyring"` file nor a write on login.
    #[tokio::test]
    async fn config_dir_never_touches_the_keychain() {
        let server = MockServer::start().await;
        let (key, url) = host_of(&server);
        // Both the unauthenticated `api` call and the login's validation hit
        // this path; the login sends its token, `api` sends none.
        Mock::given(path(USER_PATH))
            .respond_with(user_json("agent-a"))
            .mount(&server)
            .await;
        let work = TempDir::new().unwrap();
        let home = work.path().join("home");
        let xdg = work.path().join("xdg");
        let dir_a = work.path().join("agent-a");
        write_hosts(&dir_a, &key, &url, None, Some("keyring"), 0o600);
        let fake = fake_secret_tool(work.path());

        let mut api = ifg(&home, &xdg);
        with_fake(&mut api, &fake);
        api.env("IRONFORGE_CONFIG_DIR", &dir_a)
            .args(["--host", &url, "api", "users/me"])
            .assert()
            .success();

        let mut login = ifg(&home, &xdg);
        with_fake(&mut login, &fake);
        login
            .env("IRONFORGE_CONFIG_DIR", &dir_a)
            .args(["--host", &url, "auth", "login", "--with-token"])
            .write_stdin("ifp_agent_login\n")
            .assert()
            .success();

        assert!(
            !fake.record.exists(),
            "keychain was used: {:?}",
            std::fs::read_to_string(&fake.record)
        );
        let stored = std::fs::read_to_string(dir_a.join("hosts.toml")).unwrap();
        assert!(
            stored.contains("ifp_agent_login"),
            "token belongs in the file: {stored}"
        );
    }

    /// Env-only runs do not probe the keychain either, even when the stored
    /// file asks for it.
    #[tokio::test]
    async fn env_only_run_never_probes_the_keychain() {
        let server = MockServer::start().await;
        let (key, url) = host_of(&server);
        Mock::given(any())
            .respond_with(user_json("env-user"))
            .mount(&server)
            .await;
        let work = TempDir::new().unwrap();
        let home = work.path().join("home");
        let xdg = work.path().join("xdg");
        write_hosts(
            &xdg.join("ironforge"),
            &key,
            &url,
            None,
            Some("keyring"),
            0o600,
        );
        let fake = fake_secret_tool(work.path());
        let mut cmd = ifg(&home, &xdg);
        with_fake(&mut cmd, &fake);
        cmd.env("IRONFORGE_URL", &url)
            .env("IRONFORGE_PAT", "ifp_env_token")
            .args(["api", "users/me"])
            .assert()
            .success();
        assert!(!fake.record.exists(), "keychain was probed");
    }
}
