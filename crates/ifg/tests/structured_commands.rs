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
        .env("IRONFORGE_PAT", "ifp_fixture")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["-R", "alice/demo"]);
    cmd
}

#[tokio::test]
async fn unsupported_formats_are_rejected_before_network_or_git_side_effects() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    for args in [
        vec!["repo", "clone", "alice/demo"],
        vec!["repo", "delete", "--yes"],
        vec!["issue", "view", "1", "--web"],
        vec!["pr", "view", "1", "--web"],
        vec!["pr", "checkout", "1"],
        vec!["pr", "diff", "1"],
        vec!["ci", "logs", "1"],
        vec!["release", "download", "v1"],
        vec!["release", "upload", "v1", "missing.bin"],
        vec!["release", "delete", "v1", "--yes"],
        vec!["label", "delete", "bug", "--yes"],
        vec!["milestone", "delete", "v1", "--yes"],
        vec!["auth", "token"],
        vec!["auth", "login", "--with-token"],
        vec!["version"],
        vec!["completions", "bash"],
    ] {
        for flags in [vec!["--json"], vec!["-q", "{{.id}}"]] {
            let result = command(&server, &home)
                .args(&args)
                .args(flags)
                .timeout(std::time::Duration::from_secs(3))
                .assert()
                .code(2);
            assert!(
                String::from_utf8_lossy(&result.get_output().stderr)
                    .contains("does not support --json or --template"),
                "{args:?}"
            );
        }
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn api_templates_handle_objects_arrays_and_paginated_results() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    for (route, body) in [
        ("profile", json!({"name":"alice"})),
        ("array", json!([{"name":"alice"},{"name":"bob"}])),
    ] {
        Mock::given(path(format!("/api/v1/{route}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
    }
    for (page, name) in [(1, "alice"), (2, "bob")] {
        Mock::given(path("/api/v1/list")).and(query_param("page",page.to_string()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"name":name}],
                "pagination":{"page":page,"per_page":100,"total":2,"total_pages":2,"has_next":page==1,"has_prev":page>1}})))
            .expect(1).mount(&server).await;
    }
    for (route, paginate, expected) in [
        ("profile", false, "alice\n"),
        ("array", false, "alice\nbob\n"),
        ("list", true, "alice\nbob\n"),
    ] {
        let mut cmd = command(&server, &home);
        cmd.args(["api", route, "-q", "{{.name}}"]);
        if paginate {
            cmd.arg("--paginate");
        }
        assert_eq!(
            cmd.assert().success().get_output().stdout,
            expected.as_bytes()
        );
    }
}

#[tokio::test]
async fn explicit_json_and_templates_require_json_but_raw_api_stays_unmodified() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(path("/api/v1/html"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>raw</html>"))
        .expect(3)
        .mount(&server)
        .await;
    for flags in [vec!["--json"], vec!["-q", "{{.name}}"]] {
        command(&server, &home)
            .args(["api", "html"])
            .args(flags)
            .assert()
            .failure();
    }
    assert_eq!(
        command(&server, &home)
            .args(["api", "html"])
            .assert()
            .success()
            .get_output()
            .stdout,
        b"<html>raw</html>"
    );
}

#[tokio::test]
async fn ci_filters_match_short_and_full_branch_names_and_keep_tags_explicit() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(path("/api/v1/repos/alice/demo/pipelines"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id":1,"ref_name":"refs/heads/feature"},{"id":2,"ref_name":"feature"},
            {"id":3,"ref_name":"refs/tags/feature"}])))
        .expect(3)
        .mount(&server)
        .await;
    for (selector, ids) in [
        ("feature", vec![1, 2]),
        ("refs/heads/feature", vec![1, 2]),
        ("refs/tags/feature", vec![3]),
    ] {
        let output = command(&server, &home)
            .args(["ci", "runs", "--branch", selector, "--json"])
            .assert()
            .success();
        let items: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
        assert_eq!(
            items
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["id"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            ids
        );
    }
}

#[tokio::test]
async fn ci_actions_and_pr_reviews_keep_machine_stdout_clean() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    for action in ["cancel", "retry"] {
        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/repos/alice/demo/pipelines/17/{action}"
            )))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"id":17,"action":action})),
            )
            .expect(2)
            .mount(&server)
            .await;
        let result = command(&server, &home)
            .args(["ci", action, "17", "--json"])
            .assert()
            .success();
        let value: Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
        assert_eq!(value["action"], action);
        let result = command(&server, &home)
            .args(["ci", action, "17", "-q", "{{.action}}"])
            .assert()
            .success();
        assert_eq!(result.get_output().stdout, format!("{action}\n").as_bytes());
    }
    Mock::given(method("POST"))
        .and(path("/api/v1/repos/alice/demo/pulls/1/reviews"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"action":"approve"})))
        .expect(1)
        .mount(&server)
        .await;
    let result = command(&server, &home)
        .args(["pr", "review", "1", "--approve", "-q", "{{.action}}"])
        .assert()
        .success();
    assert_eq!(result.get_output().stdout, b"approve\n");
}

#[tokio::test]
async fn notification_count_and_empty_auth_status_support_machine_formats() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(path("/api/v1/notifications/unread-count"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"unread_count":5})))
        .expect(1)
        .mount(&server)
        .await;
    let result = command(&server, &home)
        .args(["notifications", "count", "-q", "{{.unread_count}}"])
        .assert()
        .success();
    assert_eq!(result.get_output().stdout, b"5\n");
    let result = command(&server, &home)
        .args(["auth", "status", "--json"])
        .assert()
        .success();
    assert_eq!(
        serde_json::from_slice::<Value>(&result.get_output().stdout).unwrap(),
        json!([])
    );
}

#[tokio::test]
async fn html_mutation_response_never_prints_a_removal_confirmation() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    Mock::given(path("/api/v1/users/bob"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":2,"username":"bob"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/orgs/acme/members/2"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>fallback</html>"))
        .expect(1)
        .mount(&server)
        .await;
    let result = command(&server, &home)
        .args(["org", "remove-member", "acme", "bob"])
        .assert()
        .failure();
    assert!(result.get_output().stdout.is_empty());
}
