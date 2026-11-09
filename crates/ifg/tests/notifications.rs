//! `ifg notifications`: the endpoint returns `{data, pagination}`, so the CLI
//! must page through it instead of decoding a bare array.

use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::{
    matchers::{method, path, query_param},
    Mock, MockServer, ResponseTemplate,
};

fn command(server: &MockServer, home: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path());
    cmd
}

fn notification(id: i64) -> Value {
    json!({
        "id": id, "user_id": 1, "event_type": "issue", "title": format!("n{id}"),
        "body": null, "repo_id": 1, "is_read": false,
        "created_at": "2026-11-09T10:00:00Z"
    })
}

fn page(ids: std::ops::RangeInclusive<i64>, page: u64, has_next: bool) -> ResponseTemplate {
    let data: Vec<Value> = ids.map(notification).collect();
    ResponseTemplate::new(200).set_body_json(json!({
        "data": data,
        "pagination": {
            "page": page, "per_page": 100,
            "total": 150, "total_pages": 2,
            "has_next": has_next, "has_prev": page > 1
        }
    }))
}

/// The baseline server wraps notifications in `{data, pagination}`; decoding
/// it as `Vec<Notification>` made the plain list fail and the template mode
/// print nothing.
#[tokio::test]
async fn list_pages_through_the_envelope() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/notifications"))
        .and(query_param("page", "1"))
        .and(query_param("per_page", "100"))
        .respond_with(page(1..=100, 1, true))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/notifications"))
        .and(query_param("page", "2"))
        .and(query_param("per_page", "100"))
        .respond_with(page(101..=150, 2, false))
        .expect(2)
        .mount(&server)
        .await;

    let output = command(&server, &home)
        .args(["notifications", "list", "--limit", "150", "--json"])
        .assert()
        .success();
    let items: Value =
        serde_json::from_slice(&output.get_output().stdout).expect("valid JSON array");
    assert_eq!(items.as_array().expect("array").len(), 150);

    // Template mode reads the same items instead of treating the envelope as
    // an array and printing nothing.
    let output = command(&server, &home)
        .args(["notifications", "list", "--limit", "150", "-q", "{{.id}}"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).into_owned();
    assert_eq!(
        stdout.lines().count(),
        150,
        "template output: {stdout:.200}"
    );
}

/// The default limit asks for exactly 30 and stops after the first page.
#[tokio::test]
async fn default_limit_requests_one_page() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/notifications"))
        .and(query_param("page", "1"))
        .and(query_param("per_page", "30"))
        .respond_with(page(1..=30, 1, true))
        .expect(1)
        .mount(&server)
        .await;

    command(&server, &home)
        .args(["notifications", "list", "--json"])
        .assert()
        .success();
}

/// A server error must surface: the previous code turned a decode failure in
/// template mode into an empty, successful listing.
#[tokio::test]
async fn broken_envelope_is_an_error_not_an_empty_list() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/notifications"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&server)
        .await;

    command(&server, &home)
        .args(["notifications", "list", "-q", "{{.id}}"])
        .assert()
        .failure();
}
