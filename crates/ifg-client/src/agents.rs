//! Agent accounts a person owns (`/agents`) and the one-time tokens they use.
//!
//! Every route needs a person: an agent's own token is refused. The server
//! answers an agent that belongs to someone else exactly like one that does not
//! exist (404). Only raw JSON twins exist, because the CLI prints the objects
//! as the server sent them.
//!
//! [`Agents::create_token_raw`] returns the response **with the secret**
//! (`token`), the only time the server shows it. Its caller must keep that
//! value out of logs and terminals.

use serde_json::{json, Value};

use crate::error::Error;
use crate::http::Client;

/// Request of [`Agents::create_token_raw`]. Unset fields are left out so the
/// server defaults apply (scope `repo:write`, no repository or branch limit,
/// no expiry).
#[derive(Debug, Clone, Default)]
pub struct AgentTokenRequest {
    pub name: String,
    /// Space- or comma-separated scopes; repository scopes only.
    pub scopes: Option<String>,
    /// Limit the token to these repositories (`owner/name`).
    pub repos: Vec<String>,
    /// Limit the branches the token may write (patterns such as `agent/*`);
    /// the server needs `repos` too.
    pub refs: Vec<String>,
    /// RFC 3339 expiry.
    pub expires_at: Option<String>,
}

/// Accessor for the agent endpoints of the current user.
pub struct Agents<'a> {
    client: &'a Client,
}

impl Client {
    /// Agent endpoints.
    pub fn agents(&self) -> Agents<'_> {
        Agents { client: self }
    }
}

fn agent_path(username: &str) -> String {
    format!("agents/{}", crate::path_segment(username))
}

impl Agents<'_> {
    /// `GET /agents`: the caller's agent accounts.
    pub async fn list_raw(&self) -> Result<Value, Error> {
        self.client.send(crate::Method::GET, "agents", None).await
    }

    /// `POST /agents`. The account is named `<your username>-agent-<name>`.
    pub async fn create_raw(&self, name: &str, description: Option<&str>) -> Result<Value, Error> {
        let mut body = json!({ "name": name });
        if let Some(description) = description {
            body["description"] = Value::String(description.to_string());
        }
        self.client
            .send(crate::Method::POST, "agents", Some(&body))
            .await
    }

    /// `GET /agents/{username}`.
    pub async fn get_raw(&self, username: &str) -> Result<Value, Error> {
        self.client
            .send(crate::Method::GET, &agent_path(username), None)
            .await
    }

    /// `PATCH /agents/{username}`. An empty `description` clears it; `None`
    /// leaves each field alone. `is_active: Some(false)` deactivates the agent
    /// (its tokens are refused until it is reactivated).
    pub async fn update_raw(
        &self,
        username: &str,
        description: Option<&str>,
        is_active: Option<bool>,
    ) -> Result<Value, Error> {
        let mut body = json!({});
        if let Some(description) = description {
            body["description"] = Value::String(description.to_string());
        }
        if let Some(active) = is_active {
            body["is_active"] = Value::Bool(active);
        }
        self.client
            .send(crate::Method::PATCH, &agent_path(username), Some(&body))
            .await
    }

    /// `GET /agents/{username}/tokens`: metadata only, never the secrets.
    pub async fn tokens_raw(&self, username: &str) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::GET,
                &format!("{}/tokens", agent_path(username)),
                None,
            )
            .await
    }

    /// `POST /agents/{username}/tokens`. The answer carries the secret in
    /// `token`; it is shown only once.
    pub async fn create_token_raw(
        &self,
        username: &str,
        request: &AgentTokenRequest,
    ) -> Result<Value, Error> {
        let mut body = json!({ "name": request.name });
        if let Some(scopes) = &request.scopes {
            body["scopes"] = Value::String(scopes.clone());
        }
        if !request.repos.is_empty() {
            body["repos"] = json!(request.repos);
        }
        if !request.refs.is_empty() {
            body["refs"] = json!(request.refs);
        }
        if let Some(expires_at) = &request.expires_at {
            body["expires_at"] = Value::String(expires_at.clone());
        }
        self.client
            .send(
                crate::Method::POST,
                &format!("{}/tokens", agent_path(username)),
                Some(&body),
            )
            .await
    }

    /// `DELETE /agents/{username}/tokens/{id}`: revoke one token. The server
    /// answers 204, so the result is `null`.
    pub async fn delete_token_raw(&self, username: &str, id: i64) -> Result<Value, Error> {
        let bytes = self
            .client
            .raw(
                crate::Method::DELETE,
                &format!("{}/tokens/{id}", agent_path(username)),
                None,
            )
            .await?;
        crate::error::json_or_null(&bytes)
    }
}
