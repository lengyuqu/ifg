use ifg_client::{Client, Error};
use serde_json::json;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

#[tokio::test]
async fn html_mutation_responses_are_decode_errors_across_resources() {
    let server = MockServer::start().await;
    Mock::given(|_: &wiremock::Request| true)
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>proxy fallback</html>"))
        .mount(&server)
        .await;
    let client = Client::builder(server.uri()).build().unwrap();
    assert!(matches!(
        client.orgs().remove_member("acme", 2).await,
        Err(Error::Decode { .. })
    ));
    assert!(matches!(
        client.repos().delete("alice", "demo").await,
        Err(Error::Decode { .. })
    ));
    assert!(matches!(
        client.repos().remove_collaborator("alice", "demo", 2).await,
        Err(Error::Decode { .. })
    ));
    assert!(matches!(
        client
            .pulls("alice", "demo")
            .remove_reviewer(1, "bob")
            .await,
        Err(Error::Decode { .. })
    ));
    assert!(matches!(
        client.wiki("alice", "demo").delete("Page").await,
        Err(Error::Decode { .. })
    ));
    assert!(matches!(
        client.labels("alice", "demo").delete(1).await,
        Err(Error::Decode { .. })
    ));
    assert!(matches!(
        client.milestones("alice", "demo").delete(1).await,
        Err(Error::Decode { .. })
    ));
    assert!(matches!(
        client.releases("alice", "demo").delete(1).await,
        Err(Error::Decode { .. })
    ));
    assert!(matches!(
        client.notifications().mark_read(1).await,
        Err(Error::Decode { .. })
    ));
    assert!(matches!(
        client.notifications().mark_all_read().await,
        Err(Error::Decode { .. })
    ));
    assert!(matches!(
        client.notifications().delete(1).await,
        Err(Error::Decode { .. })
    ));
}

#[tokio::test]
async fn empty_and_valid_json_mutation_responses_remain_supported() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/notifications/1/read"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/notifications/2/read"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"is_read":true})))
        .expect(1)
        .mount(&server)
        .await;
    let client = Client::builder(server.uri()).build().unwrap();
    assert_eq!(
        client.notifications().mark_read(1).await.unwrap(),
        json!(null)
    );
    assert_eq!(
        client.notifications().mark_read(2).await.unwrap(),
        json!({"is_read":true})
    );
}
