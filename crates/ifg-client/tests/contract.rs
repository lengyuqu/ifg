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

// Check the operations as well as their schemas: an intact but unused schema
// must not hide a response or request body that changed to an untyped shape.
fn schema_ref<'a>(spec: &'a Value, pointer: &str) -> &'a Value {
    let schema = spec.pointer(pointer).expect("operation schema exists");
    let reference = schema["$ref"].as_str().expect("typed schema reference");
    spec.pointer(reference.strip_prefix('#').expect("local schema reference"))
        .expect("referenced schema exists")
}

fn check_fields(schema: &Value, fields: &[(&str, &str)], required: &[&str]) -> Vec<String> {
    let mut errors = Vec::new();
    for (name, expected_type) in fields {
        let ty = &schema["properties"][name]["type"];
        let matches = ty.as_str() == Some(expected_type)
            || ty
                .as_array()
                .is_some_and(|types| types.iter().any(|t| t.as_str() == Some(expected_type)));
        if !matches {
            errors.push(format!("{name}: expected type {expected_type}, got {ty}"));
        }
    }
    for name in required {
        if !schema["required"]
            .as_array()
            .is_some_and(|fields| fields.iter().any(|f| f.as_str() == Some(name)))
        {
            errors.push(format!("{name}: no longer required"));
        }
        if schema["properties"][name]["type"]
            .as_array()
            .is_some_and(|types| types.iter().any(|t| t.as_str() == Some("null")))
        {
            errors.push(format!("{name}: unexpectedly nullable"));
        }
    }
    errors
}

#[test]
fn critical_typed_operation_fields_match_client_contract() {
    type OperationContract<'a> = (&'a str, &'a [(&'a str, &'a str)], &'a [&'a str]);
    let spec: Value = serde_json::from_str(
        &fs::read_to_string(newest_snapshot(&fixtures_dir())).expect("readable snapshot"),
    )
    .expect("valid JSON snapshot");
    let contracts: &[OperationContract<'_>] = &[
        (
            "/paths/~1users~1me/get/responses/200/content/application~1json/schema",
            &[("id", "integer"), ("username", "string")],
            &["id", "username"],
        ),
        (
            "/paths/~1repos/post/requestBody/content/application~1json/schema",
            &[
                ("name", "string"),
                ("description", "string"),
                ("is_private", "boolean"),
                ("org", "string"),
                ("auto_init", "boolean"),
                ("default_branch", "string"),
                ("gitignores", "string"),
                ("license", "string"),
                ("readme", "string"),
                ("issue_labels", "string"),
            ],
            &["name"],
        ),
        (
            "/paths/~1repos/post/responses/201/content/application~1json/schema",
            &[
                ("id", "integer"),
                ("owner_id", "integer"),
                ("name", "string"),
                ("is_private", "boolean"),
                ("default_branch", "string"),
                ("stars_count", "integer"),
                ("forks_count", "integer"),
            ],
            &[
                "id",
                "owner_id",
                "name",
                "is_private",
                "default_branch",
                "stars_count",
                "forks_count",
            ],
        ),
    ];
    for (pointer, fields, required) in contracts {
        let errors = check_fields(schema_ref(&spec, pointer), fields, required);
        assert!(
            errors.is_empty(),
            "contract drift at {pointer}: {}",
            errors.join(", ")
        );
    }
    let request = schema_ref(
        &spec,
        "/paths/~1repos/post/requestBody/content/application~1json/schema",
    );
    let required = request["required"]
        .as_array()
        .expect("request required fields");
    assert!(
        required.iter().all(|field| field.as_str() == Some("name")),
        "repository creation gained a required field: {required:?}"
    );
}

#[test]
fn field_check_detects_removed_changed_and_optional_fields() {
    let schema = serde_json::json!({"properties": {"id": {"type": "string"}}, "required": []});
    let errors = check_fields(&schema, &[("id", "integer"), ("name", "string")], &["id"]);
    assert_eq!(errors.len(), 3, "{errors:?}");
}
