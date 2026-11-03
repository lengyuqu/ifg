//! Global search (`GET /search`).
//!
//! One endpoint covers repositories, issues and wiki pages; the result items
//! are uniform (`SearchResult`), so the CLI can render a single table.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Error;
use crate::http::Client;

/// One search hit.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchHit {
    /// `repo` / `issue` / `wiki`
    pub result_type: String,
    pub id: i64,
    pub title: String,
    #[serde(default)]
    pub excerpt: Option<String>,
    #[serde(default)]
    pub repo_owner: Option<String>,
    #[serde(default)]
    pub repo_name: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub number: Option<i64>,
}

impl SearchHit {
    /// `OWNER/NAME` when the hit belongs to a repository.
    pub fn repo_label(&self) -> String {
        match (&self.repo_owner, &self.repo_name) {
            (Some(owner), Some(name)) => format!("{owner}/{name}"),
            _ => "—".to_string(),
        }
    }
}

/// The paginated search envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchPage {
    pub results: Vec<SearchHit>,
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub page: u64,
    #[serde(default)]
    pub per_page: u64,
}

/// Accessor for the global search endpoint.
pub struct Search<'a> {
    client: &'a Client,
}

impl Client {
    /// Search endpoints.
    pub fn search(&self) -> Search<'_> {
        Search { client: self }
    }
}

impl Search<'_> {
    /// `GET /search?q=&type=&page=&per_page=`.
    pub async fn query(
        &self,
        query: &str,
        search_type: &str,
        page: u64,
        per_page: u64,
    ) -> Result<SearchPage, Error> {
        let encoded = urlencode(query);
        self.client
            .send(
                crate::Method::GET,
                &format!("search?q={encoded}&type={search_type}&page={page}&per_page={per_page}"),
                None,
            )
            .await
    }

    /// Raw JSON twin of [`Search::query`].
    pub async fn query_raw(
        &self,
        query: &str,
        search_type: &str,
        page: u64,
        per_page: u64,
    ) -> Result<Value, Error> {
        let encoded = urlencode(query);
        self.client
            .send(
                crate::Method::GET,
                &format!("search?q={encoded}&type={search_type}&page={page}&per_page={per_page}"),
                None,
            )
            .await
    }
}

/// Percent-encode everything but RFC 3986 unreserved characters.
fn urlencode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_queries() {
        assert_eq!(urlencode("fix bug"), "fix+bug");
        assert_eq!(urlencode("a/b?c=d"), "a%2Fb%3Fc%3Dd");
        assert_eq!(
            urlencode("平滑 重构"),
            "%E5%B9%B3%E6%BB%91+%E9%87%8D%E6%9E%84"
        );
    }
}
