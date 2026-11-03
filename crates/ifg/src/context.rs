//! Host context resolution (design doc §5.2).
//!
//! Priority: `--host` > `IRONFORGE_URL` + `IRONFORGE_PAT` > git remote
//! inference (must match a configured host) > `default_host`.
//! `IRONFORGE_PAT` without `IRONFORGE_URL` binds the token to whichever host
//! is resolved; `IRONFORGE_URL` without `IRONFORGE_PAT` takes the token from
//! the config entry of that host.

use ifg_client::Secret;

use crate::config::{self, Config, HostEntry};
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
        if is_url(repo_arg) {
            if let Some(remote) = git::parse_remote_url(repo_arg) {
                if let Some(resolved) = from_configured_host(config, &remote.host, env_pat.clone())
                {
                    return Ok(resolved);
                }
            }
        }
    }

    // 3. Git remote inference: the remote's host must be a configured one.
    for (_, remote) in git::git_remotes() {
        if let Some(resolved) = from_configured_host(config, &remote.host, env_pat.clone()) {
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
    host_name: &str,
    env_pat: Option<Secret>,
) -> Option<ResolvedHost> {
    let (name, entry): (&String, &HostEntry) = config
        .hosts
        .iter()
        .find(|(_, entry)| config::entry_matches_url(&entry.url, host_name))?;
    Some(ResolvedHost {
        name: Some(name.clone()),
        url: entry.url.clone(),
        token: env_pat.or_else(|| entry.token.clone().map(Secret::new)),
    })
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
        if is_url(repo_arg) {
            let remote = crate::git::parse_remote_url(repo_arg).ok_or_else(|| {
                crate::error::AppError::Usage(format!(
                    "cannot parse '{repo_arg}' as an IronForge repository URL \
                     (expected https://host/git/OWNER/NAME)"
                ))
            })?;
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
        return Ok(RepoContext {
            owner: owner.to_string(),
            name: name.to_string(),
        });
    }

    for (_, remote) in crate::git::git_remotes() {
        if config::entry_matches_url(&resolved_host.url, &remote.host) {
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
