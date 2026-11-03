//! Git remote handling (design doc §5.2, §5.3): everything goes through the
//! system `git` binary, no gix/libgit2 dependency. Only the URL parsing is
//! needed in M1 — for host inference from inside a work tree.

use std::process::Command;

/// A repository location parsed from a git remote URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    /// `host` or `host:port`, for matching configured hosts.
    pub host: String,
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
        return from_authority_and_path(strip_credentials(authority), path);
    }
    if let Some(rest) = raw.strip_prefix("ssh://") {
        let rest = strip_credentials(rest);
        let (authority, path) = rest.split_once('/')?;
        return from_authority_and_path(authority, path);
    }
    // scp syntax: credentials, then a single ':' separating host and path.
    let rest = strip_credentials(raw);
    let (authority, path) = rest.split_once(':')?;
    from_authority_and_path(authority, path)
}

/// Drop `user[:password]@` from an authority string.
fn strip_credentials(authority: &str) -> &str {
    match authority.rsplit_once('@') {
        Some((_, host)) => host,
        None => authority,
    }
}

fn from_authority_and_path(authority: &str, path: &str) -> Option<Remote> {
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
        owner: owner.to_string(),
        name: name.to_string(),
    })
}

/// Remotes of the current work tree, `origin` first, then alphabetically.
/// Returns an empty vector outside a git work tree.
pub fn git_remotes() -> Vec<(String, Remote)> {
    let output = Command::new("git")
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
        let Some((key, url)) = line.split_once(' ') else {
            continue;
        };
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

#[cfg(test)]
mod tests {
    use super::*;

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
