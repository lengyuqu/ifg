//! Optional keychain backend for tokens (design doc D6): with
//! `token_store = "keyring"` in `hosts.toml` the PAT never lands in the file.
//!
//! Windows and macOS go through the `keyring` crate, which calls the native
//! credential API in-process. That matters for secrecy: the macOS `security`
//! CLI only accepts the password as `-w <token>`, so the PAT would show up in
//! `ps` for every local user — the same leak `git.rs` avoids by passing the
//! token through `GIT_CONFIG_*` instead of argv.
//!
//! Linux uses `secret-tool` with the token on stdin, which is equally safe.
//! When no backend answers, callers degrade to the config file with a warning.

const SERVICE: &str = "ifg";

#[cfg(any(windows, target_os = "macos"))]
fn native_entry(host_key: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, host_key).map_err(|e| format!("system keychain: {e}"))
}

/// Store (or replace) the token for `host_key`.
#[cfg(any(windows, target_os = "macos"))]
pub fn store(host_key: &str, token: &str) -> Result<(), String> {
    native_entry(host_key)?
        .set_password(token)
        .map_err(|e| format!("system keychain: {e}"))
}

/// Fetch the token for `host_key`.
#[cfg(any(windows, target_os = "macos"))]
pub fn load(host_key: &str) -> Option<String> {
    native_entry(host_key)
        .ok()?
        .get_password()
        .ok()
        .filter(|token| !token.is_empty())
}

/// Remove the token for `host_key` (missing entries are fine).
#[cfg(any(windows, target_os = "macos"))]
pub fn delete(host_key: &str) -> Result<(), String> {
    match native_entry(host_key)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("system keychain: {e}")),
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn secret_tool_available() -> bool {
    std::process::Command::new("sh")
        .args(["-c", "command -v secret-tool >/dev/null 2>&1"])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Store (or replace) the token for `host_key`.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn store(host_key: &str, token: &str) -> Result<(), String> {
    if !secret_tool_available() {
        return Err("no keychain backend (secret-tool) found".into());
    }
    let label = format!("{SERVICE}: {host_key}");
    run(
        "secret-tool",
        &[
            "store", "--label", &label, "service", SERVICE, "host", host_key,
        ],
        Some(token),
    )
}

/// Fetch the token for `host_key`.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn load(host_key: &str) -> Option<String> {
    if !secret_tool_available() {
        return None;
    }
    let output = std::process::Command::new("secret-tool")
        .args(["lookup", "service", SERVICE, "host", host_key])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!token.is_empty()).then_some(token)
}

/// Remove the token for `host_key` (missing entries are fine).
#[cfg(all(unix, not(target_os = "macos")))]
pub fn delete(host_key: &str) -> Result<(), String> {
    if !secret_tool_available() {
        return Ok(());
    }
    let _ = std::process::Command::new("secret-tool")
        .args(["clear", "service", SERVICE, "host", host_key])
        .status();
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn run(program: &str, args: &[&str], stdin: Option<&str>) -> Result<(), String> {
    use std::io::Write;
    let mut command = std::process::Command::new(program);
    command.args(args);
    if stdin.is_some() {
        command.stdin(std::process::Stdio::piped());
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    if let (Some(input), Some(pipe)) = (stdin, child.stdin.as_mut()) {
        pipe.write_all(input.as_bytes())
            .map_err(|e| format!("could not write to {program}: {e}"))?;
    }
    let status = child
        .wait()
        .map_err(|e| format!("{program} did not finish: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with {status}"))
    }
}

#[cfg(all(test, any(windows, target_os = "macos")))]
mod tests {
    #[test]
    fn native_credential_roundtrip() {
        // The macOS keychain needs an unlocked login session; headless CI
        // runners have none, so require an explicit opt-in there. Windows
        // Credential Manager works without one.
        #[cfg(target_os = "macos")]
        if std::env::var_os("IFG_KEYRING_TEST").is_none() {
            return;
        }
        let host = format!("ifg-test-{}", std::process::id());
        super::store(&host, "secret-token").expect("store");
        assert_eq!(super::load(&host).as_deref(), Some("secret-token"));
        super::delete(&host).expect("delete");
        assert_eq!(super::load(&host), None);
    }
}
