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
    #[serde(default)]
    pub settings_version: Option<String>,
    pub stars_count: i64,
    pub forks_count: i64,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Repository collaborator row. `id` is the collaborator record ID, whereas
/// `user_id` identifies the user and is used by the delete endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct Collaborator {
    pub id: i64,
    pub repo_id: i64,
    pub user_id: i64,
    pub permission: String,
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
    pub async fn collaborators(&self, owner: &str, name: &str) -> Result<Vec<Collaborator>, Error> {
        self.client
            .send(
                crate::Method::GET,
                &format!("{}/collaborators", repo_path(owner, name)),
                None,
            )
            .await
    }

    pub async fn collaborators_raw(&self, owner: &str, name: &str) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::GET,
                &format!("{}/collaborators", repo_path(owner, name)),
                None,
            )
            .await
    }

    pub async fn add_collaborator(
        &self,
        owner: &str,
        name: &str,
        username: &str,
        permission: &str,
    ) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::POST,
                &format!("{}/collaborators", repo_path(owner, name)),
                Some(&serde_json::json!({"username": username, "permission": permission})),
            )
            .await
    }

    pub async fn update_collaborator(
        &self,
        owner: &str,
        name: &str,
        collaborator_id: i64,
        permission: &str,
    ) -> Result<Value, Error> {
        let response: Value = self
            .client
            .send(
                crate::Method::PATCH,
                &format!("{}/collaborators/{collaborator_id}", repo_path(owner, name)),
                Some(&serde_json::json!({"permission": permission})),
            )
            .await?;
        let actual = self.collaborators(owner, name).await?;
        let persisted = actual.iter().find(|entry| entry.id == collaborator_id);
        if !persisted.is_some_and(|entry| entry.permission == permission) {
            return Err(Error::StateMismatch(format!("collaborator {collaborator_id} permission is not {permission}; re-read the repository collaborators")));
        }
        Ok(response)
    }

    pub async fn remove_collaborator(
        &self,
        owner: &str,
        name: &str,
        user_id: i64,
    ) -> Result<(), Error> {
        self.client
            .raw(
                crate::Method::DELETE,
                &format!("{}/collaborators/{user_id}", repo_path(owner, name)),
                None,
            )
            .await?;
        Ok(())
    }
    /// List a user's or organization's repositories (paginated).
    pub fn list(&self, owner: &str) -> Paginator<Repo> {
        self.client
            .paginate(format!("repos/{}", crate::path_segment(owner)))
    }

    /// Raw JSON twin of [`Repos::list`] for `--json` (D5).
    pub fn list_raw(&self, owner: &str) -> Paginator<Value> {
        self.client
            .paginate(format!("repos/{}", crate::path_segment(owner)))
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

    /// Update repository settings with the version read from GET.
    pub async fn update_settings(
        &self,
        owner: &str,
        name: &str,
        body: &Value,
    ) -> Result<Value, Error> {
        self.client
            .send(crate::Method::PATCH, &repo_path(owner, name), Some(body))
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

/// `repos/{owner}/{name}` with both segments encoded. Without this, an owner
/// or name containing `..` or `?` rewrites the request route: `-R '../users'`
/// used to reach `/api/v1/users`, and `-R 'x/y?per_page=1'` turned the rest
/// of the path into a query string.
pub(crate) fn repo_path(owner: &str, name: &str) -> String {
    format!(
        "repos/{}/{}",
        crate::path_segment(owner),
        crate::path_segment(name)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A segment must never be able to add a path separator or a query.
    #[test]
    fn repo_path_encodes_route_changing_segments() {
        assert_eq!(repo_path("alice", "demo"), "repos/alice/demo");
        assert_eq!(repo_path("..", "demo"), "repos/%252E%252E/demo");
        assert_eq!(repo_path("../users", "demo"), "repos/..%2Fusers/demo");
        assert_eq!(repo_path("a", "b?x=1"), "repos/a/b%3Fx%3D1");
        assert_eq!(repo_path("a/b", "c"), "repos/a%2Fb/c");
    }
}
