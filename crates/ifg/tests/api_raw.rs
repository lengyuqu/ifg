use assert_cmd::Command;
use tempfile::TempDir;
use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn raw_api_preserves_empty_binary_and_text_responses_exactly() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    for (index, payload) in [
        b"".as_slice(),
        &[0, 1, 255],
        b"text without newline",
        b"text with newline\n",
    ]
    .into_iter()
    .enumerate()
    {
        let endpoint = format!("raw/{index}");
        Mock::given(path(format!("/api/v1/{endpoint}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(payload.to_vec()))
            .expect(1)
            .mount(&server)
            .await;
        let output = Command::cargo_bin("ifg")
            .unwrap()
            .env("IRONFORGE_URL", server.uri())
            .env("IRONFORGE_PAT", "ifp_test")
            .env("XDG_CONFIG_HOME", home.path())
            .args(["api", &endpoint])
            .assert()
            .success();
        assert_eq!(output.get_output().stdout, payload, "case {index}");
    }
}

#[tokio::test]
async fn json_api_accepts_empty_and_rejects_non_json() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    for (endpoint, payload, valid) in [("empty", "", true), ("html", "<html>", false)] {
        Mock::given(path(format!("/api/v1/{endpoint}")))
            .respond_with(ResponseTemplate::new(200).set_body_string(payload))
            .expect(1)
            .mount(&server)
            .await;
        let mut command = Command::cargo_bin("ifg").unwrap();
        command
            .env("IRONFORGE_URL", server.uri())
            .env("IRONFORGE_PAT", "ifp_test")
            .env("XDG_CONFIG_HOME", home.path())
            .args(["--json", "api", endpoint]);
        if valid {
            let output = command.assert().success();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&output.get_output().stdout).unwrap(),
                serde_json::Value::Null
            );
        } else {
            let output = command.assert().failure();
            assert!(output.get_output().stdout.is_empty());
        }
    }
}
