use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::{
    matchers::{path, query_param},
    Mock, MockServer, ResponseTemplate,
};

fn command(server: &MockServer, home: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("IRONFORGE_URL", server.uri())
        .env_remove("IRONFORGE_PAT")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["search", "demo"]);
    cmd
}

#[tokio::test]
async fn json_search_preserves_unknown_hit_and_envelope_fields_across_pages() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    for page in [1, 2] {
        let count = if page == 1 { 100 } else { 1 };
        let hits: Vec<Value> = (0..count)
            .map(|n| {
                json!({"id":(page-1)*100+n,
            "result_type":"repo","title":"demo","url":format!("/repo/{page}/{n}")})
            })
            .collect();
        Mock::given(path("/api/v1/search"))
            .and(query_param("page", page.to_string()))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"results":hits,"total":101,
                "page":page,"per_page":100,"future_envelope":{"ranking":"v2"}})),
            )
            .expect(1)
            .mount(&server)
            .await;
    }
    let result = command(&server, &home)
        .args(["--limit", "101", "--json"])
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
    assert_eq!(value["future_envelope"]["ranking"], "v2");
    assert_eq!(value["results"].as_array().unwrap().len(), 101);
    assert_eq!(value["results"][100]["url"], "/repo/2/0");
    assert!(value["results"][0].get("state").is_none());
}

#[tokio::test]
async fn template_search_can_read_unknown_resource_fields() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(path("/api/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results":[{
            "id":1,"result_type":"repo","title":"demo","url":"/alice/demo"}],"total":1,"page":1,"per_page":30})))
        .expect(1).mount(&server).await;
    let result = command(&server, &home)
        .args(["-q", "{{.url}}"])
        .assert()
        .success();
    assert_eq!(result.get_output().stdout, b"/alice/demo\n");
}
