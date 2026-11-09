#![cfg(unix)]
use std::process::{Command, Stdio};
use tempfile::TempDir;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn failed_stdout_writes_fail_for_raw_structured_scalar_and_completion_output() {
    let server = MockServer::start().await;
    Mock::given(|_: &wiremock::Request| true)
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "username":"alice","unread_count":5,"version":"0.2.0"
        })))
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();
    let sink = home.path().join("read-only-output");
    std::fs::write(&sink, b"unchanged").unwrap();
    for args in [
        vec!["api", "users/me"],
        vec!["api", "users/me", "--json"],
        vec!["api", "users/me", "-q", "{{.username}}"],
        vec!["notifications", "count"],
        vec!["version"],
        vec!["completions", "bash"],
    ] {
        let args_owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let uri = server.uri();
        let config = home.path().to_owned();
        let file = std::fs::File::open(&sink).unwrap();
        let result = tokio::task::spawn_blocking(move || {
            Command::new(assert_cmd::cargo::cargo_bin("ifg"))
                .env("IRONFORGE_URL", uri)
                .env("IRONFORGE_PAT", "ifp_fixture")
                .env("XDG_CONFIG_HOME", config)
                .args(args_owned)
                .stdout(Stdio::from(file))
                .stderr(Stdio::piped())
                .output()
                .unwrap()
        })
        .await
        .unwrap();
        assert_eq!(
            result.status.code(),
            Some(1),
            "{args:?}: {:?}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stderr).contains("could not write stdout"));
    }
    assert_eq!(std::fs::read(sink).unwrap(), b"unchanged");
}

#[tokio::test]
async fn broken_pipe_is_still_an_expected_success() {
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    let server = MockServer::start().await;
    Mock::given(|_: &wiremock::Request| true)
        .respond_with(ResponseTemplate::new(200).set_body_string("response"))
        .mount(&server)
        .await;
    let home = TempDir::new().unwrap();
    let (reader, writer) = UnixStream::pair().unwrap();
    drop(reader);
    let uri = server.uri();
    let result = tokio::task::spawn_blocking(move || {
        Command::new(assert_cmd::cargo::cargo_bin("ifg"))
            .env("IRONFORGE_URL", uri)
            .env("XDG_CONFIG_HOME", home.path())
            .env_remove("IRONFORGE_PAT")
            .args(["api", "example"])
            .stdout(Stdio::from(OwnedFd::from(writer)))
            .stderr(Stdio::piped())
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(
        result.status.success(),
        "{:?}",
        String::from_utf8_lossy(&result.stderr)
    );
}
