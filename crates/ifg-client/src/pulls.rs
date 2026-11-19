//! `pulls` resource group. `pr checkout` is deliberately absent (M3; needs
//! `refs/pull/<n>/head` or a fetch workaround, design doc §7.2).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Error;
use crate::http::Client;
use crate::pagination::Paginator;
use crate::repos::{repo_path, to_body};

/// Pull request view. `state` is `open` / `merged` / `closed`.
#[derive(Debug, Clone, Deserialize)]
pub struct PullRequest {
    pub id: i64,
    pub repo_id: i64,
    pub number: i64,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    pub state: String,
    #[serde(default)]
    pub is_draft: bool,
    pub author_id: i64,
    pub head_branch: String,
    pub base_branch: String,
    #[serde(default)]
    pub head_sha: Option<String>,
    #[serde(default)]
    pub merge_commit_sha: Option<String>,
    /// Fork PRs point at another repo; `None` means same-repo.
    #[serde(default)]
    pub head_repo_id: Option<i64>,
    /// Fork head repository namespace/name (enriched by the server on PR
    /// detail; absent for same-repo PRs and older servers).
    #[serde(default)]
    pub head_repo_owner: Option<String>,
    #[serde(default)]
    pub head_repo_name: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub merged_at: Option<String>,
    #[serde(default)]
    pub closed_at: Option<String>,
}

impl PullRequest {
    /// `head -> base` display form.
    pub fn branch_label(&self) -> String {
        format!("{} -> {}", self.head_branch, self.base_branch)
    }
}

/// `POST …/pulls` request body. `head` supports `owner:branch` for fork PRs.
#[derive(Debug, Default, Serialize)]
pub struct CreatePrOptions {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub head: String,
    pub base: String,
    #[serde(default)]
    pub draft: bool,
}

/// Partial PR update; omitted fields remain unchanged.
#[derive(Debug, Default, Serialize)]
pub struct UpdatePrOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft: Option<bool>,
}

/// Merge strategy (`merge` / `squash` / `rebase`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStrategy {
    Merge,
    Squash,
    Rebase,
}

impl MergeStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            MergeStrategy::Merge => "merge",
            MergeStrategy::Squash => "squash",
            MergeStrategy::Rebase => "rebase",
        }
    }
}

/// `POST …/pulls/{number}/merge` response.
#[derive(Debug, Clone, Deserialize)]
pub struct MergeOutcome {
    #[serde(default)]
    pub merge_commit_sha: Option<String>,
    #[serde(default)]
    pub strategy: Option<String>,
}

/// Review action for `POST …/pulls/{number}/reviews`.
#[derive(Debug, Clone, Copy)]
pub enum ReviewAction {
    Comment,
    Approve,
    RequestChanges,
}

impl ReviewAction {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewAction::Comment => "comment",
            ReviewAction::Approve => "approve",
            ReviewAction::RequestChanges => "request_changes",
        }
    }
}

/// Accessor for pull-request endpoints of one repository.
/// An inline review comment on a pull request.
#[derive(Debug, Clone, Deserialize)]
pub struct ReviewComment {
    pub id: i64,
    pub pr_id: i64,
    pub review_id: i64,
    #[serde(default)]
    pub author_id: Option<i64>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub line: Option<i64>,
    #[serde(default)]
    pub side: Option<String>,
    pub body: String,
    #[serde(default)]
    pub created_at: Option<String>,
}

pub struct Pulls<'a> {
    client: &'a Client,
    owner: String,
    repo: String,
}

impl Client {
    /// Pull-request endpoints of `owner/repo`.
    pub fn pulls(&self, owner: &str, repo: &str) -> Pulls<'_> {
        Pulls {
            client: self,
            owner: owner.to_string(),
            repo: repo.to_string(),
        }
    }
}

impl Pulls<'_> {
    fn pulls_path(&self) -> String {
        format!("{}/pulls", repo_path(&self.owner, &self.repo))
    }

    fn pull_path(&self, number: i64) -> String {
        format!("{}/pulls/{number}", repo_path(&self.owner, &self.repo))
    }

    pub async fn update_raw(&self, number: i64, options: &UpdatePrOptions) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::PATCH,
                &self.pull_path(number),
                Some(&to_body(options)?),
            )
            .await
    }

    pub async fn reviewers(&self, number: i64) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::GET,
                &format!("{}/reviewers", self.pull_path(number)),
                None,
            )
            .await
    }

    pub async fn reviews_raw(&self, number: i64) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::GET,
                &format!("{}/reviews", self.pull_path(number)),
                None,
            )
            .await
    }

    pub async fn request_reviewer(&self, number: i64, username: &str) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::POST,
                &format!("{}/reviewers", self.pull_path(number)),
                Some(&serde_json::json!({"username": username})),
            )
            .await
    }

    pub async fn remove_reviewer(&self, number: i64, username: &str) -> Result<(), Error> {
        let bytes = self
            .client
            .raw(
                crate::Method::DELETE,
                &format!(
                    "{}/reviewers/{}",
                    self.pull_path(number),
                    crate::collab_segment(username)
                ),
                None,
            )
            .await?;
        crate::error::json_or_null(&bytes)?;
        Ok(())
    }

    /// List pull requests, optionally filtered by state (paginated).
    pub fn list(&self, state: Option<&str>) -> Paginator<PullRequest> {
        let path = match state {
            Some(state) => format!("{}?state={state}", self.pulls_path()),
            None => self.pulls_path(),
        };
        self.client.paginate(&path)
    }

    /// Raw JSON twin of [`Pulls::list`].
    pub fn list_raw(&self, state: Option<&str>) -> Paginator<Value> {
        let path = match state {
            Some(state) => format!("{}?state={state}", self.pulls_path()),
            None => self.pulls_path(),
        };
        self.client.paginate(&path)
    }

    /// `GET …/pulls/{number}`.
    pub async fn get(&self, number: i64) -> Result<PullRequest, Error> {
        self.client
            .send(crate::Method::GET, &self.pull_path(number), None)
            .await
    }

    /// Raw JSON twin of [`Pulls::get`].
    pub async fn get_raw(&self, number: i64) -> Result<Value, Error> {
        self.client
            .send(crate::Method::GET, &self.pull_path(number), None)
            .await
    }

    /// `POST …/pulls`.
    pub async fn create(&self, options: &CreatePrOptions) -> Result<PullRequest, Error> {
        self.client
            .send(
                crate::Method::POST,
                &self.pulls_path(),
                Some(&to_body(options)?),
            )
            .await
    }

    /// Raw JSON twin of [`Pulls::create`].
    pub async fn create_raw(&self, options: &CreatePrOptions) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::POST,
                &self.pulls_path(),
                Some(&to_body(options)?),
            )
            .await
    }

    /// `GET …/pulls/{number}/diff` — plain text.
    pub async fn diff(&self, number: i64) -> Result<String, Error> {
        let bytes = self
            .client
            .raw(
                crate::Method::GET,
                &format!("{}/diff", self.pull_path(number)),
                None,
            )
            .await?;
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }

    /// `POST …/pulls/{number}/merge`.
    pub async fn merge(&self, number: i64, strategy: MergeStrategy) -> Result<MergeOutcome, Error> {
        self.merge_with_expected_head(number, strategy, None).await
    }

    /// Raw JSON twin of [`Pulls::merge`].
    pub async fn merge_raw(&self, number: i64, strategy: MergeStrategy) -> Result<Value, Error> {
        self.merge_with_expected_head_raw(number, strategy, None)
            .await
    }

    /// `POST …/pulls/{number}/merge` guarded by `expected_head_sha`. The
    /// server merges nothing and answers 409 `REF_ADVANCED` when the head
    /// branch no longer points at that commit. `None` sends the same body as
    /// [`Pulls::merge`]. The SHA must be the full, lower-case commit SHA.
    pub async fn merge_with_expected_head(
        &self,
        number: i64,
        strategy: MergeStrategy,
        expected_head_sha: Option<&str>,
    ) -> Result<MergeOutcome, Error> {
        let value: Value = self
            .merge_with_expected_head_raw(number, strategy, expected_head_sha)
            .await?;
        serde_json::from_value(value).map_err(|source| Error::Decode {
            body_snippet: "merge response".to_string(),
            source,
        })
    }

    /// Raw JSON twin of [`Pulls::merge_with_expected_head`].
    pub async fn merge_with_expected_head_raw(
        &self,
        number: i64,
        strategy: MergeStrategy,
        expected_head_sha: Option<&str>,
    ) -> Result<Value, Error> {
        let mut body = serde_json::json!({ "strategy": strategy.as_str() });
        if let Some(sha) = expected_head_sha {
            body["expected_head_sha"] = Value::String(sha.to_string());
        }
        self.client
            .send(
                crate::Method::POST,
                &format!("{}/merge", self.pull_path(number)),
                Some(&body),
            )
            .await
    }

    /// `GET …/pulls/{number}/merge-readiness`: what stands between the pull
    /// request and a merge (conflicts, protection gates, required checks).
    /// Read-only. `strategy` defaults to `merge` on the server;
    /// `conflicts_after` continues a cut conflict list after that path.
    pub async fn merge_readiness_raw(
        &self,
        number: i64,
        strategy: Option<MergeStrategy>,
        conflicts_after: Option<&str>,
    ) -> Result<Value, Error> {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        if let Some(strategy) = strategy {
            query.append_pair("strategy", strategy.as_str());
        }
        if let Some(path) = conflicts_after {
            query.append_pair("conflicts_after", path);
        }
        let query = query.finish();
        let mut path = format!("{}/merge-readiness", self.pull_path(number));
        if !query.is_empty() {
            path.push('?');
            path.push_str(&query);
        }
        self.client.send(crate::Method::GET, &path, None).await
    }

    /// `POST …/pulls/{number}/update-branch`: merge the base branch into the
    /// head branch, built from exactly `expected_head_sha` (409 `REF_ADVANCED`
    /// when the head moved, `MERGE_CONFLICTS` when the base does not merge).
    pub async fn update_branch_raw(
        &self,
        number: i64,
        expected_head_sha: &str,
        dry_run: bool,
    ) -> Result<Value, Error> {
        let mut body = serde_json::json!({ "expected_head_sha": expected_head_sha });
        if dry_run {
            body["dry_run"] = Value::Bool(true);
        }
        self.client
            .send(
                crate::Method::POST,
                &format!("{}/update-branch", self.pull_path(number)),
                Some(&body),
            )
            .await
    }

    /// `PUT …/pulls/{number}/auto-merge` — enable auto-merge for when the
    /// required checks pass.
    pub async fn enable_auto_merge(
        &self,
        number: i64,
        strategy: MergeStrategy,
    ) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::PUT,
                &format!("{}/auto-merge", self.pull_path(number)),
                Some(&serde_json::json!({ "strategy": strategy.as_str() })),
            )
            .await
    }

    /// `GET …/pulls/{number}/comments` — inline review comments (plain array).
    pub async fn comments(&self, number: i64) -> Result<Vec<ReviewComment>, Error> {
        self.client
            .send(
                crate::Method::GET,
                &format!("{}/{number}/comments", self.pulls_path()),
                None,
            )
            .await
    }

    /// Raw JSON twin of [`Pulls::comments`].
    pub async fn comments_raw(&self, number: i64) -> Result<Value, Error> {
        let bytes = self
            .client
            .raw(
                crate::Method::GET,
                &format!("{}/{number}/comments", self.pulls_path()),
                None,
            )
            .await?;
        serde_json::from_slice(&bytes).map_err(|source| Error::Decode {
            body_snippet: crate::error::snippet(&bytes),
            source,
        })
    }

    /// `POST …/pulls/{number}/reviews`. The response shape varies by action,
    /// so only the raw JSON twin exists.
    pub async fn review_raw(
        &self,
        number: i64,
        action: ReviewAction,
        body: Option<&str>,
    ) -> Result<Value, Error> {
        let mut payload = serde_json::json!({ "action": action.as_str() });
        if let Some(body) = body {
            payload["body"] = Value::String(body.to_string());
        }
        self.client
            .send(
                crate::Method::POST,
                &format!("{}/reviews", self.pull_path(number)),
                Some(&payload),
            )
            .await
    }
}
