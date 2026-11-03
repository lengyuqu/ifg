//! Contract drift test (design doc §6.1): every path in
//! [`ifg_client::USED_PATHS`] must exist in the newest OpenAPI snapshot
//! (`fixtures/openapi/openapi-*.json`), or be explicitly registered in
//! `fixtures/openapi/uncovered.txt`. A path that becomes covered but stays
//! in `uncovered.txt` also fails, so the lists cannot rot.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use ifg_client::USED_PATHS;
use serde_json::Value;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/openapi")
        .canonicalize()
        .expect("fixtures/openapi exists")
}

fn newest_snapshot(dir: &Path) -> PathBuf {
    let mut snapshots: Vec<PathBuf> = fs::read_dir(dir)
        .expect("read fixtures/openapi")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("openapi-v") && name.ends_with(".json"))
                .unwrap_or(false)
        })
        .collect();
    assert!(
        !snapshots.is_empty(),
        "no OpenAPI snapshot in {}; run scripts/sync-openapi.sh <url>",
        dir.display()
    );
    snapshots.sort();
    snapshots.pop().expect("at least one snapshot")
}

fn uncovered_entries(dir: &Path) -> HashSet<String> {
    let path = dir.join("uncovered.txt");
    let text = fs::read_to_string(&path).unwrap_or_default();
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

#[test]
fn used_paths_still_exist_in_the_openapi_snapshot() {
    let dir = fixtures_dir();
    let snapshot_path = newest_snapshot(&dir);
    let spec: Value =
        serde_json::from_str(&fs::read_to_string(&snapshot_path).expect("readable snapshot"))
            .expect("valid JSON snapshot");
    let uncovered = uncovered_entries(&dir);

    let mut drift: Vec<String> = Vec::new();
    for (method, path) in USED_PATHS {
        let key = format!("{method} {path}");
        let covered = spec["paths"]
            .get(*path)
            .and_then(|item| item.get(method.to_lowercase()))
            .is_some();
        if covered {
            if uncovered.contains(&key) {
                drift.push(format!(
                    "{key}: covered by the snapshot now; remove it from uncovered.txt"
                ));
            }
        } else if !uncovered.contains(&key) {
            drift.push(format!(
                "{key}: missing from {} and not registered in uncovered.txt",
                snapshot_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
            ));
        }
    }
    assert!(
        drift.is_empty(),
        "API contract drift (server moved without a snapshot update?):\n  {}",
        drift.join("\n  ")
    );
}
