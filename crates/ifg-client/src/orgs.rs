//! Organizations (`/orgs`): list, view, create, members.
//!
//! Members are addressed by username in the CLI; the server wants ids, so
//! [`Orgs::add_member`] takes an id and the CLI resolves the name first.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Error;
use crate::http::Client;

/// An organization.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Organization {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub owner_id: Option<i64>,
    /// public / private
    #[serde(default)]
    pub visibility: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// An organization member (the server resolves the username for display).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OrgMember {
    pub id: i64,
    pub org_id: i64,
    pub user_id: i64,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    /// owner / admin / member
    pub role: String,
    #[serde(default)]
    pub created_at: Option<String>,
}

impl OrgMember {
    /// Username when the server resolved it, else the numeric id.
    pub fn label(&self) -> String {
        self.username
            .clone()
            .unwrap_or_else(|| format!("#{}", self.user_id))
    }
}

/// `POST /orgs` body.
#[derive(Debug, Default, Serialize)]
pub struct CreateOrgOptions {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// public / private
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
}

/// Accessor for the organization endpoints.
pub struct Orgs<'a> {
    client: &'a Client,
}

impl Client {
    /// Organization endpoints.
    pub fn orgs(&self) -> Orgs<'_> {
        Orgs { client: self }
    }
}

impl Orgs<'_> {
    /// `GET /orgs` — the organizations the caller belongs to (plain array).
    pub async fn list(&self) -> Result<Vec<Organization>, Error> {
        self.client.send(crate::Method::GET, "orgs", None).await
    }

    /// Raw JSON twin of [`Orgs::list`].
    pub async fn list_raw(&self) -> Result<Value, Error> {
        self.client.send(crate::Method::GET, "orgs", None).await
    }

    /// `GET /orgs/{name}`.
    pub async fn get(&self, name: &str) -> Result<Organization, Error> {
        self.client
            .send(
                crate::Method::GET,
                &format!("orgs/{}", crate::path_segment(name)),
                None,
            )
            .await
    }

    /// Raw JSON twin of [`Orgs::get`].
    pub async fn get_raw(&self, name: &str) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::GET,
                &format!("orgs/{}", crate::path_segment(name)),
                None,
            )
            .await
    }

    /// `POST /orgs`.
    pub async fn create(&self, options: &CreateOrgOptions) -> Result<Organization, Error> {
        self.client
            .send(
                crate::Method::POST,
                "orgs",
                Some(&serde_json::to_value(options).map_err(|e| Error::Config(e.to_string()))?),
            )
            .await
    }

    /// Raw JSON twin of [`Orgs::create`].
    pub async fn create_raw(&self, options: &CreateOrgOptions) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::POST,
                "orgs",
                Some(&serde_json::to_value(options).map_err(|e| Error::Config(e.to_string()))?),
            )
            .await
    }

    /// `GET /orgs/{name}/members`.
    pub async fn members(&self, name: &str) -> Result<Vec<OrgMember>, Error> {
        self.client
            .send(
                crate::Method::GET,
                &format!("orgs/{}/members", crate::path_segment(name)),
                None,
            )
            .await
    }

    /// Raw JSON twin of [`Orgs::members`].
    pub async fn members_raw(&self, name: &str) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::GET,
                &format!("orgs/{}/members", crate::path_segment(name)),
                None,
            )
            .await
    }

    /// `POST /orgs/{name}/members` — `role` is owner/admin/member.
    pub async fn add_member(
        &self,
        name: &str,
        user_id: i64,
        role: Option<&str>,
    ) -> Result<Value, Error> {
        let mut body = serde_json::json!({ "user_id": user_id });
        if let Some(role) = role {
            body["role"] = serde_json::Value::String(role.to_string());
        }
        self.client
            .send(
                crate::Method::POST,
                &format!("orgs/{}/members", crate::path_segment(name)),
                Some(&body),
            )
            .await
    }

    /// `DELETE /orgs/{name}/members/{user_id}`.
    pub async fn remove_member(&self, name: &str, user_id: i64) -> Result<Value, Error> {
        let bytes = self
            .client
            .raw(
                crate::Method::DELETE,
                &format!("orgs/{}/members/{user_id}", crate::path_segment(name)),
                None,
            )
            .await?;
        Ok(crate::error::json_or_null(&bytes))
    }
}
