//! `repos` resource group. DTOs mirror the server's `RepoResponse` /
//! `CreateRepoRequest` field names; only fields the CLI displays are
//! declared (D4: unknown fields ignored).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Error;
use crate::http::Client;
use crate::pagination::Paginator;

/// Repository view (`GET /repos/{owner}/{name}`, list item, create result).
#[derive(Debug, Clone, Deserialize)]
pub struct Repo {
    pub id: i64,
    pub owner_id: i64,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub is_private: bool,
    pub default_branch: String,
    pub stars_count: i64,
    pub forks_count: i64,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// `POST /repos` request body. Field names match the server exactly;
/// `Option` fields are omitted from the JSON when unset.
#[derive(Debug, Default, Serialize)]
pub struct CreateRepoOptions {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_private: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_init: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gitignores: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue_labels: Option<String>,
}

pub(crate) fn to_body<T: Serialize>(value: &T) -> Result<Value, Error> {
    serde_json::to_value(value).map_err(|e| Error::Config(format!("could not encode request: {e}")))
}

/// Accessor for repository endpoints; borrows the client, holds no state.
pub struct Repos<'a> {
    client: &'a Client,
}

impl Client {
    /// Repository endpoints.
    pub fn repos(&self) -> Repos<'_> {
        Repos { client: self }
    }
}

impl Repos<'_> {
    /// List a user's or organization's repositories (paginated).
    pub fn list(&self, owner: &str) -> Paginator<Repo> {
        self.client.paginate(format!("repos/{owner}"))
    }

    /// Raw JSON twin of [`Repos::list`] for `--json` (D5).
    pub fn list_raw(&self, owner: &str) -> Paginator<Value> {
        self.client.paginate(format!("repos/{owner}"))
    }

    /// `GET /repos/{owner}/{name}`.
    pub async fn get(&self, owner: &str, name: &str) -> Result<Repo, Error> {
        self.client
            .send(crate::Method::GET, &repo_path(owner, name), None)
            .await
    }

    /// Raw JSON twin of [`Repos::get`].
    pub async fn get_raw(&self, owner: &str, name: &str) -> Result<Value, Error> {
        self.client
            .send(crate::Method::GET, &repo_path(owner, name), None)
            .await
    }

    /// `POST /repos`.
    pub async fn create(&self, options: &CreateRepoOptions) -> Result<Repo, Error> {
        self.client
            .send(crate::Method::POST, "repos", Some(&to_body(options)?))
            .await
    }

    /// Raw JSON twin of [`Repos::create`].
    pub async fn create_raw(&self, options: &CreateRepoOptions) -> Result<Value, Error> {
        self.client
            .send(crate::Method::POST, "repos", Some(&to_body(options)?))
            .await
    }

    /// `DELETE /repos/{owner}/{name}` — used by e2e cleanup.
    pub async fn delete(&self, owner: &str, name: &str) -> Result<(), Error> {
        self.client
            .raw(crate::Method::DELETE, &repo_path(owner, name), None)
            .await?;
        Ok(())
    }
}

pub(crate) fn repo_path(owner: &str, name: &str) -> String {
    format!("repos/{owner}/{name}")
}
