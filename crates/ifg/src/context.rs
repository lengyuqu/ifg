//! Host context resolution (design doc §5.2).
//!
//! Priority: `--host` > `IRONFORGE_URL` + `IRONFORGE_PAT` > git remote
//! inference (must match a configured host) > `default_host`.
//! `IRONFORGE_PAT` without `IRONFORGE_URL` binds the token to whichever host
//! is resolved; `IRONFORGE_URL` without `IRONFORGE_PAT` takes the token from
//! the config entry of that host.

use ifg_client::Secret;

use crate::config::{self, Config};
use crate::git;

/// The host a command talks to.
#[derive(Debug, Clone)]
pub struct ResolvedHost {
    /// Config key, when the host is (or became) a configured one.
    pub name: Option<String>,
    pub url: String,
    pub token: Option<Secret>,
}

pub fn env_url() -> Option<String> {
    std::env::var("IRONFORGE_URL")
        .ok()
        .filter(|value| !value.is_empty())
}

pub fn env_pat() -> Option<String> {
    std::env::var("IRONFORGE_PAT")
        .ok()
        .filter(|value| !value.is_empty())
}

pub fn is_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn normalize_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

/// Resolve the host for a command that needs one (everything but
/// `auth login`/`auth logout`, which manage the config directly).
pub fn resolve_host(
    host_arg: Option<&str>,
    repo_arg: Option<&str>,
    config: &Config,
) -> Result<ResolvedHost, crate::error::AppError> {
    let env_pat = env_pat().map(Secret::new);

    // 1. --host: a name from the config or a raw URL.
    if let Some(host_arg) = host_arg {
        if is_url(host_arg) {
            let url = normalize_url(host_arg);
            let token = env_pat.or_else(|| token_from_url(config, &url));
            return Ok(ResolvedHost {
                name: config::host_key_for_url(&url),
                url,
                token,
            });
        }
        let entry = config.hosts.get(host_arg).ok_or_else(|| {
            crate::error::AppError::Other(format!(
                "unknown host '{host_arg}'; configured: {}. Use `ifg auth login --host <url>`.",
                configured_names(config)
            ))
        })?;
        let resolved = ResolvedHost {
            name: Some(host_arg.to_string()),
            url: entry.url.clone(),
            token: env_pat.or_else(|| entry.token.clone().map(Secret::new)),
        };
        return Ok(resolved);
    }

    // 2. IRONFORGE_URL (+ optional IRONFORGE_PAT).
    if let Some(env_url) = env_url() {
        let url = normalize_url(&env_url);
        let token = env_pat.or_else(|| token_from_url(config, &url));
        return Ok(ResolvedHost {
            name: config::host_key_for_url(&url),
            url,
            token,
        });
    }

    // 2.5. `-R https://host/git/owner/name` also decides the host.
    if let Some(repo_arg) = repo_arg {
        if let Some(remote) = git::parse_remote_url(repo_arg) {
            if let Some(resolved) = from_configured_host(config, &remote, env_pat.clone())? {
                return Ok(resolved);
            }
        }
    }

    // 3. Git remote inference: the remote's host must be a configured one.
    for (_, remote) in git::git_remotes() {
        if let Some(resolved) = from_configured_host(config, &remote, env_pat.clone())? {
            return Ok(resolved);
        }
    }

    // 4. default_host.
    if let Some(default) = &config.default_host {
        if let Some(entry) = config.hosts.get(default) {
            return Ok(ResolvedHost {
                name: Some(default.clone()),
                url: entry.url.clone(),
                token: env_pat.or_else(|| entry.token.clone().map(Secret::new)),
            });
        }
    }

    Err(crate::error::AppError::Other(
        "no IronForge host configured. Set --host, IRONFORGE_URL, or run \
         `ifg auth login --host <url>`."
            .into(),
    ))
}

fn from_configured_host(
    config: &Config,
    remote: &git::Remote,
    env_pat: Option<Secret>,
) -> Result<Option<ResolvedHost>, crate::error::AppError> {
    let mut matches = config
        .hosts
        .iter()
        .filter(|(_, entry)| remote_matches_url(remote, &entry.url));
    let Some((name, entry)) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        return Err(crate::error::AppError::Other(format!(
            "multiple configured hosts match SSH remote {}; select one with --host",
            remote.host
        )));
    }
    Ok(Some(ResolvedHost {
        name: Some(name.clone()),
        url: entry.url.clone(),
        token: env_pat.or_else(|| entry.token.clone().map(Secret::new)),
    }))
}

fn remote_matches_url(remote: &git::Remote, entry_url: &str) -> bool {
    if !remote.is_ssh {
        return config::entry_matches_url(entry_url, &remote.host);
    }
    let Ok(ssh) = url::Url::parse(&format!("ssh://{}", remote.host)) else {
        return false;
    };
    let Ok(api) = url::Url::parse(entry_url) else {
        return false;
    };
    match (ssh.host_str(), api.host_str()) {
        (Some(ssh), Some(api)) => ssh.eq_ignore_ascii_case(api),
        _ => false,
    }
}

fn token_from_url(config: &Config, url: &str) -> Option<Secret> {
    let key = config::host_key_for_url(url)?;
    config
        .hosts
        .get(&key)
        .and_then(|entry| entry.token.clone())
        .map(Secret::new)
}

fn configured_names(config: &Config) -> String {
    if config.hosts.is_empty() {
        "(none)".to_string()
    } else {
        config.hosts.keys().cloned().collect::<Vec<_>>().join(", ")
    }
}

/// The repository a command operates on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoContext {
    pub owner: String,
    pub name: String,
}

impl std::fmt::Display for RepoContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

/// Resolve the repository context: `-R OWNER/NAME` (or a full remote URL),
/// else the first git remote (origin first) whose host matches the resolved
/// host (design doc §5.2).
pub fn resolve_repo(
    repo_arg: Option<&str>,
    resolved_host: &ResolvedHost,
) -> Result<RepoContext, crate::error::AppError> {
    if let Some(repo_arg) = repo_arg {
        if is_url(repo_arg)
            || repo_arg.starts_with("ssh://")
            || (repo_arg.contains(':') && git::parse_remote_url(repo_arg).is_some())
        {
            let remote = crate::git::parse_remote_url(repo_arg).ok_or_else(|| {
                crate::error::AppError::Usage(format!(
                    "cannot parse '{repo_arg}' as an IronForge repository URL \
                     (expected https://host/git/OWNER/NAME)"
                ))
            })?;
            validate_repo_component(&remote.owner, "owner")?;
            validate_repo_component(&remote.name, "repository name")?;
            return Ok(RepoContext {
                owner: remote.owner,
                name: remote.name,
            });
        }
        let Some((owner, name)) = repo_arg.split_once('/') else {
            return Err(crate::error::AppError::Usage(format!(
                "expected OWNER/NAME, got '{repo_arg}'"
            )));
        };
        if owner.is_empty() || name.is_empty() {
            return Err(crate::error::AppError::Usage(format!(
                "expected OWNER/NAME, got '{repo_arg}'"
            )));
        }
        validate_repo_component(owner, "owner")?;
        validate_repo_component(name, "repository name")?;
        return Ok(RepoContext {
            owner: owner.to_string(),
            name: name.to_string(),
        });
    }

    for (_, remote) in crate::git::git_remotes() {
        if remote_matches_url(&remote, &resolved_host.url) {
            return Ok(RepoContext {
                owner: remote.owner,
                name: remote.name,
            });
        }
    }
    Err(crate::error::AppError::Usage(
        "no repository context: pass -R OWNER/NAME or run inside a git clone".into(),
    ))
}

/// A repository name that would change the request route is not a name: `/`,
/// `\`, `?` and `#` end or split the path, and `.`/`..` are normalized away by
/// the URL parser. Rejecting them here turns a silent route escape into a
/// usage error; `repo_path` still encodes the segments as a second layer.
fn validate_repo_component(value: &str, label: &str) -> Result<(), crate::error::AppError> {
    let invalid = value.is_empty()
        || matches!(value, "." | "..")
        || value.contains(['/', '\\', '?', '#', '\0']);
    if invalid {
        return Err(crate::error::AppError::Usage(format!(
            "invalid {label} '{value}': / \\ ? # and dot segments are not allowed"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_ports_match_api_hosts_but_http_ports_remain_distinct() {
        let ssh = git::parse_remote_url("ssh://git@forge.example.com:2222/alice/demo.git").unwrap();
        assert!(remote_matches_url(&ssh, "https://forge.example.com:8443"));
        let scp = git::parse_remote_url("git@forge.example.com:alice/demo.git").unwrap();
        assert!(remote_matches_url(&scp, "https://forge.example.com:8443"));
        let http =
            git::parse_remote_url("https://forge.example.com:8443/git/alice/demo.git").unwrap();
        assert!(!remote_matches_url(&http, "https://forge.example.com:9443"));
        assert!(remote_matches_url(&http, "https://forge.example.com:8443"));
    }

    #[test]
    fn ssh_host_inference_rejects_ambiguous_configurations() {
        let mut config = Config::default();
        config.hosts.insert(
            "one".into(),
            crate::config::HostEntry::new("https://forge.example.com:8443"),
        );
        let ssh = git::parse_remote_url("ssh://git@forge.example.com:2222/alice/demo.git").unwrap();
        assert_eq!(
            from_configured_host(&config, &ssh, None)
                .unwrap()
                .unwrap()
                .name
                .as_deref(),
            Some("one")
        );
        config.hosts.insert(
            "two".into(),
            crate::config::HostEntry::new("https://forge.example.com:9443"),
        );
        assert!(from_configured_host(&config, &ssh, None)
            .unwrap_err()
            .to_string()
            .contains("--host"));
    }

    #[test]
    fn explicit_ssh_and_scp_repository_urls_are_parsed() {
        let host = ResolvedHost {
            name: None,
            url: "https://forge.example.com".into(),
            token: None,
        };
        for input in [
            "ssh://git@forge.example.com:2222/alice/demo.git",
            "git@forge.example.com:alice/demo.git",
        ] {
            let repo = resolve_repo(Some(input), &host).unwrap();
            assert_eq!(repo.owner, "alice");
            assert_eq!(repo.name, "demo");
        }
    }

    #[test]
    fn route_changing_repository_names_are_rejected() {
        let host = ResolvedHost {
            name: None,
            url: "https://forge.example.com".into(),
            token: None,
        };
        for input in [
            "../users",
            "a/../../users",
            "a/b?per_page=1",
            "a/b#frag",
            "a/..",
            "a/.",
            "a/b\\c",
        ] {
            let error =
                resolve_repo(Some(input), &host).expect_err(&format!("{input:?} must be rejected"));
            assert_eq!(error.exit_code(), crate::error::EXIT_USAGE, "{input:?}");
        }
        // Ordinary names, including dots and dashes, still resolve.
        for (input, owner, name) in [
            ("alice/demo", "alice", "demo"),
            ("a.b/c-d_e", "a.b", "c-d_e"),
        ] {
            let repo = resolve_repo(Some(input), &host).unwrap();
            assert_eq!((repo.owner.as_str(), repo.name.as_str()), (owner, name));
        }
    }
}
