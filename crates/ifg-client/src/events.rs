//! Repository event log (`GET /repos/{owner}/{name}/events`).
//!
//! The OpenAPI snapshot documents the query parameters `since` (exclusive
//! cursor), `types` (comma-separated event names), `limit` and `timeout`,
//! and describes the `200` body only as "Events after the cursor, with
//! next_cursor" with no schema. The body is therefore returned as raw JSON:
//! nothing is decoded into a DTO, so unknown fields survive. `timeout` is
//! never sent, which keeps every call a plain, non-blocking read.

use serde_json::Value;

use crate::error::Error;
use crate::http::Client;
use crate::repos::repo_path;

/// Accessor for one repository's event log.
pub struct Events<'a> {
    client: &'a Client,
    path: String,
}

impl Client {
    /// Event log endpoints of `owner/name`.
    pub fn events(&self, owner: &str, name: &str) -> Events<'_> {
        Events {
            client: self,
            path: repo_path(owner, name),
        }
    }
}

impl Events<'_> {
    /// `GET …/events`, returning the server object unchanged.
    ///
    /// `since` is omitted when `None`, which asks the server for the current
    /// cursor only. `kinds` is sent comma-separated and omitted when empty.
    /// `limit` is omitted when `None`, leaving the server default in place.
    pub async fn list(
        &self,
        since: Option<i64>,
        kinds: &[String],
        limit: Option<u64>,
    ) -> Result<Value, Error> {
        let path = format!("{}/events{}", self.path, query_string(since, kinds, limit));
        self.client.send(reqwest::Method::GET, &path, None).await
    }
}

/// Build the `?since=…&types=…&limit=…` suffix; empty when nothing is set.
pub(crate) fn query_string(since: Option<i64>, kinds: &[String], limit: Option<u64>) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    if let Some(since) = since {
        query.append_pair("since", &since.to_string());
    }
    if !kinds.is_empty() {
        query.append_pair("types", &kinds.join(","));
    }
    if let Some(limit) = limit {
        query.append_pair("limit", &limit.to_string());
    }
    let encoded = query.finish();
    if encoded.is_empty() {
        String::new()
    } else {
        format!("?{encoded}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_is_empty_without_options() {
        assert_eq!(query_string(None, &[], None), "");
    }

    #[test]
    fn query_carries_each_option_once() {
        let kinds = vec!["issue".to_string(), "pr".to_string()];
        assert_eq!(
            query_string(Some(42), &kinds, Some(5)),
            "?since=42&types=issue%2Cpr&limit=5"
        );
        assert_eq!(query_string(Some(0), &[], None), "?since=0");
    }
}
