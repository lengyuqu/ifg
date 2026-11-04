//! Optional keychain backend for tokens (design doc D6): with
//! `token_store = "keyring"` in `hosts.toml` the PAT never lands in the file.
//!
//! Implemented through the platform CLI (`security` on macOS, `secret-tool`
//! on Linux) and the Windows Credential Manager (via the `keyring` crate) so
//! the client keeps its tiny dependency budget; when no backend answers,
//! callers degrade to the config file with a warning.

const SERVICE: &str = "ifg";

#[cfg(windows)]
fn windows_entry(host_key: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, host_key).map_err(|e| format!("Windows Credential Manager: {e}"))
}

/// Store (or replace) the token for `host_key`.
#[cfg(windows)]
pub fn store(host_key: &str, token: &str) -> Result<(), String> {
    windows_entry(host_key)?
        .set_password(token)
        .map_err(|e| format!("Windows Credential Manager: {e}"))
}

/// Fetch the token for `host_key`.
#[cfg(windows)]
pub fn load(host_key: &str) -> Option<String> {
    let entry = windows_entry(host_key).ok()?;
    entry.get_password().ok().filter(|token| !token.is_empty())
}

/// Remove the token for `host_key` (missing entries are fine).
#[cfg(windows)]
pub fn delete(host_key: &str) -> Result<(), String> {
    match windows_entry(host_key)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("Windows Credential Manager: {e}")),
    }
}

#[cfg(not(windows))]
enum Backend {
    MacOsSecurity,
    SecretTool,
}

#[cfg(not(windows))]
fn backend() -> Option<Backend> {
    fn exists(program: &str) -> bool {
        std::process::Command::new("sh")
            .args(["-c", &format!("command -v {program} >/dev/null 2>&1")])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
    if cfg!(target_os = "macos") && exists("security") {
        return Some(Backend::MacOsSecurity);
    }
    if cfg!(target_os = "linux") && exists("secret-tool") {
        return Some(Backend::SecretTool);
    }
    None
}

/// Store (or replace) the token for `host_key`.
#[cfg(not(windows))]
pub fn store(host_key: &str, token: &str) -> Result<(), String> {
    match backend() {
        Some(Backend::MacOsSecurity) => run(
            "security",
            &[
                "add-generic-password",
                "-a",
                host_key,
                "-s",
                SERVICE,
                "-w",
                token,
                "-U",
            ],
            None,
        ),
        Some(Backend::SecretTool) => run(
            "secret-tool",
            &[
                "store",
                "--label",
                &format!("{SERVICE}: {host_key}"),
                "service",
                SERVICE,
                "host",
                host_key,
            ],
            Some(token),
        ),
        None => Err("no keychain backend (security / secret-tool) found".into()),
    }
}

/// Fetch the token for `host_key`.
#[cfg(not(windows))]
pub fn load(host_key: &str) -> Option<String> {
    let output = match backend() {
        Some(Backend::MacOsSecurity) => std::process::Command::new("security")
            .args(["find-generic-password", "-a", host_key, "-s", SERVICE, "-w"])
            .output(),
        Some(Backend::SecretTool) => std::process::Command::new("secret-tool")
            .args(["lookup", "service", SERVICE, "host", host_key])
            .output(),
        None => return None,
    }
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!token.is_empty()).then_some(token)
}

/// Remove the token for `host_key` (missing entries are fine).
#[cfg(not(windows))]
pub fn delete(host_key: &str) -> Result<(), String> {
    match backend() {
        Some(Backend::MacOsSecurity) => {
            let _ = std::process::Command::new("security")
                .args(["delete-generic-password", "-a", host_key, "-s", SERVICE])
                .status();
            Ok(())
        }
        Some(Backend::SecretTool) => {
            let _ = std::process::Command::new("secret-tool")
                .args(["clear", "service", SERVICE, "host", host_key])
                .status();
            Ok(())
        }
        None => Ok(()),
    }
}

#[cfg(not(windows))]
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

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn windows_credential_roundtrip() {
        let host = format!("ifg-test-{}", std::process::id());
        super::store(&host, "secret-token").expect("store");
        assert_eq!(super::load(&host).as_deref(), Some("secret-token"));
        super::delete(&host).expect("delete");
        assert_eq!(super::load(&host), None);
    }
}
