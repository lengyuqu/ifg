//! Milestones resource group (`/repos/{owner}/{repo}/milestones`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Error;
use crate::http::Client;
use crate::pagination::Paginator;
use crate::repos::{repo_path, to_body};

/// A repository milestone.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Milestone {
    pub id: i64,
    pub repo_id: i64,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub due_date: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// `POST …/milestones` body.
#[derive(Debug, Default, Serialize)]
pub struct MilestoneOptions {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// open / closed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
}

/// Accessor for milestone endpoints of one repository.
pub struct Milestones<'a> {
    client: &'a Client,
    owner: String,
    repo: String,
}

impl Client {
    /// Milestone endpoints of `owner/repo`.
    pub fn milestones(&self, owner: &str, repo: &str) -> Milestones<'_> {
        Milestones {
            client: self,
            owner: owner.to_string(),
            repo: repo.to_string(),
        }
    }
}

impl Milestones<'_> {
    fn path(&self) -> String {
        format!("{}/milestones", repo_path(&self.owner, &self.repo))
    }

    fn milestone_path(&self, id: i64) -> String {
        format!("{}/{id}", self.path())
    }

    /// List milestones (paginated; server may return all).
    pub fn list(&self) -> Paginator<Milestone> {
        self.client.paginate(self.path())
    }

    /// Raw JSON twin of [`Milestones::list`].
    pub fn list_raw(&self) -> Paginator<Value> {
        self.client.paginate(self.path())
    }

    /// `POST …/milestones`.
    pub async fn create(&self, options: &MilestoneOptions) -> Result<Milestone, Error> {
        self.client
            .send(crate::Method::POST, &self.path(), Some(&to_body(options)?))
            .await
    }

    /// Raw JSON twin of [`Milestones::create`].
    pub async fn create_raw(&self, options: &MilestoneOptions) -> Result<Value, Error> {
        self.client
            .send(crate::Method::POST, &self.path(), Some(&to_body(options)?))
            .await
    }

    /// `PUT …/milestones/{id}`.
    pub async fn update(&self, id: i64, options: &MilestoneOptions) -> Result<Milestone, Error> {
        self.client
            .send(
                crate::Method::PATCH,
                &self.milestone_path(id),
                Some(&to_body(options)?),
            )
            .await
    }

    /// Raw JSON twin of [`Milestones::update`].
    pub async fn update_raw(&self, id: i64, options: &MilestoneOptions) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::PATCH,
                &self.milestone_path(id),
                Some(&to_body(options)?),
            )
            .await
    }

    /// `DELETE …/milestones/{id}`.
    pub async fn delete(&self, id: i64) -> Result<Value, Error> {
        let bytes = self
            .client
            .raw(crate::Method::DELETE, &self.milestone_path(id), None)
            .await?;
        Ok(crate::error::json_or_null(&bytes))
    }
}
