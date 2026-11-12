use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

#[tokio::test]
async fn release_view_preserves_raw_release_and_asset_fields() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let release = json!({
        "id":7,"repo_id":1,"tag_name":"v1","title":"v1","target_commitish":"main",
        "is_draft":false,"is_prerelease":false,"author_id":1,
        "future_release_field":{"value":true},"assets":"replace me"
    });
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data":[release.clone()], "pagination":{"page":1,"per_page":100,
            "total":1,"total_pages":1,"has_next":false,"has_prev":false}
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/releases/7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(release))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/repos/alice/demo/releases/7/assets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
            "id":9,"release_id":7,"filename":"asset.bin","size":3,
            "content_type":"application/octet-stream","download_count":0,
            "future_asset_field":{"nested":true}
        }])))
        .expect(1)
        .mount(&server)
        .await;
    let output = Command::cargo_bin("ifg")
        .unwrap()
        .env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["--json", "-R", "alice/demo", "release", "view", "v1"])
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert_eq!(value["future_release_field"]["value"], true);
    assert_eq!(value["assets"][0]["future_asset_field"]["nested"], true);
    assert!(value["assets"][0].get("sha256").is_none());
}
