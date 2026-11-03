//! DTO deserialization tests against real response samples recorded from a
//! live IronForge instance (design doc §6.2): `fixtures/responses/*.json`.

use std::fs;
use std::path::{Path, PathBuf};

use ifg_client::{Comment, Issue, MergeOutcome, Page, PullRequest, Repo};
use serde::de::DeserializeOwned;

fn sample(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/responses")
        .join(name)
}

fn load<T: DeserializeOwned>(name: &str) -> T {
    let text = fs::read_to_string(sample(name))
        .unwrap_or_else(|e| panic!("could not read fixture {name}: {e}"));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("could not decode fixture {name} into the DTO: {e}"))
}

#[test]
fn repo_view_decodes() {
    let repo: Repo = load("repo-view.json");
    assert_eq!(repo.name, "sample");
    assert_eq!(repo.default_branch, "main");
    assert!(!repo.is_private);
    assert_eq!(repo.stars_count, 0);
}

#[test]
fn repo_list_decodes_as_page() {
    let page: Page<Repo> = load("repo-list.json");
    assert!(!page.data.is_empty());
    assert_eq!(page.data[0].name, "sample");
    assert!(page.pagination.per_page > 0);
}

#[test]
fn issue_view_decodes_with_labels_string() {
    let issue: Issue = load("issue-view.json");
    assert_eq!(issue.number, 1);
    assert_eq!(issue.title, "Bug: crash on save");
    assert_eq!(issue.state, "open");
    assert_eq!(issue.author.as_deref(), Some("m2-1793702492"));
    assert_eq!(issue.label_list(), vec!["bug".to_string()]);
}

#[test]
fn issue_list_decodes_as_page() {
    let page: Page<Issue> = load("issue-list.json");
    assert!(!page.data.is_empty());
    assert_eq!(page.data[0].author.as_deref(), Some("m2-1793702492"));
}

#[test]
fn comments_decode_both_ways() {
    let comment: Comment = load("comment-create.json");
    assert_eq!(comment.body, "Investigating.");
    let list: Vec<Comment> = load("comment-list.json");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].body, "Investigating.");
}

#[test]
fn pull_request_decodes() {
    let pr: PullRequest = load("pr-create.json");
    assert_eq!(pr.number, 1);
    assert_eq!(pr.state, "open");
    assert!(!pr.is_draft);
    assert_eq!(pr.branch_label(), "feature -> main");
    assert!(pr.head_sha.is_some());
}

#[test]
fn pull_list_decodes_as_page() {
    let page: Page<PullRequest> = load("pr-list.json");
    assert!(!page.data.is_empty());
}

#[test]
fn merge_outcome_decodes() {
    let outcome: MergeOutcome = load("pr-merge.json");
    assert_eq!(outcome.strategy.as_deref(), Some("squash"));
    assert!(outcome.merge_commit_sha.is_some());
}
