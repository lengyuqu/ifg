//! `issues` resource group (also used by PR comments where the API matches).
//! DTOs mirror the server's flattened issue model + enrichments
//! (`author`, `assignees`); `labels` is a JSON-encoded string server-side.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Error;
use crate::http::Client;
use crate::pagination::Paginator;
use crate::repos::{repo_path, to_body};

/// Issue view. The server stores `labels` as a JSON-encoded string
/// (`"[\"bug\"]"`); parse it with [`Issue::label_list`].
#[derive(Debug, Clone, Deserialize)]
pub struct Issue {
    pub id: i64,
    pub repo_id: i64,
    pub number: i64,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    /// `open` / `closed`.
    pub state: String,
    pub author_id: i64,
    #[serde(default)]
    pub labels: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub assignees: Vec<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub closed_at: Option<String>,
}

impl Issue {
    /// `labels` decoded into plain strings (`[]` when absent or malformed).
    pub fn label_list(&self) -> Vec<String> {
        self.labels
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_default()
    }
}

/// `GET /repos/{owner}/{name}/issues` filters (server `ListQuery`).
#[derive(Debug, Default, Clone)]
pub struct IssueListFilter {
    /// `open` / `closed` / `all`.
    pub state: Option<String>,
    /// Comma-separated label names.
    pub labels: Option<String>,
    /// Assignee username.
    pub assignee: Option<String>,
}

impl IssueListFilter {
    fn to_query(&self) -> String {
        let mut pairs = Vec::new();
        if let Some(state) = &self.state {
            pairs.push(format!("state={state}"));
        }
        if let Some(labels) = &self.labels {
            pairs.push(format!("labels={labels}"));
        }
        if let Some(assignee) = &self.assignee {
            pairs.push(format!("assignee={assignee}"));
        }
        pairs.join("&")
    }
}

/// `POST …/issues` request body.
#[derive(Debug, Default, Serialize)]
pub struct CreateIssueOptions {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignees: Option<Vec<String>>,
}

/// `PATCH …/issues/{number}` request body — close, reopen and edit.
#[derive(Debug, Default, Serialize)]
pub struct UpdateIssueOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// `open` / `closed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// Replace the label set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<String>>,
    /// Assign a user by id (the CLI resolves names).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee_id: Option<i64>,
    /// Attach a milestone by id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub milestone_id: Option<i64>,
}

/// Issue comment (plain JSON array endpoint, not paginated).
#[derive(Debug, Clone, Deserialize)]
pub struct Comment {
    pub id: i64,
    pub issue_id: i64,
    pub author_id: i64,
    pub body: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// Accessor for issue endpoints of one repository.
pub struct Issues<'a> {
    client: &'a Client,
    owner: String,
    repo: String,
}

impl Client {
    /// Issue endpoints of `owner/repo`.
    pub fn issues(&self, owner: &str, repo: &str) -> Issues<'_> {
        Issues {
            client: self,
            owner: owner.to_string(),
            repo: repo.to_string(),
        }
    }
}

impl Issues<'_> {
    fn issues_path(&self) -> String {
        format!("{}/issues", repo_path(&self.owner, &self.repo))
    }

    fn issue_path(&self, number: i64) -> String {
        format!("{}/issues/{number}", repo_path(&self.owner, &self.repo))
    }

    fn comments_path(&self, number: i64) -> String {
        format!("{}/comments", self.issue_path(number))
    }

    /// List issues with filters (paginated).
    pub fn list(&self, filter: &IssueListFilter) -> Paginator<Issue> {
        let query = filter.to_query();
        let path = if query.is_empty() {
            self.issues_path()
        } else {
            format!("{}?{query}", self.issues_path())
        };
        self.client.paginate(&path)
    }

    /// Raw JSON twin of [`Issues::list`].
    pub fn list_raw(&self, filter: &IssueListFilter) -> Paginator<Value> {
        let query = filter.to_query();
        let path = if query.is_empty() {
            self.issues_path()
        } else {
            format!("{}?{query}", self.issues_path())
        };
        self.client.paginate(&path)
    }

    /// `GET …/issues/{number}`.
    pub async fn get(&self, number: i64) -> Result<Issue, Error> {
        self.client
            .send(crate::Method::GET, &self.issue_path(number), None)
            .await
    }

    /// Raw JSON twin of [`Issues::get`].
    pub async fn get_raw(&self, number: i64) -> Result<Value, Error> {
        self.client
            .send(crate::Method::GET, &self.issue_path(number), None)
            .await
    }

    /// `POST …/issues`.
    pub async fn create(&self, options: &CreateIssueOptions) -> Result<Issue, Error> {
        self.client
            .send(
                crate::Method::POST,
                &self.issues_path(),
                Some(&to_body(options)?),
            )
            .await
    }

    /// Raw JSON twin of [`Issues::create`].
    pub async fn create_raw(&self, options: &CreateIssueOptions) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::POST,
                &self.issues_path(),
                Some(&to_body(options)?),
            )
            .await
    }

    /// `PATCH …/issues/{number}` — close/reopen/edit.
    pub async fn update(&self, number: i64, options: &UpdateIssueOptions) -> Result<Issue, Error> {
        self.client
            .send(
                crate::Method::PATCH,
                &self.issue_path(number),
                Some(&to_body(options)?),
            )
            .await
    }

    /// Raw JSON twin of [`Issues::update`].
    pub async fn update_raw(
        &self,
        number: i64,
        options: &UpdateIssueOptions,
    ) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::PATCH,
                &self.issue_path(number),
                Some(&to_body(options)?),
            )
            .await
    }

    /// `GET …/issues/{number}/comments` — plain array.
    pub async fn comments(&self, number: i64) -> Result<Vec<Comment>, Error> {
        self.client
            .send(crate::Method::GET, &self.comments_path(number), None)
            .await
    }

    /// Raw JSON twin of [`Issues::comments`].
    pub async fn comments_raw(&self, number: i64) -> Result<Value, Error> {
        self.client
            .send(crate::Method::GET, &self.comments_path(number), None)
            .await
    }

    /// `POST …/issues/{number}/comments`.
    pub async fn add_comment(&self, number: i64, body: &str) -> Result<Comment, Error> {
        self.client
            .send(
                crate::Method::POST,
                &self.comments_path(number),
                Some(&serde_json::json!({ "body": body })),
            )
            .await
    }

    /// Raw JSON twin of [`Issues::add_comment`].
    pub async fn add_comment_raw(&self, number: i64, body: &str) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::POST,
                &self.comments_path(number),
                Some(&serde_json::json!({ "body": body })),
            )
            .await
    }
}
