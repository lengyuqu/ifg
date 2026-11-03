//! Labels resource group (`/repos/{owner}/{repo}/labels`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Error;
use crate::http::Client;
use crate::pagination::Paginator;
use crate::repos::{repo_path, to_body};

/// A repository label.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Label {
    pub id: i64,
    pub repo_id: i64,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// `POST …/labels` body (and the patchable fields for updates).
#[derive(Debug, Default, Serialize)]
pub struct LabelOptions {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Accessor for label endpoints of one repository.
pub struct Labels<'a> {
    client: &'a Client,
    owner: String,
    repo: String,
}

impl Client {
    /// Label endpoints of `owner/repo`.
    pub fn labels(&self, owner: &str, repo: &str) -> Labels<'_> {
        Labels {
            client: self,
            owner: owner.to_string(),
            repo: repo.to_string(),
        }
    }
}

impl Labels<'_> {
    fn path(&self) -> String {
        format!("{}/labels", repo_path(&self.owner, &self.repo))
    }

    fn label_path(&self, id: i64) -> String {
        format!("{}/{id}", self.path())
    }

    /// List labels (paginated).
    pub fn list(&self) -> Paginator<Label> {
        self.client.paginate(self.path())
    }

    /// Raw JSON twin of [`Labels::list`].
    pub fn list_raw(&self) -> Paginator<Value> {
        self.client.paginate(self.path())
    }

    /// `POST …/labels`.
    pub async fn create(&self, options: &LabelOptions) -> Result<Label, Error> {
        self.client
            .send(crate::Method::POST, &self.path(), Some(&to_body(options)?))
            .await
    }

    /// Raw JSON twin of [`Labels::create`].
    pub async fn create_raw(&self, options: &LabelOptions) -> Result<Value, Error> {
        self.client
            .send(crate::Method::POST, &self.path(), Some(&to_body(options)?))
            .await
    }

    /// `PUT …/labels/{id}` (the server takes the full label body).
    pub async fn update(&self, id: i64, options: &LabelOptions) -> Result<Label, Error> {
        self.client
            .send(
                crate::Method::PATCH,
                &self.label_path(id),
                Some(&to_body(options)?),
            )
            .await
    }

    /// Raw JSON twin of [`Labels::update`].
    pub async fn update_raw(&self, id: i64, options: &LabelOptions) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::PATCH,
                &self.label_path(id),
                Some(&to_body(options)?),
            )
            .await
    }

    /// `DELETE …/labels/{id}`.
    pub async fn delete(&self, id: i64) -> Result<Value, Error> {
        let bytes = self
            .client
            .raw(crate::Method::DELETE, &self.label_path(id), None)
            .await?;
        Ok(crate::error::json_or_null(&bytes))
    }
}
