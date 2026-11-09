use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::{
    matchers::{path, query_param},
    Mock, MockServer, ResponseTemplate,
};

fn checkout() -> TempDir {
    let dir = TempDir::new().unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-b", "feature"])
        .current_dir(dir.path())
        .output()
        .unwrap()
        .status
        .success());
    dir
}

fn command(server: &MockServer, home: &TempDir, work: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("IRONFORGE_URL", server.uri())
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .current_dir(work.path())
        .args(["-R", "alice/demo", "status"]);
    cmd
}

fn page(data: Vec<Value>, number: u64, has_next: bool) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "data": data,
        "pagination": {"page":number,"per_page":100,"total":200,"total_pages":2,
                       "has_next":has_next,"has_prev":number > 1}
    }))
}

fn pipeline(id: i64, reference: &str, trigger: &str) -> Value {
    json!({"id":id,"repo_id":1,"ref_name":reference,"status":"success",
           "trigger_type":trigger,"future_pipeline_field":"retained"})
}

async fn mount(server: &MockServer, with_pr: bool, runs: Vec<Value>) {
    let pulls = if with_pr {
        vec![
            json!({"id":1,"repo_id":1,"number":12,"title":"Feature","state":"open",
                    "author_id":1,"head_branch":"feature","base_branch":"main",
                    "future_pr_field":"retained"}),
        ]
    } else {
        vec![]
    };
    Mock::given(path("/api/v1/repos/alice/demo/pulls"))
        .respond_with(page(pulls, 1, false))
        .expect(1)
        .mount(server)
        .await;
    Mock::given(path("/api/v1/repos/alice/demo/pipelines"))
        .and(query_param("page", "1"))
        .respond_with(page(runs, 1, false))
        .expect(1)
        .mount(server)
        .await;
}

fn mixed_refs() -> Vec<Value> {
    vec![
        pipeline(17, "refs/heads/feature", "full_branch"),
        pipeline(18, "feature", "short_branch"),
        pipeline(19, "refs/tags/feature", "same_named_tag"),
        pipeline(20, "refs/heads/other", "other_branch"),
    ]
}

#[tokio::test]
async fn human_status_matches_branch_refs_without_matching_tags() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = checkout();
    mount(&server, true, mixed_refs()).await;
    let output = command(&server, &home, &work).assert().success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    assert!(stdout.contains("full_branch"), "{stdout}");
    assert!(stdout.contains("short_branch"), "{stdout}");
    assert!(!stdout.contains("same_named_tag"), "{stdout}");
    assert!(!stdout.contains("other_branch"), "{stdout}");
}

#[tokio::test]
async fn json_status_preserves_resource_fields_and_reports_scan_count() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = checkout();
    mount(&server, true, mixed_refs()).await;
    let output = command(&server, &home, &work)
        .arg("--json")
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert_eq!(value["repository"], "alice/demo");
    assert_eq!(value["branch"], "feature");
    assert_eq!(value["pull"]["number"], 12);
    assert_eq!(value["pull"]["future_pr_field"], "retained");
    assert_eq!(value["runs"][0]["future_pipeline_field"], "retained");
    assert_eq!(value["runs"][0]["ref_name"], "refs/heads/feature");
    assert_eq!(value["runs"].as_array().unwrap().len(), 2);
    assert_eq!(value["scanned_runs"], 4);
}

#[tokio::test]
async fn template_status_can_read_the_pull_and_run_list() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = checkout();
    mount(&server, true, mixed_refs()).await;
    let output = command(&server, &home, &work)
        .args([
            "-q",
            "{{.repository}}|{{.branch}}|{{.pull.number}}|{{range .runs}}{{.id}},{{end}}",
        ])
        .assert()
        .success();
    assert_eq!(
        output.get_output().stdout,
        b"alice/demo|feature|12|17,18,\n"
    );
}

#[tokio::test]
async fn empty_status_uses_null_pull_and_empty_runs() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = checkout();
    mount(&server, false, vec![]).await;
    let output = command(&server, &home, &work)
        .arg("--json")
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert!(value["pull"].is_null());
    assert_eq!(value["runs"], json!([]));
    assert_eq!(value["scanned_runs"], 0);
}

#[tokio::test]
async fn status_continues_to_later_pages_and_limits_matches_to_five() {
    let server = MockServer::start().await;
    let home = TempDir::new().unwrap();
    let work = checkout();
    Mock::given(path("/api/v1/repos/alice/demo/pulls"))
        .respond_with(page(vec![], 1, false))
        .mount(&server)
        .await;
    Mock::given(path("/api/v1/repos/alice/demo/pipelines"))
        .and(query_param("page", "1"))
        .respond_with(page(
            (1..=100).map(|id| pipeline(id, "other", "push")).collect(),
            1,
            true,
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/api/v1/repos/alice/demo/pipelines"))
        .and(query_param("page", "2"))
        .respond_with(page(
            (101..=106)
                .map(|id| pipeline(id, "refs/heads/feature", "manual"))
                .collect(),
            2,
            false,
        ))
        .expect(1)
        .mount(&server)
        .await;
    let output = command(&server, &home, &work)
        .arg("--json")
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&output.get_output().stdout).unwrap();
    assert_eq!(value["runs"].as_array().unwrap().len(), 5);
    assert_eq!(value["runs"][0]["id"], 101);
    assert_eq!(value["runs"][4]["id"], 105);
    assert_eq!(value["scanned_runs"], 106);
}
