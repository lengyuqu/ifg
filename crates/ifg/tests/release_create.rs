use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};

fn command(server: &MockServer, home: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["-R", "alice/demo", "release", "create", "v1"]);
    cmd
}

async fn mount_create(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/alice/demo/releases"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id":7,"repo_id":1,"tag_name":"v1","title":"v1","target_commitish":"main",
            "is_draft":false,"is_prerelease":false,"author_id":1,
            "future_release_field":"retained"
        })))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_asset(server: &MockServer, success: bool) {
    let response = if success {
        ResponseTemplate::new(201).set_body_json(json!({
            "id":9,"release_id":7,"filename":"asset.bin","size":3,
            "content_type":"application/octet-stream","download_count":0,
            "future_asset_field":"retained"
        }))
    } else {
        ResponseTemplate::new(400).set_body_json(json!({"error":{"message":"upload refused"}}))
    };
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/alice/demo/releases/7/assets"))
        .and(header("x-asset-filename", "asset.bin"))
        .respond_with(response)
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn json_create_with_an_asset_outputs_one_complete_document() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let file = home.path().join("asset.bin");
    std::fs::write(&file, [0, 1, 255]).unwrap();
    mount_create(&server).await;
    mount_asset(&server, true).await;
    let output = command(&server, &home)
        .arg("--json")
        .arg(&file)
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert_eq!(value["tag_name"], "v1");
    assert_eq!(value["future_release_field"], "retained");
    assert_eq!(value["assets"][0]["filename"], "asset.bin");
    assert_eq!(value["assets"][0]["future_asset_field"], "retained");
}

#[tokio::test]
async fn template_create_without_assets_renders_the_response() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    mount_create(&server).await;
    let output = command(&server, &home)
        .args(["-q", "{{.tag_name}}|{{.future_release_field}}"])
        .assert()
        .success();
    assert_eq!(output.get_output().stdout, b"v1|retained\n");
}

#[tokio::test]
async fn template_create_can_read_uploaded_assets() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let file = home.path().join("asset.bin");
    std::fs::write(&file, [0, 1, 255]).unwrap();
    mount_create(&server).await;
    mount_asset(&server, true).await;
    let output = command(&server, &home)
        .args(["-q", "{{.tag_name}}|{{range .assets}}{{.filename}};{{end}}"])
        .arg(&file)
        .assert()
        .success();
    assert_eq!(output.get_output().stdout, b"v1|asset.bin;\n");
}

#[tokio::test]
async fn failed_upload_returns_failure_without_claiming_a_complete_json_result() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let file = home.path().join("asset.bin");
    std::fs::write(&file, [0, 1, 255]).unwrap();
    mount_create(&server).await;
    mount_asset(&server, false).await;
    let output = command(&server, &home)
        .arg("--json")
        .arg(&file)
        .assert()
        .failure();
    assert!(output.get_output().stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.get_output().stderr);
    assert!(stderr.contains("Created release v1 (7)"), "{stderr}");
    assert!(stderr.contains("upload refused"), "{stderr}");
}
