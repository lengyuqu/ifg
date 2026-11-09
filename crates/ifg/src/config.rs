//! `hosts.toml` (design doc §5.1): `$XDG_CONFIG_HOME/ironforge/hosts.toml`,
//! else the platform-native config directory (`$HOME/.config` on POSIX,
//! `%APPDATA%` on Windows). On POSIX it is written with 0600 permissions; a
//! file with wider permissions is loaded but its tokens are refused
//! (`*.ai/guardrails.md` credential policy).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_host: Option<String>,
    /// Token storage backend: `"file"` (default) or `"keyring"` (D6 — the
    /// PAT lives in the system keychain and never in this file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_store: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hosts: BTreeMap<String, HostEntry>,
    /// Host keys whose token was read from the keychain in this process.
    /// They must never be written back: the file would then hold a plaintext
    /// copy of a secret the user asked to keep in the keychain.
    #[serde(skip)]
    keyring_tokens: BTreeSet<String>,
}

impl Config {
    /// Whether tokens belong in the system keychain instead of the file.
    pub fn uses_keyring(&self) -> bool {
        self.token_store.as_deref() == Some("keyring")
    }

    /// Keep `token` in the file for this host. Used when the keychain is
    /// selected but unavailable, so the fallback is not cleared by
    /// [`for_disk`] on save.
    pub fn set_file_token(&mut self, key: &str, token: String) {
        if let Some(entry) = self.hosts.get_mut(key) {
            entry.token = Some(token);
        }
        self.keyring_tokens.remove(key);
    }

    /// Drop this host's token from the in-memory config as well as from the
    /// set that must never be written back.
    pub fn clear_token(&mut self, key: &str) {
        if let Some(entry) = self.hosts.get_mut(key) {
            entry.token = None;
        }
        self.keyring_tokens.remove(key);
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

/// The config directory: `$XDG_CONFIG_HOME/ironforge` when set, else the
/// platform-native location — `%APPDATA%\ironforge` on Windows,
/// `$HOME/.config/ironforge` elsewhere. Deliberately not `dirs::config_dir()`,
/// which would return `~/Library/Application Support` on macOS (design doc §5.2).
pub fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(native_config_base)?;
    Some(base.join("ironforge"))
}

/// `%APPDATA%` is the Windows convention and is set for every interactive
/// user; `HOME` usually is not, so without this fallback `auth login` could
/// not persist anything from PowerShell or cmd.
#[cfg(windows)]
fn native_config_base() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(not(windows))]
fn native_config_base() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|value| !value.is_empty())?;
    Some(PathBuf::from(home).join(".config"))
}

pub fn config_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("hosts.toml"))
}

/// Load the config. A missing file yields an empty config. When the file is
/// more permissive than 0600, a warning is printed and tokens are dropped.
pub fn load() -> Result<Config, ConfigError> {
    match config_path() {
        Some(path) => load_from(&path),
        None => Ok(Config::default()),
    }
}

fn load_from(path: &Path) -> Result<Config, ConfigError> {
    load_with_keyring(path, crate::keyring::load)
}

fn load_with_keyring(
    path: &Path,
    mut keyring_token: impl FnMut(&str) -> Option<String>,
) -> Result<Config, ConfigError> {
    if !path.exists() {
        return Ok(Config::default());
    }
    let insecure = has_group_or_other_permissions(path);
    let text = fs::read_to_string(path)?;
    let mut config: Config = toml::from_str(&text).map_err(|e| ConfigError::Parse {
        path: path.to_path_buf(),
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
    // Keychain-backed tokens: the file only carries a marker-free entry. The
    // keys are remembered so a later save never writes them back to disk.
    if config.uses_keyring() {
        let keys: Vec<String> = config.hosts.keys().cloned().collect();
        for key in keys {
            // An explicit file token is the durable fallback after a failed
            // keychain write. A recovered keychain may still contain the old
            // token, which must not override or erase that fallback.
            if config.hosts[&key].token.is_some() {
                continue;
            }
            if let Some(token) = keyring_token(&key) {
                if let Some(entry) = config.hosts.get_mut(&key) {
                    entry.token = Some(token);
                }
                config.keyring_tokens.insert(key);
            }
        }
    }
    Ok(config)
}

/// Read-modify-write under an exclusive lock. Two `ifg auth` runs that touch
/// different hosts must not drop each other's entries: the load and the write
/// happen inside one lock, and the write is atomic.
///
/// The lock is a separate `.hosts.toml.lock` file because the rename in
/// [`write_atomic`] replaces the inode the config lock would be held on.
pub fn update<T>(
    change: impl FnOnce(&mut Config) -> Result<T, ConfigError>,
) -> Result<T, ConfigError> {
    let (dir, path) = config_locations()?;
    fs::create_dir_all(&dir)?;
    update_at(&dir, &path, change)
}

fn update_at<T>(
    dir: &Path,
    path: &Path,
    change: impl FnOnce(&mut Config) -> Result<T, ConfigError>,
) -> Result<T, ConfigError> {
    let _lock = lock(dir)?;
    let mut config = load_from(path)?;
    let value = change(&mut config)?;
    write_atomic(dir, path, &config)?;
    Ok(value)
}

fn config_locations() -> Result<(PathBuf, PathBuf), ConfigError> {
    let path = config_path().ok_or_else(|| {
        ConfigError::Io(std::io::Error::other(if cfg!(windows) {
            "neither XDG_CONFIG_HOME nor APPDATA is set"
        } else {
            "neither XDG_CONFIG_HOME nor HOME is set"
        }))
    })?;
    let dir = path
        .parent()
        .ok_or_else(|| ConfigError::Io(std::io::Error::other("invalid config path")))?
        .to_path_buf();
    Ok((dir, path))
}

fn lock(dir: &Path) -> Result<fs::File, ConfigError> {
    let path = dir.join(".hosts.toml.lock");
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
    }
    // Blocking: a second `ifg auth` waits for the first to finish rather than
    // clobbering it. The lock is released when `file` is dropped.
    file.lock()?;
    Ok(file)
}

fn write_atomic(dir: &Path, path: &Path, config: &Config) -> Result<(), ConfigError> {
    let disk = for_disk(config);
    let text = toml::to_string_pretty(&disk).map_err(|e| ConfigError::Parse {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    let mut temp = tempfile::Builder::new()
        .prefix(".hosts-")
        .suffix(".toml")
        .tempfile_in(dir)?;
    // `tempfile` creates 0600 already; set it explicitly so the guarantee does
    // not depend on that detail.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    temp.write_all(text.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|error| ConfigError::Io(error.error))?;
    Ok(())
}

/// The copy that may be written to disk: tokens loaded from the keychain are
/// cleared, because the file must never hold a secret the user put in the
/// keychain (a `logout` of one host used to persist every other host's PAT).
fn for_disk(config: &Config) -> Config {
    let mut disk = config.clone();
    for key in &disk.keyring_tokens {
        if let Some(entry) = disk.hosts.get_mut(key) {
            entry.token = None;
        }
    }
    disk.keyring_tokens.clear();
    disk
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

/// Stored credentials belong to one HTTP origin and deployment base path.
/// A host/port key alone cannot authorize a downgrade from HTTPS to HTTP.
pub fn same_base_url(left: &str, right: &str) -> bool {
    let (Ok(left), Ok(right)) = (url::Url::parse(left), url::Url::parse(right)) else {
        return false;
    };
    matches!(left.scheme(), "http" | "https")
        && left.origin() == right.origin()
        && left.path().trim_end_matches('/') == right.path().trim_end_matches('/')
}

#[cfg(unix)]
fn has_group_or_other_permissions(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|meta| meta.permissions().mode() & 0o077 != 0)
        .unwrap_or(false)
}

// Windows ACLs are not POSIX mode bits. Access is governed by the ACL inherited
// from the user profile directory, which is private to the user by default, so
// the group/other warning does not apply and no tightening is needed. Returning
// false (rather than true) keeps the warning silent instead of pretending the
// file was checked.
#[cfg(not(unix))]
fn has_group_or_other_permissions(_path: &std::path::Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercise recovery against an actual native keychain entry. The write
    /// failure itself is represented by the same durable fallback state as
    /// auth login; no user's keychain settings or credentials are changed.
    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn native_file_fallback_recovery() {
        #[cfg(target_os = "macos")]
        if std::env::var_os("IFG_KEYRING_TEST").is_none() {
            return;
        }
        struct EntryCleanup(String);
        impl Drop for EntryCleanup {
            fn drop(&mut self) {
                let _ = crate::keyring::delete(&self.0);
            }
        }
        let host = format!("ifg-native-fallback-{}", std::process::id());
        let _cleanup = EntryCleanup(host.clone());
        crate::keyring::store(&host, "old-native-test-credential").unwrap();
        assert_eq!(
            crate::keyring::load(&host).as_deref(),
            Some("old-native-test-credential")
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.toml");
        let mut config = Config {
            token_store: Some("keyring".into()),
            ..Default::default()
        };
        config
            .hosts
            .insert(host.clone(), HostEntry::new("https://native-test.invalid"));
        config.set_file_token(&host, "new-native-test-credential".into());
        write_atomic(dir.path(), &path, &config).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(
            loaded.hosts[&host].token.as_deref(),
            Some("new-native-test-credential")
        );
        write_atomic(dir.path(), &path, &loaded).unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("new-native-test-credential"));
        crate::keyring::store(&host, "new-native-test-credential").unwrap();
        let mut config = loaded;
        config.clear_token(&host);
        write_atomic(dir.path(), &path, &config).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(
            loaded.hosts[&host].token.as_deref(),
            Some("new-native-test-credential")
        );
        write_atomic(dir.path(), &path, &loaded).unwrap();
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("new-native-test-credential"));
        drop(_cleanup);
        assert!(crate::keyring::load(&host).is_none());
    }

    #[test]
    fn file_fallback_survives_keychain_recovery_and_config_updates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.toml");
        let mut config = Config {
            token_store: Some("keyring".into()),
            ..Default::default()
        };
        config
            .hosts
            .insert("forge".into(), HostEntry::new("https://forge.example.com"));
        config.set_file_token("forge", "new-token".into());
        write_atomic(dir.path(), &path, &config).unwrap();

        let loaded =
            load_with_keyring(&path, |_| panic!("old keychain token must not be loaded")).unwrap();
        assert_eq!(loaded.hosts["forge"].token.as_deref(), Some("new-token"));
        write_atomic(dir.path(), &path, &loaded).unwrap();
        let loaded = load_with_keyring(&path, |_| Some("old-token".into())).unwrap();
        assert_eq!(loaded.hosts["forge"].token.as_deref(), Some("new-token"));

        let mut keychain_only = loaded;
        keychain_only.clear_token("forge");
        write_atomic(dir.path(), &path, &keychain_only).unwrap();
        let loaded = load_with_keyring(&path, |_| Some("keychain-token".into())).unwrap();
        assert_eq!(
            loaded.hosts["forge"].token.as_deref(),
            Some("keychain-token")
        );
        assert!(for_disk(&loaded).hosts["forge"].token.is_none());
    }

    #[test]
    fn credential_scope_includes_protocol_port_and_base_path() {
        assert!(same_base_url(
            "https://forge.example.com/forge/",
            "https://forge.example.com:443/forge"
        ));
        assert!(!same_base_url(
            "https://forge.example.com",
            "http://forge.example.com"
        ));
        assert!(!same_base_url(
            "https://forge.example.com:8443",
            "https://forge.example.com:9443"
        ));
        assert!(!same_base_url(
            "https://forge.example.com/one",
            "https://forge.example.com/two"
        ));
    }

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

    #[cfg(unix)]
    #[test]
    fn write_atomic_creates_a_private_file_and_no_litter() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.toml");
        let mut config = Config::default();
        config
            .hosts
            .insert("h".into(), HostEntry::new("https://h.example.com"));

        write_atomic(dir.path(), &path, &config).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "token files must not be group/other readable");
        assert!(!has_group_or_other_permissions(&path));
        assert!(load_from(&path).unwrap().hosts.contains_key("h"));

        let leftovers: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "hosts.toml")
            .collect();
        assert!(leftovers.is_empty(), "leftover files: {leftovers:?}");
    }

    /// A token read from the keychain must never be written back: `logout` of
    /// one host used to persist every other host's PAT in plaintext.
    #[test]
    fn keyring_tokens_never_reach_the_file() {
        let mut config = Config {
            token_store: Some("keyring".into()),
            ..Default::default()
        };
        let mut entry = HostEntry::new("https://a.example.com");
        entry.token = Some("ifp_from_keyring".into());
        config.hosts.insert("a".into(), entry);
        config.keyring_tokens.insert("a".into());

        let disk = for_disk(&config);
        assert!(disk.hosts["a"].token.is_none());
        // The in-memory copy keeps the token for the rest of this run.
        assert_eq!(config.hosts["a"].token.as_deref(), Some("ifp_from_keyring"));
    }

    /// Two writers that touch different hosts must both survive: the read and
    /// the write happen under one lock, so the second one cannot overwrite the
    /// first with a stale snapshot.
    #[test]
    fn concurrent_updates_keep_every_host() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts.toml");
        let spawn = |prefix: &'static str, start: usize| {
            let dir = dir.path().to_path_buf();
            let path = path.clone();
            std::thread::spawn(move || {
                for index in start..start + 10 {
                    update_at(&dir, &path, |config| {
                        config.hosts.insert(
                            format!("{prefix}{index}"),
                            HostEntry::new("https://example.com"),
                        );
                        Ok(())
                    })
                    .unwrap();
                }
            })
        };
        let first = spawn("a", 0);
        let second = spawn("b", 10);
        first.join().unwrap();
        second.join().unwrap();

        let config = load_from(&path).unwrap();
        assert_eq!(
            config.hosts.len(),
            20,
            "lost updates: {:?}",
            config.hosts.keys().collect::<Vec<_>>()
        );
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
            keyring_tokens: Default::default(),
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
