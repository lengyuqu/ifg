//! Git remote handling (design doc §5.2, §5.3): everything goes through the
//! system `git` binary, no gix/libgit2 dependency. Only the URL parsing is
//! needed in M1 — for host inference from inside a work tree.

use std::process::Command;

/// Match short and fully qualified branch refs without treating a same-named
/// tag as a branch. An explicitly qualified tag/ref remains an exact match.
pub fn matches_ref(reference: &str, selector: &str) -> bool {
    if let Some(branch) = selector.strip_prefix("refs/heads/") {
        matches_branch_ref(reference, branch)
    } else if selector.starts_with("refs/") {
        reference == selector
    } else {
        matches_branch_ref(reference, selector)
    }
}

/// Here `branch` is a branch name from Git, not an explicit ref selector.
pub fn matches_branch_ref(reference: &str, branch: &str) -> bool {
    match reference.strip_prefix("refs/heads/") {
        Some(name) => name == branch,
        None => !reference.starts_with("refs/") && reference == branch,
    }
}

/// A repository location parsed from a git remote URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    /// `host` or `host:port`, for matching configured hosts.
    pub host: String,
    /// SSH ports are independent of the HTTP API port.
    pub is_ssh: bool,
    pub owner: String,
    pub name: String,
}

/// Parse an IronForge remote URL. HTTP(S) clones carry the `/git/` prefix —
/// a GitHub-style `https://host/owner/name` URL is NOT an IronForge remote
/// and returns `None`.
///
/// Supported forms:
/// - `https://[user[:token]@]host[:port]/git/<owner>/<name>[.git]`
/// - `ssh://[user@]host[:port]/<owner>/<name>[.git]`
/// - `[user@]host:<owner>/<name>[.git]` (scp syntax)
pub fn parse_remote_url(raw: &str) -> Option<Remote> {
    let raw = raw.trim();

    if let Some(rest) = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))
    {
        let (authority, path) = rest.split_once('/')?;
        // IronForge HTTP remotes carry the `/git/` path prefix; a
        // GitHub-style `host/owner/name` URL is not one of ours.
        let path = path.strip_prefix("git/")?;
        return from_authority_and_path(strip_credentials(authority), path, false);
    }
    if let Some(rest) = raw.strip_prefix("ssh://") {
        let rest = strip_credentials(rest);
        let (authority, path) = rest.split_once('/')?;
        return from_authority_and_path(authority, path, true);
    }
    // scp syntax: credentials, then a single ':' separating host and path.
    let rest = strip_credentials(raw);
    let (authority, path) = rest.split_once(':')?;
    from_authority_and_path(authority, path, true)
}

/// Drop `user[:password]@` from an authority string.
fn strip_credentials(authority: &str) -> &str {
    match authority.rsplit_once('@') {
        Some((_, host)) => host,
        None => authority,
    }
}

fn from_authority_and_path(authority: &str, path: &str, is_ssh: bool) -> Option<Remote> {
    let authority = authority.trim();
    if authority.is_empty() {
        return None;
    }
    let path = path.trim_end_matches('/');
    let (owner, name) = path.split_once('/')?;
    let name = name.strip_suffix(".git").unwrap_or(name);
    if owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(Remote {
        host: authority.to_string(),
        is_ssh,
        owner: owner.to_string(),
        name: name.to_string(),
    })
}

/// A `git` command for ifg to run. Under `--no-input` it is set up so git
/// cannot wait on a person (see [`apply_noninteractive`]); otherwise it is a
/// plain `git`.
pub fn command() -> Command {
    let mut command = Command::new("git");
    apply_noninteractive(&mut command, crate::input::enabled());
    command
}

/// When `enabled`, keep git and the ssh it starts from asking anyone
/// anything: terminal prompts are off, the askpass program is blanked so no
/// GUI or terminal helper is started, and ssh runs in batch mode, which
/// fails instead of asking for a password or a host key. A `GIT_SSH` or
/// `GIT_SSH_COMMAND` the user set is left alone. When `enabled` is false the
/// command is not touched.
pub fn apply_noninteractive(command: &mut Command, enabled: bool) {
    if !enabled {
        return;
    }
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.env("GIT_ASKPASS", "");
    if std::env::var_os("GIT_SSH_COMMAND").is_none() && std::env::var_os("GIT_SSH").is_none() {
        command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
}

/// Remotes of the current work tree, `origin` first, then alphabetically.
/// Returns an empty vector outside a git work tree.
pub fn git_remotes() -> Vec<(String, Remote)> {
    let output = command()
        .args(["config", "--get-regexp", r"^remote\..*\.url$"])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut remotes: Vec<(String, Remote)> = Vec::new();
    for line in text.lines() {
        let Some((key, url)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let url = url.trim_start();
        let Some(remote_name) = key
            .strip_prefix("remote.")
            .and_then(|rest| rest.strip_suffix(".url"))
        else {
            continue;
        };
        if let Some(remote) = parse_remote_url(url) {
            remotes.push((remote_name.to_string(), remote));
        }
    }
    remotes.sort_by(|a, b| {
        let a_origin = a.0 != "origin";
        let b_origin = b.0 != "origin";
        a_origin.cmp(&b_origin).then_with(|| a.0.cmp(&b.0))
    });
    remotes
}

/// Environment variables that hand git one extra config entry,
/// `http.extraheader = Authorization: Bearer <token>`, through
/// `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_n`/`GIT_CONFIG_VALUE_n` (git ≥ 2.31).
/// Unlike `git -c …`, this keeps the token out of the process arguments,
/// which other local users can read with `ps`. Entries already present in
/// the caller's environment (`existing_count`) are kept by appending after
/// them.
pub fn bearer_header_env(token: &str, existing_count: Option<&str>) -> Vec<(String, String)> {
    let index: usize = existing_count
        .and_then(|count| count.trim().parse().ok())
        .unwrap_or(0);
    vec![
        ("GIT_CONFIG_COUNT".into(), (index + 1).to_string()),
        (format!("GIT_CONFIG_KEY_{index}"), "http.extraheader".into()),
        (
            format!("GIT_CONFIG_VALUE_{index}"),
            format!("Authorization: Bearer {token}"),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names_and_explicit_tag_selectors_remain_distinct() {
        assert!(matches_ref("refs/heads/feature", "feature"));
        assert!(matches_ref("feature", "refs/heads/feature"));
        assert!(!matches_ref("refs/tags/feature", "feature"));
        assert!(matches_ref("refs/tags/feature", "refs/tags/feature"));
        assert!(!matches_branch_ref(
            "refs/tags/feature",
            "refs/tags/feature"
        ));
        assert!(matches_branch_ref(
            "refs/heads/refs/tags/feature",
            "refs/tags/feature"
        ));
    }

    #[test]
    fn noninteractive_env_is_only_added_when_the_switch_is_on() {
        let mut off = Command::new("git");
        apply_noninteractive(&mut off, false);
        assert!(off.get_envs().all(|(key, _)| key != "GIT_TERMINAL_PROMPT"));

        let mut on = Command::new("git");
        apply_noninteractive(&mut on, true);
        let prompt = on
            .get_envs()
            .find(|(key, _)| *key == "GIT_TERMINAL_PROMPT")
            .and_then(|(_, value)| value);
        assert_eq!(prompt, Some(std::ffi::OsStr::new("0")));
    }

    /// Real git against a server that always answers 401. With the switch on
    /// git must fail on its own: it must not start the askpass program the
    /// environment names, and it must not wait for an answer.
    #[cfg(unix)]
    #[test]
    fn git_fails_instead_of_asking_when_switched_on() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant};

        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer);
                let _ = stream.write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"ifg\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });

        let record = dir.path().join("askpass-called");
        let askpass = dir.path().join("askpass.sh");
        std::fs::write(
            &askpass,
            format!(
                "#!/bin/sh\necho called > '{}'\necho secret\n",
                record.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&askpass, std::fs::Permissions::from_mode(0o755)).unwrap();
        let global_config = dir.path().join("gitconfig");
        std::fs::write(&global_config, "").unwrap();

        let mut command = Command::new("git");
        command
            .current_dir(dir.path())
            .args(["ls-remote", &format!("http://{addr}/git/owner/repo.git")])
            .env("GIT_CONFIG_GLOBAL", &global_config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_ASKPASS", &askpass)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        apply_noninteractive(&mut command, true);
        let mut child = command.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                panic!("git kept running instead of failing");
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(!status.success());
        assert!(!record.exists(), "askpass must not be started");
    }

    #[test]
    fn bearer_header_env_starts_at_zero() {
        let env = bearer_header_env("ifp_x", None);
        assert_eq!(
            env,
            vec![
                ("GIT_CONFIG_COUNT".to_string(), "1".to_string()),
                (
                    "GIT_CONFIG_KEY_0".to_string(),
                    "http.extraheader".to_string()
                ),
                (
                    "GIT_CONFIG_VALUE_0".to_string(),
                    "Authorization: Bearer ifp_x".to_string()
                ),
            ]
        );
    }

    #[test]
    fn bearer_header_env_appends_after_existing_entries() {
        let env = bearer_header_env("ifp_x", Some("2"));
        assert_eq!(env[0], ("GIT_CONFIG_COUNT".into(), "3".into()));
        assert_eq!(env[1].0, "GIT_CONFIG_KEY_2");
        assert_eq!(env[2].0, "GIT_CONFIG_VALUE_2");
    }

    fn parse(url: &str) -> Option<Remote> {
        parse_remote_url(url)
    }

    #[test]
    fn https_with_git_prefix() {
        let remote = parse("https://forge.example.com/git/alice/demo.git").unwrap();
        assert_eq!(remote.host, "forge.example.com");
        assert_eq!(remote.owner, "alice");
        assert_eq!(remote.name, "demo");
    }

    #[test]
    fn https_without_dot_git_suffix() {
        let remote = parse("https://forge.example.com/git/alice/demo").unwrap();
        assert_eq!(remote.name, "demo");
    }

    #[test]
    fn https_with_custom_port_and_credentials() {
        let remote =
            parse("https://alice:ifp_secret@forge.example.com:8443/git/alice/demo.git").unwrap();
        assert_eq!(remote.host, "forge.example.com:8443");
        assert_eq!(remote.owner, "alice");
        // The PAT embedded in the URL must not leak into the parsed remote.
        assert!(!format!("{remote:?}").contains("ifp_secret"));
    }

    #[test]
    fn https_github_style_is_not_ironforge() {
        assert!(parse("https://github.com/lengyuqu/ironforge.git").is_none());
        assert!(parse("https://forge.example.com/alice/demo.git").is_none());
    }

    #[test]
    fn ssh_url_form() {
        let remote = parse("ssh://git@git.example.com:2222/alice/demo.git").unwrap();
        assert_eq!(remote.host, "git.example.com:2222");
        assert_eq!(remote.owner, "alice");
        assert_eq!(remote.name, "demo");
    }

    #[test]
    fn ssh_url_without_port() {
        let remote = parse("ssh://git@git.example.com/alice/demo").unwrap();
        assert_eq!(remote.host, "git.example.com");
    }

    #[test]
    fn scp_syntax() {
        let remote = parse("git@git.example.com:alice/demo.git").unwrap();
        assert_eq!(remote.host, "git.example.com");
        assert_eq!(remote.owner, "alice");
        assert_eq!(remote.name, "demo");
    }

    #[test]
    fn scp_syntax_without_user() {
        let remote = parse("git.example.com:alice/demo.git").unwrap();
        assert_eq!(remote.host, "git.example.com");
    }

    #[test]
    fn incomplete_urls_are_rejected() {
        assert!(parse("https://forge.example.com/git/alice").is_none());
        assert!(parse("https://forge.example.com/git/").is_none());
        assert!(parse("git@example.com:").is_none());
        assert!(parse("").is_none());
        assert!(parse("not a url").is_none());
    }
}
