//! `hosts.toml` (design doc §5.1): `$XDG_CONFIG_HOME/ironforge/hosts.toml`,
//! falling back to `$HOME/.config/ironforge/hosts.toml`. Written with 0600
//! permissions; a file with wider permissions is loaded but its tokens are
//! refused (`*.ai/guardrails.md` credential policy).

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_host: Option<String>,
    /// Token storage backend: `"file"` (default) or `"keyring"` (D6 — the
    /// PAT lives in the system keychain and never in this file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_store: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hosts: BTreeMap<String, HostEntry>,
}

impl Config {
    /// Whether tokens belong in the system keychain instead of the file.
    pub fn uses_keyring(&self) -> bool {
        self.token_store.as_deref() == Some("keyring")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostEntry {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_protocol: Option<String>,
}

impl HostEntry {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            user: None,
            token: None,
            git_protocol: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("config parse error in {path}: {message}")]
    Parse { path: PathBuf, message: String },
}

/// The config directory: `$XDG_CONFIG_HOME/ironforge`, else
/// `$HOME/.config/ironforge`. Deliberately not `dirs::config_dir()`, which
/// would return `~/Library/Application Support` on macOS (design doc §5.2).
pub fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            let home = std::env::var_os("HOME").filter(|value| !value.is_empty())?;
            Some(PathBuf::from(home).join(".config"))
        })?;
    Some(base.join("ironforge"))
}

pub fn config_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("hosts.toml"))
}

/// Load the config. A missing file yields an empty config. When the file is
/// more permissive than 0600, a warning is printed and tokens are dropped.
pub fn load() -> Result<Config, ConfigError> {
    let Some(path) = config_path() else {
        return Ok(Config::default());
    };
    if !path.exists() {
        return Ok(Config::default());
    }
    let insecure = has_group_or_other_permissions(&path);
    let text = fs::read_to_string(&path)?;
    let mut config: Config = toml::from_str(&text).map_err(|e| ConfigError::Parse {
        path: path.clone(),
        message: e.to_string(),
    })?;
    if insecure {
        eprintln!(
            "ifg: warning: {} is readable by others; tokens were not loaded. \
             Run `chmod 600 {}`.",
            path.display(),
            path.display()
        );
        for entry in config.hosts.values_mut() {
            entry.token = None;
        }
    }
    // Keychain-backed tokens: the file only carries a marker-free entry.
    if config.uses_keyring() {
        for (key, entry) in config.hosts.iter_mut() {
            if let Some(token) = crate::keyring::load(key) {
                entry.token = Some(token);
            }
        }
    }
    Ok(config)
}

/// Write the config, creating parent directories, with 0600 permissions.
pub fn save(config: &Config) -> Result<(), ConfigError> {
    let Some(path) = config_path() else {
        return Err(ConfigError::Io(std::io::Error::other(
            "neither XDG_CONFIG_HOME nor HOME is set",
        )));
    };
    let Some(dir) = path.parent() else {
        return Err(ConfigError::Io(std::io::Error::other(
            "invalid config path",
        )));
    };
    fs::create_dir_all(dir)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)?;
    #[cfg(unix)]
    tighten_permissions(&path)?;
    let text = toml::to_string_pretty(config).map_err(|e| ConfigError::Parse {
        path: path.clone(),
        message: e.to_string(),
    })?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

/// Config key for a URL: `host` or `host:port` (non-default ports only).
pub fn host_key_for_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url.trim()).ok()?;
    let host = parsed.host_str()?.to_string();
    let default_port = match parsed.scheme() {
        "http" => 80,
        "https" => 443,
        _ => return Some(host),
    };
    match parsed.port() {
        Some(port) if port != default_port => Some(format!("{host}:{port}")),
        _ => Some(host),
    }
}

/// Whether the resolved (or given) host name matches this entry.
pub fn entry_matches_url(entry_url: &str, host_name: &str) -> bool {
    host_key_for_url(entry_url).as_deref() == Some(host_name)
}

#[cfg(unix)]
fn has_group_or_other_permissions(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|meta| meta.permissions().mode() & 0o077 != 0)
        .unwrap_or(false)
}

#[cfg(unix)]
fn tighten_permissions(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if fs::metadata(path)?.permissions().mode() & 0o077 != 0 {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_key_uses_port_only_when_non_default() {
        assert_eq!(
            host_key_for_url("https://forge.example.com"),
            Some("forge.example.com".into())
        );
        assert_eq!(
            host_key_for_url("https://forge.example.com:8443/git/a/b"),
            Some("forge.example.com:8443".into())
        );
        assert_eq!(
            host_key_for_url("http://127.0.0.1:8080"),
            Some("127.0.0.1:8080".into())
        );
        assert_eq!(host_key_for_url("not a url"), None);
    }

    #[test]
    fn entry_matches_by_host_key() {
        assert!(entry_matches_url(
            "https://forge.example.com",
            "forge.example.com"
        ));
        assert!(entry_matches_url(
            "https://forge.example.com:8443/forge/",
            "forge.example.com:8443"
        ));
        assert!(!entry_matches_url(
            "https://other.example.com",
            "forge.example.com"
        ));
    }

    #[test]
    fn roundtrip_serialization() {
        let config = Config {
            default_host: Some("forge.example.com".into()),
            token_store: None,
            hosts: [(
                "forge.example.com".to_string(),
                HostEntry {
                    url: "https://forge.example.com".into(),
                    user: Some("alice".into()),
                    token: Some("ifp_x".into()),
                    git_protocol: Some("https".into()),
                },
            )]
            .into_iter()
            .collect(),
        };
        let text = toml::to_string_pretty(&config).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.default_host.as_deref(), Some("forge.example.com"));
        assert_eq!(
            back.hosts["forge.example.com"].token.as_deref(),
            Some("ifp_x")
        );
    }
}
