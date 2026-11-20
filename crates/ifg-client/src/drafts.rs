//! Agent draft namespaces (`/repos/{owner}/{name}/agent-drafts`).
//!
//! A draft namespace reserves a branch directory (`agent/<id>/`) whose pushes
//! deliver no webhooks or mail and which the server deletes once its TTL and
//! grace period have passed. Only raw JSON twins exist: the CLI prints the
//! server's `DraftNamespaceResponse` objects as sent.

use serde_json::{json, Value};

use crate::error::Error;
use crate::http::Client;
use crate::repos::repo_path;

/// Accessor for the draft namespaces of one repository.
pub struct Drafts<'a> {
    client: &'a Client,
    owner: String,
    repo: String,
}

impl Client {
    /// Draft namespace endpoints of `owner/repo`.
    pub fn drafts(&self, owner: &str, repo: &str) -> Drafts<'_> {
        Drafts {
            client: self,
            owner: owner.to_string(),
            repo: repo.to_string(),
        }
    }
}

impl Drafts<'_> {
    fn path(&self, tail: &str) -> String {
        format!("{}/agent-drafts{tail}", repo_path(&self.owner, &self.repo))
    }

    /// `GET …/agent-drafts`: the repository's namespaces.
    pub async fn list_raw(&self) -> Result<Value, Error> {
        self.client
            .send(crate::Method::GET, &self.path(""), None)
            .await
    }

    /// `POST …/agent-drafts`: reserve `prefix` (e.g. `agent/42/`) for
    /// `ttl_secs`. `grace_secs` is how long the branches survive after expiry
    /// (server default one day).
    pub async fn create_raw(
        &self,
        prefix: &str,
        ttl_secs: i64,
        grace_secs: Option<i64>,
    ) -> Result<Value, Error> {
        let mut body = json!({ "prefix": prefix, "ttl_secs": ttl_secs });
        if let Some(grace) = grace_secs {
            body["grace_secs"] = json!(grace);
        }
        self.client
            .send(crate::Method::POST, &self.path(""), Some(&body))
            .await
    }

    /// `PATCH …/agent-drafts/{id}`: add `ttl_secs` to the deadline. The server
    /// never shortens it and caps it at `created_at + 4 × ttl`.
    pub async fn extend_raw(&self, id: i64, ttl_secs: i64) -> Result<Value, Error> {
        let body = json!({ "ttl_secs": ttl_secs });
        self.client
            .send(
                crate::Method::PATCH,
                &self.path(&format!("/{id}")),
                Some(&body),
            )
            .await
    }

    /// `DELETE …/agent-drafts/{id}`: close the gate now; the server deletes
    /// the branches after the grace period.
    pub async fn close_raw(&self, id: i64) -> Result<Value, Error> {
        self.client
            .send(crate::Method::DELETE, &self.path(&format!("/{id}")), None)
            .await
    }
}
