//! Endpoints that write commits or move branches without a local checkout:
//! batch file commits, unified-diff commits, cherry-pick, revert and branch
//! deletion.
//!
//! Every commit-writing call is pinned to `expected_sha`: the commit the
//! branch must still be at. A branch that moved is a 409 `REF_ADVANCED`, and
//! the server never creates the branch. `dry_run` makes the server run every
//! check and report what would happen without writing. Only raw JSON twins
//! exist because the response shapes differ per operation and the CLI prints
//! them as the server sent them.

use serde_json::{json, Value};

use crate::error::Error;
use crate::http::Client;
use crate::repos::repo_path;

/// Which of the two commit-replay endpoints to call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickKind {
    /// `POST …/commits/{sha}/cherry-pick`: apply the commit's change.
    CherryPick,
    /// `POST …/commits/{sha}/revert`: apply the inverse of the commit's change.
    Revert,
}

impl PickKind {
    fn route(self) -> &'static str {
        match self {
            PickKind::CherryPick => "cherry-pick",
            PickKind::Revert => "revert",
        }
    }
}

/// Request of [`Commits::pick_raw`].
#[derive(Debug, Clone, Copy)]
pub struct PickRequest<'a> {
    pub kind: PickKind,
    /// The commit to replay (any revision the server resolves).
    pub sha: &'a str,
    pub branch: &'a str,
    pub expected_sha: &'a str,
    /// 1-based parent number for a merge commit (git's `-m`).
    pub mainline: Option<u32>,
    pub message: Option<&'a str>,
    pub dry_run: bool,
}

/// Accessor for the commit-writing endpoints of one repository.
pub struct Commits<'a> {
    client: &'a Client,
    owner: String,
    repo: String,
}

/// Accessor for branch endpoints of one repository.
pub struct Branches<'a> {
    client: &'a Client,
    owner: String,
    repo: String,
}

impl Client {
    /// Commit-writing endpoints of `owner/repo`.
    pub fn commits(&self, owner: &str, repo: &str) -> Commits<'_> {
        Commits {
            client: self,
            owner: owner.to_string(),
            repo: repo.to_string(),
        }
    }

    /// Branch endpoints of `owner/repo`.
    pub fn branches(&self, owner: &str, repo: &str) -> Branches<'_> {
        Branches {
            client: self,
            owner: owner.to_string(),
            repo: repo.to_string(),
        }
    }
}

/// Add `dry_run: true` to `body`; `false` is left out so a normal request
/// keeps exactly the body the server documents.
fn with_dry_run(mut body: Value, dry_run: bool) -> Value {
    if dry_run {
        body["dry_run"] = Value::Bool(true);
    }
    body
}

impl Commits<'_> {
    fn path(&self, tail: &str) -> String {
        format!("{}/commits{tail}", repo_path(&self.owner, &self.repo))
    }

    /// `POST …/commits`: many file edits as one commit. `files` is the
    /// server's array of tagged operations (`create`, `update`, `delete`,
    /// `move`) and is sent as given.
    pub async fn batch_raw(
        &self,
        branch: &str,
        expected_sha: &str,
        message: &str,
        files: &Value,
        dry_run: bool,
    ) -> Result<Value, Error> {
        let body = with_dry_run(
            json!({
                "branch": branch,
                "expected_sha": expected_sha,
                "message": message,
                "files": files,
            }),
            dry_run,
        );
        self.client
            .send(crate::Method::POST, &self.path(""), Some(&body))
            .await
    }

    /// `POST …/commits/apply-patch`: a unified diff as one commit. Without
    /// `message` the server generates one.
    pub async fn apply_patch_raw(
        &self,
        branch: &str,
        expected_sha: &str,
        patch: &str,
        message: Option<&str>,
        dry_run: bool,
    ) -> Result<Value, Error> {
        let mut body = json!({
            "branch": branch,
            "expected_sha": expected_sha,
            "patch": patch,
        });
        if let Some(message) = message {
            body["message"] = Value::String(message.to_string());
        }
        let body = with_dry_run(body, dry_run);
        self.client
            .send(crate::Method::POST, &self.path("/apply-patch"), Some(&body))
            .await
    }

    /// `POST …/commits/{sha}/cherry-pick` or `…/revert`.
    pub async fn pick_raw(&self, request: &PickRequest<'_>) -> Result<Value, Error> {
        let mut body = json!({
            "branch": request.branch,
            "expected_sha": request.expected_sha,
        });
        if let Some(mainline) = request.mainline {
            body["mainline"] = json!(mainline);
        }
        if let Some(message) = request.message {
            body["message"] = Value::String(message.to_string());
        }
        let body = with_dry_run(body, request.dry_run);
        let tail = format!(
            "/{}/{}",
            crate::path_segment(request.sha),
            request.kind.route()
        );
        self.client
            .send(crate::Method::POST, &self.path(&tail), Some(&body))
            .await
    }
}

impl Branches<'_> {
    /// `DELETE …/branches/{branch}`. The server refuses the default branch,
    /// a protected branch and a branch an open pull request uses. With
    /// `expected_sha` it deletes only while the branch is still at that
    /// commit (409 `REF_ADVANCED` otherwise).
    pub async fn delete_raw(
        &self,
        branch: &str,
        expected_sha: Option<&str>,
        dry_run: bool,
    ) -> Result<Value, Error> {
        // The branch name keeps its slashes (the route is a wildcard); every
        // segment between them is encoded like any other path segment.
        let name = branch
            .split('/')
            .map(crate::path_segment)
            .collect::<Vec<_>>()
            .join("/");
        let mut path = format!("{}/branches/{name}", repo_path(&self.owner, &self.repo));
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        if let Some(sha) = expected_sha {
            query.append_pair("expected_sha", sha);
        }
        if dry_run {
            query.append_pair("dry_run", "true");
        }
        let query = query.finish();
        if !query.is_empty() {
            path.push('?');
            path.push_str(&query);
        }
        self.client.send(crate::Method::DELETE, &path, None).await
    }
}
