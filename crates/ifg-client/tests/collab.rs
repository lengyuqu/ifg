use ifg_client::{Client, Method, Secret, SessionCredentials};
use serde_json::json;
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};

#[tokio::test]
async fn session_credentials_are_separate_and_not_forwarded_on_redirect() {
    let server = MockServer::start().await;
    let other = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/alice/project/collaboration/tasks"))
        .and(header("authorization", "Bearer user-pat"))
        .and(header("x-ironforge-session", "session-id"))
        .and(header("x-ironforge-session-secret", "session-secret"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", other.uri()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&other)
        .await;
    let client = Client::builder(server.uri())
        .token(Secret::new("user-pat"))
        .build()
        .unwrap();
    let session = SessionCredentials::new("session-id", Secret::new("session-secret"));
    assert!(!format!("{session:?}").contains("session-secret"));
    let result = client
        .collaboration("alice", "project")
        .request(
            Method::POST,
            "tasks",
            Some(&json!({"title":"hello"})),
            Some(&session),
        )
        .await;
    assert_eq!(result.unwrap_err().status(), Some(302));
}

#[tokio::test]
async fn session_creation_never_includes_secret_in_decode_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/a/b/collaboration/sessions"))
        .respond_with(ResponseTemplate::new(201).set_body_string("{secret-leak"))
        .mount(&server)
        .await;
    let client = Client::builder(server.uri()).build().unwrap();
    let error = client
        .collaboration("a", "b")
        .start_session("a", "ifg")
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("secret-leak"));
}

#[tokio::test]
async fn collaboration_redacts_session_secrets_in_response_and_errors() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/a/b/collaboration/tasks"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"session-secret": ["echo session-secret"]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/a/b/collaboration/tasks"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(json!({"error":{"code":"BAD", "message":"session-secret"}})),
        )
        .mount(&server)
        .await;
    let client = Client::builder(server.uri()).build().unwrap();
    let session = SessionCredentials::new("id", Secret::new("session-secret"));
    let api = client.collaboration("a", "b");
    let value = api
        .request(Method::GET, "tasks", None, Some(&session))
        .await
        .unwrap();
    assert!(!value.to_string().contains("session-secret"));
    let error = api
        .request(Method::POST, "tasks", Some(&json!({})), Some(&session))
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("session-secret"));
}

#[test]
fn route_segments_do_not_allow_dot_traversal() {
    assert_eq!(ifg_client::collab_segment(".."), "%252E%252E");
    assert_eq!(ifg_client::collab_segment("a/b"), "a%2Fb");
    assert_eq!(ifg_client::collab_segment("a b+c"), "a%20b%2Bc");
}
