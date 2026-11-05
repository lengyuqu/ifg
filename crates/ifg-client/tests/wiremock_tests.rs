//! wiremock-driven tests for the M1 HTTP layer (design doc §7): auth header,
//! error-body parsing (§4.2), pagination (§4.3) and retry semantics (§4.4).

use std::time::Duration;

use ifg_client::{Client, Error, Method, Secret, PER_PAGE};
use serde_json::json;
use wiremock::matchers::{header, method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client_for(server: &MockServer) -> Client {
    Client::builder(server.uri())
        .token(Secret::new("ifp_testtoken"))
        .timeout(Duration::from_secs(5))
        .initial_backoff(Duration::from_millis(1))
        .build()
        .unwrap()
}

fn no_retry_client_for(server: &MockServer) -> Client {
    Client::builder(server.uri())
        .token(Secret::new("ifp_testtoken"))
        .max_retries(0)
        .build()
        .unwrap()
}

fn me_body() -> serde_json::Value {
    json!({"id": 1, "username": "alice", "email": null, "display_name": null,
           "avatar_url": null, "is_admin": false, "created_at": "2026-01-01T00:00:00Z"})
}

#[tokio::test]
async fn sends_bearer_token_and_parses_profile() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .and(header("authorization", "Bearer ifp_testtoken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(me_body()))
        .expect(1)
        .mount(&server)
        .await;

    let me = client_for(&server).users().me().await.unwrap();
    assert_eq!(me.username, "alice");
    assert_eq!(me.id, 1);
}

#[tokio::test]
async fn nested_error_body_is_parsed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/ghost"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "error": {"code": "NOT_FOUND", "message": "repository not found", "request_id": null}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let err: Error = no_retry_client_for(&server)
        .raw(Method::GET, "repos/ghost", None)
        .await
        .unwrap_err();
    assert_eq!(err.status(), Some(404));
    assert_eq!(err.code(), Some("NOT_FOUND"));
    assert!(err.is_not_found());
    assert!(err.to_string().contains("repository not found"));
}

#[tokio::test]
async fn non_json_error_body_falls_back_to_snippet() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/api/v1/.*$"))
        .respond_with(
            ResponseTemplate::new(502)
                .set_body_string("<html>Bad Gateway</html>")
                .insert_header("content-type", "text/html"),
        )
        .mount(&server)
        .await;

    let err: Error = no_retry_client_for(&server)
        .raw(Method::GET, "repos", None)
        .await
        .unwrap_err();
    assert_eq!(err.status(), Some(502));
    assert_eq!(err.code(), None);
    assert!(err.to_string().contains("Bad Gateway"));
}

#[tokio::test]
async fn get_retries_on_503_and_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(2)
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .and(header("authorization", "Bearer ifp_testtoken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(me_body()))
        .expect(1)
        .mount(&server)
        .await;

    let me = client_for(&server).users().me().await.unwrap();
    assert_eq!(me.username, "alice");
    assert_eq!(requests_to(&server, "/api/v1/users/me").await, 3);
}

#[tokio::test]
async fn retry_after_header_is_honored() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .respond_with(ResponseTemplate::new(503).insert_header("Retry-After", "0"))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(me_body()))
        .expect(1)
        .mount(&server)
        .await;

    let me = client_for(&server).users().me().await.unwrap();
    assert_eq!(me.username, "alice");
    assert_eq!(requests_to(&server, "/api/v1/users/me").await, 2);
}

/// Number of recorded requests whose path matches exactly.
async fn requests_to(server: &MockServer, target: &str) -> usize {
    server
        .received_requests()
        .await
        .expect("mock server request log")
        .iter()
        .filter(|request| request.url.path() == target)
        .count()
}

#[tokio::test]
async fn post_is_never_retried() {
    let server = MockServer::start().await;
    let mut failing = Mock::given(method("POST"))
        .and(path("/api/v1/repos"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1);
    failing = failing.up_to_n_times(100);
    failing.mount(&server).await;

    let err: Error = client_for(&server)
        .raw(Method::POST, "repos", Some(&json!({"name": "demo"})))
        .await
        .unwrap_err();
    assert_eq!(err.status(), Some(503));
}

#[tokio::test]
async fn exhausted_retries_surface_the_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({
            "error": {"code": "INTERNAL_ERROR", "message": "maintenance"}
        })))
        .expect(4) // 1 attempt + 3 retries
        .mount(&server)
        .await;

    let err: Error = client_for(&server).users().me().await.unwrap_err();
    assert_eq!(err.status(), Some(503));
    assert_eq!(err.code(), Some("INTERNAL_ERROR"));
}

fn page(items: serde_json::Value, page: u64, has_next: bool) -> ResponseTemplate {
    let total_pages = if has_next { page + 1 } else { page };
    ResponseTemplate::new(200).set_body_json(json!({
        "data": items,
        "pagination": {
            "page": page,
            "per_page": PER_PAGE,
            "total": 3,
            "total_pages": total_pages,
            "has_next": has_next,
            "has_prev": page > 1,
        }
    }))
}

#[tokio::test]
async fn paginator_follows_has_next_and_uses_max_per_page() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos"))
        .and(query_param("page", "1"))
        .and(query_param("per_page", "100"))
        .respond_with(page(json!([{"id": 1}, {"id": 2}]), 1, true))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos"))
        .and(query_param("page", "2"))
        .respond_with(page(json!([{"id": 3}]), 2, false))
        .expect(1)
        .mount(&server)
        .await;

    let items: Vec<serde_json::Value> = client_for(&server)
        .paginate("repos")
        .collect()
        .await
        .unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[2]["id"], json!(3));
}

#[tokio::test]
async fn paginator_stops_on_empty_data_with_has_next() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos"))
        .respond_with(page(json!([]), 1, true))
        .expect(1) // must not fetch a second page
        .mount(&server)
        .await;

    let items: Vec<serde_json::Value> = client_for(&server)
        .paginate::<serde_json::Value>("repos")
        .collect()
        .await
        .unwrap();
    assert!(items.is_empty());
}

#[tokio::test]
async fn paginator_limit_stops_before_the_next_page() {
    let server = MockServer::start().await;
    let many: Vec<_> = (0..100).map(|i| json!({"id": i})).collect();
    Mock::given(method("GET"))
        .and(path("/api/v1/repos"))
        .and(query_param("page", "1"))
        .respond_with(page(json!(many), 1, true))
        .expect(1) // limit reached after page 1: no second request
        .mount(&server)
        .await;

    let items: Vec<serde_json::Value> = client_for(&server)
        .paginate::<serde_json::Value>("repos")
        .limit(5)
        .collect()
        .await
        .unwrap();
    assert_eq!(items.len(), 5);
}

#[tokio::test]
async fn paginator_limit_narrows_the_request_per_page() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos"))
        .and(query_param("page", "1"))
        .and(query_param("per_page", "5"))
        .respond_with(page(json!([{"id": 1}, {"id": 2}, {"id": 3}]), 1, false))
        .expect(1) // one request, asking for 5 rather than PER_PAGE
        .mount(&server)
        .await;

    let items: Vec<serde_json::Value> = client_for(&server)
        .paginate::<serde_json::Value>("repos")
        .limit(5)
        .collect()
        .await
        .unwrap();
    assert_eq!(items.len(), 3);
}

#[tokio::test]
async fn paginator_limit_only_narrows_the_first_page() {
    let server = MockServer::start().await;
    // limit(3) with a server that keeps saying has_next: page 1 asks for 3,
    // page 2 goes back to the cap. Otherwise `limit(150)` would degrade into
    // 50 requests of 3 instead of 2 requests of 100.
    Mock::given(method("GET"))
        .and(path("/api/v1/repos"))
        .and(query_param("page", "1"))
        .and(query_param("per_page", "3"))
        .respond_with(page(json!([{"id": 0}, {"id": 1}]), 1, true))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos"))
        .and(query_param("page", "2"))
        .and(query_param("per_page", "100"))
        .respond_with(page(json!([{"id": 2}]), 2, false))
        .expect(1)
        .mount(&server)
        .await;

    let items: Vec<serde_json::Value> = client_for(&server)
        .paginate::<serde_json::Value>("repos")
        .limit(3)
        .collect()
        .await
        .unwrap();
    assert_eq!(items.len(), 3);
}

#[tokio::test]
async fn paginator_limit_above_the_cap_always_asks_for_the_cap() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos"))
        .and(query_param("page", "1"))
        .and(query_param("per_page", "100"))
        .respond_with(page(json!([{"id": 0}]), 1, false))
        .expect(1)
        .mount(&server)
        .await;

    // min(150, PER_PAGE) == PER_PAGE: the narrowed size must never exceed it.
    let items: Vec<serde_json::Value> = client_for(&server)
        .paginate::<serde_json::Value>("repos")
        .limit(150)
        .collect()
        .await
        .unwrap();
    assert_eq!(items.len(), 1);
}

#[tokio::test]
async fn paginator_zero_limit_never_sends_per_page_zero() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos"))
        .respond_with(page(json!([{"id": 1}]), 1, false))
        .mount(&server)
        .await;

    // limit(0) asks for nothing at all, and certainly not `per_page=0`.
    let items: Vec<serde_json::Value> = client_for(&server)
        .paginate::<serde_json::Value>("repos")
        .limit(0)
        .collect()
        .await
        .unwrap();
    assert!(items.is_empty());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn paginator_reuses_the_body_for_post_pages() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/search"))
        .and(query_param("page", "1"))
        .respond_with(page(json!([{"id": 1}]), 1, false))
        .expect(1)
        .mount(&server)
        .await;

    let items: Vec<serde_json::Value> = client_for(&server)
        .paginate_with_body(Method::POST, "search", json!({"q": "rust"}))
        .collect()
        .await
        .unwrap();
    assert_eq!(items.len(), 1);
}

#[tokio::test]
async fn raw_path_tolerates_leading_slash_and_api_v1_prefix() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(me_body()))
        .expect(3)
        .mount(&server)
        .await;

    let client = client_for(&server);
    for path in ["/users/me", "/api/v1/users/me", "users/me"] {
        let bytes = client.raw(Method::GET, path, None).await.unwrap();
        assert!(!bytes.is_empty());
    }
}

#[tokio::test]
async fn release_upload_streams_the_file_with_filename_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/alice/demo/releases/1/assets"))
        .and(header("x-asset-filename", "artifact.txt"))
        .and(header("content-type", "text/plain"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": 5, "release_id": 1, "filename": "artifact.txt", "size": 12,
            "content_type": "text/plain", "download_count": 0, "uploader_id": 1,
            "sha256": null, "created_at": "2026-11-03T10:00:00Z"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("artifact.txt");
    std::fs::write(&file, "artifact ok\n").unwrap();
    let client = Client::builder(server.uri())
        .token(Secret::new("ifp_testtoken"))
        .build()
        .unwrap();
    let asset = client
        .releases("alice", "demo")
        .upload_file(1, &file, "text/plain")
        .await
        .unwrap();
    assert_eq!(asset.filename, "artifact.txt");
    assert_eq!(asset.size, 12);
    assert_eq!(asset.sha256, None);
}

#[tokio::test]
async fn release_download_returns_the_streaming_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/releases/assets/5/download"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("binary-ish payload")
                .insert_header("x-checksum-sha256", "deadbeef"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = Client::builder(server.uri()).build().unwrap();
    let response = client.releases("alice", "demo").download(5).await.unwrap();
    assert_eq!(
        response.headers().get("x-checksum-sha256").unwrap(),
        "deadbeef"
    );
    let body = response.bytes().await.unwrap();
    assert_eq!(body.as_ref(), b"binary-ish payload");
}
