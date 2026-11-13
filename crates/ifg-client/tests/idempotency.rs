//! `Idempotency-Key` on write requests: which requests carry it, and how a
//! keyed write is retried compared with an unkeyed one.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ifg_client::{Client, CreateIssueOptions, IdempotencyKey, Method, Secret};
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "issue-create-42";
const ISSUE_PATH: &str = "/api/v1/repos/alice/demo/issues";

fn keyed(server: &MockServer) -> Client {
    Client::builder(server.uri())
        .token(Secret::new("ifp_testtoken"))
        .timeout(Duration::from_secs(5))
        .initial_backoff(Duration::from_millis(1))
        .idempotency_key(IdempotencyKey::parse(KEY).unwrap())
        .build()
        .unwrap()
}

fn unkeyed(server: &MockServer) -> Client {
    Client::builder(server.uri())
        .token(Secret::new("ifp_testtoken"))
        .timeout(Duration::from_secs(5))
        .initial_backoff(Duration::from_millis(1))
        .build()
        .unwrap()
}

fn issue_body() -> serde_json::Value {
    json!({"title": "flaky deploy"})
}

/// The `Idempotency-Key` values of every request the server saw, in order.
/// `None` means the header was absent.
async fn keys_seen(server: &MockServer) -> Vec<Option<String>> {
    server
        .received_requests()
        .await
        .expect("request recording enabled")
        .iter()
        .map(|request| {
            request
                .headers
                .get("idempotency-key")
                .map(|value| value.to_str().unwrap().to_string())
        })
        .collect()
}

#[tokio::test]
async fn key_is_sent_on_writes_and_never_on_reads() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 1, "username": "alice", "email": null, "display_name": null,
            "avatar_url": null, "is_admin": false, "created_at": "2026-01-01T00:00:00Z"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(ISSUE_PATH))
        .and(header("Idempotency-Key", KEY))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"number": 1})))
        .expect(1)
        .mount(&server)
        .await;

    let client = keyed(&server);
    client.users().me().await.unwrap();
    client
        .raw(Method::POST, "repos/alice/demo/issues", Some(&issue_body()))
        .await
        .unwrap();

    assert_eq!(
        keys_seen(&server).await,
        vec![None, Some(KEY.to_string())],
        "GET must not carry the key; the POST must"
    );
}

#[tokio::test]
async fn every_write_method_carries_the_key() {
    let server = MockServer::start().await;
    Mock::given(header("Idempotency-Key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(4)
        .mount(&server)
        .await;

    let client = keyed(&server);
    for verb in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        client
            .raw(verb.clone(), "repos/alice/demo/labels/1", Some(&json!({})))
            .await
            .unwrap_or_else(|e| panic!("{verb}: {e}"));
    }
}

#[tokio::test]
async fn keyed_create_is_retried_after_503_with_the_same_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(ISSUE_PATH))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(ISSUE_PATH))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"number": 7})))
        .mount(&server)
        .await;

    let options = CreateIssueOptions {
        title: "flaky deploy".into(),
        body: None,
        labels: None,
        assignees: None,
    };
    let body = serde_json::to_value(&options).unwrap();
    let created = keyed(&server)
        .raw(Method::POST, "repos/alice/demo/issues", Some(&body))
        .await
        .expect("the retry succeeds");
    assert!(String::from_utf8_lossy(&created).contains("\"number\":7"));
    assert_eq!(
        keys_seen(&server).await,
        vec![Some(KEY.to_string()), Some(KEY.to_string())]
    );
}

#[tokio::test]
async fn keyed_write_is_retried_after_429_and_5xx_but_not_on_400() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(ISSUE_PATH))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(ISSUE_PATH))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(ISSUE_PATH))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"number": 8})))
        .mount(&server)
        .await;

    keyed(&server)
        .raw(Method::POST, "repos/alice/demo/issues", Some(&issue_body()))
        .await
        .expect("429 and 500 are both retried with the key");
    assert_eq!(keys_seen(&server).await.len(), 3);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(ISSUE_PATH))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {"code": "IDEMPOTENCY_NOT_SUPPORTED", "message": "no",
                      "reason": "IDEMPOTENCY_NOT_SUPPORTED"}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let err = keyed(&server)
        .raw(Method::POST, "repos/alice/demo/issues", Some(&issue_body()))
        .await
        .unwrap_err();
    assert_eq!(err.status(), Some(400));
    assert_eq!(err.reason(), Some("IDEMPOTENCY_NOT_SUPPORTED"));
}

#[tokio::test]
async fn unkeyed_post_is_not_retried_after_503() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(ISSUE_PATH))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;

    let err = unkeyed(&server)
        .raw(Method::POST, "repos/alice/demo/issues", Some(&issue_body()))
        .await
        .unwrap_err();
    assert_eq!(err.status(), Some(503));
    assert_eq!(keys_seen(&server).await, vec![None]);
}

#[tokio::test]
async fn streamed_asset_upload_never_carries_the_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/alice/demo/releases/7/assets"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 3})))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::TempDir::new().unwrap();
    let file = dir.path().join("app.tar.gz");
    std::fs::write(&file, b"payload").unwrap();

    keyed(&server)
        .releases("alice", "demo")
        .upload_file_raw(7, &file, "application/gzip")
        .await
        .unwrap();
    assert_eq!(keys_seen(&server).await, vec![None]);
}

/// A server that drops the first connection right after reading the request
/// (no response at all), then answers the next one with `201`. Records the
/// `Idempotency-Key` of each request it reads.
fn dropping_then_created_server() -> (String, Arc<Mutex<Vec<Option<String>>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);
    std::thread::spawn(move || {
        for attempt in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let key = read_request_key(&mut stream);
            recorder.lock().unwrap().push(key);
            if attempt == 0 {
                // Dropped: the client sees the connection end mid-exchange.
                drop(stream);
                continue;
            }
            let body = r#"{"number":9}"#;
            let response = format!(
                "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (format!("http://{addr}"), seen)
}

/// Read one request (headers and `Content-Length` body) and return its
/// `Idempotency-Key` header, if any.
fn read_request_key(stream: &mut std::net::TcpStream) -> Option<String> {
    let mut data = Vec::new();
    let mut buf = [0u8; 1024];
    let head_end = loop {
        let n = stream.read(&mut buf).unwrap();
        assert!(n > 0, "client closed before sending a request");
        data.extend_from_slice(&buf[..n]);
        if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let head = String::from_utf8_lossy(&data[..head_end]).to_string();
    let content_length: usize = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);
    while data.len() < head_end + content_length {
        let n = stream.read(&mut buf).unwrap();
        assert!(n > 0, "client closed mid-body");
        data.extend_from_slice(&buf[..n]);
    }
    head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("idempotency-key")
            .then(|| value.trim().to_string())
    })
}

#[tokio::test]
async fn keyed_post_is_retried_after_a_dropped_connection_with_the_same_key() {
    let (base, seen) = dropping_then_created_server();
    let client = Client::builder(base)
        .timeout(Duration::from_secs(5))
        .initial_backoff(Duration::from_millis(1))
        .idempotency_key(IdempotencyKey::parse("drop-test").unwrap())
        .build()
        .unwrap();

    let value = client
        .raw(Method::POST, "repos/alice/demo/issues", Some(&issue_body()))
        .await
        .expect("the second attempt succeeds");
    assert!(String::from_utf8_lossy(&value).contains("\"number\":9"));
    assert_eq!(
        *seen.lock().unwrap(),
        vec![Some("drop-test".to_string()), Some("drop-test".to_string())]
    );
}

#[test]
fn key_validation_matches_the_server_rule() {
    assert!(IdempotencyKey::parse("ok.key_1:2-3").is_ok());
    assert!(IdempotencyKey::parse("").is_err());
    assert!(IdempotencyKey::parse(&"x".repeat(256)).is_err());
    assert!(IdempotencyKey::parse("a b").is_err());
}
