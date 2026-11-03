//! Minimum supported server version and comparison helpers (design doc D10).
//!
//! The client checks `/health.version` and warns when the server is older
//! than this floor. Server *features* are detected by which endpoints answer
//! — the version gate exists to tell people to upgrade, not to switch code
//! paths (DTOs are permissive on purpose).

/// Oldest IronForge this client is tested against (the contract snapshot
/// baseline plus the `per_page` and fork-PR fixes).
pub const MIN_SERVER_VERSION: &str = "0.2.0";

/// Compare dotted versions; missing components count as zero. Non-numeric
/// suffixes (e.g. `-rc1`) are ignored.
pub fn compare(left: &str, right: &str) -> std::cmp::Ordering {
    let parse = |text: &str| -> Vec<u64> {
        text.split(['.', '-', '+'])
            .take(3)
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (left, right) = (parse(left), parse(right));
    for index in 0..3 {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        match a.cmp(&b) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// `Some(message)` when `server` is older than [`MIN_SERVER_VERSION`].
pub fn outdated(server: &str) -> Option<String> {
    if compare(server, MIN_SERVER_VERSION) == std::cmp::Ordering::Less {
        Some(format!(
            "server {server} is older than the supported minimum {MIN_SERVER_VERSION}; upgrade IronForge"
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_dotted_versions() {
        assert_eq!(compare("0.2.0", "0.2.0"), std::cmp::Ordering::Equal);
        assert_eq!(compare("0.10.0", "0.2.0"), std::cmp::Ordering::Greater);
        assert_eq!(compare("1.0", "0.9.9"), std::cmp::Ordering::Greater);
        assert_eq!(compare("0.1.9", "0.2.0"), std::cmp::Ordering::Less);
        assert_eq!(compare("0.2.0-rc1", "0.2.0"), std::cmp::Ordering::Equal);
    }

    #[test]
    fn flags_old_servers_only() {
        assert!(outdated("0.1.0").is_some());
        assert!(outdated("0.2.0").is_none());
        assert!(outdated("0.3.1").is_none());
    }
}
