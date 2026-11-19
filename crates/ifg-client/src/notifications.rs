//! Notifications (`/notifications`): list, unread count, mark read, delete.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Error;
use crate::http::Client;
use crate::pagination::Paginator;

/// One notification.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Notification {
    pub id: i64,
    pub user_id: i64,
    /// push / issue / pr / review / pipeline …
    pub event_type: String,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub repo_id: Option<i64>,
    pub is_read: bool,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// Accessor for the notification endpoints of the current user.
pub struct Notifications<'a> {
    client: &'a Client,
}

impl Client {
    /// Notification endpoints.
    pub fn notifications(&self) -> Notifications<'_> {
        Notifications { client: self }
    }
}

impl Notifications<'_> {
    /// `GET /notifications?unread_only=` — the server answers with
    /// `{data, pagination}`, so callers page through it with
    /// [`Paginator::limit`]; a plain array is still accepted.
    pub fn list(&self, unread_only: bool) -> Paginator<Notification> {
        self.client
            .paginate(format!("notifications?unread_only={unread_only}"))
    }

    /// Raw JSON twin of [`Notifications::list`].
    pub fn list_raw(&self, unread_only: bool) -> Paginator<Value> {
        self.client
            .paginate(format!("notifications?unread_only={unread_only}"))
    }

    /// `GET /notifications/unread-count` → the count.
    pub async fn unread_count(&self) -> Result<u64, Error> {
        let value: Value = self
            .client
            .send(crate::Method::GET, "notifications/unread-count", None)
            .await?;
        Ok(value["unread_count"].as_u64().unwrap_or(0))
    }

    /// `POST /notifications/{id}/read`.
    pub async fn mark_read(&self, id: i64) -> Result<Value, Error> {
        let bytes = self
            .client
            .raw(
                crate::Method::POST,
                &format!("notifications/{id}/read"),
                None,
            )
            .await?;
        crate::error::json_or_null(&bytes)
    }

    /// `POST /notifications/mark-all-read`.
    pub async fn mark_all_read(&self) -> Result<Value, Error> {
        let bytes = self
            .client
            .raw(crate::Method::POST, "notifications/mark-all-read", None)
            .await?;
        crate::error::json_or_null(&bytes)
    }

    /// `GET /user/notification-preferences`: the effective preferences, the
    /// catalogue defaults expanded plus the caller's explicit overrides.
    pub async fn preferences_raw(&self) -> Result<Value, Error> {
        self.client
            .send(crate::Method::GET, "user/notification-preferences", None)
            .await
    }

    /// `PUT /user/notification-preferences`: store `overrides`, a bare array of
    /// `{repo_id, event_type, channel, enabled}` (`repo_id: null` changes the
    /// global default). The answer is the new effective preferences.
    pub async fn set_preferences_raw(&self, overrides: &Value) -> Result<Value, Error> {
        self.client
            .send(
                crate::Method::PUT,
                "user/notification-preferences",
                Some(overrides),
            )
            .await
    }

    /// `DELETE /notifications/{id}`.
    pub async fn delete(&self, id: i64) -> Result<Value, Error> {
        let bytes = self
            .client
            .raw(crate::Method::DELETE, &format!("notifications/{id}"), None)
            .await?;
        crate::error::json_or_null(&bytes)
    }
}
