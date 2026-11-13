//! AI-oriented aggregate endpoints (`/api/v1/ai/repos/{owner}/{name}/...`).
//!
//! The server builds these objects for agents, and `ifg context --json`
//! prints them unchanged, so every method returns the server's JSON value
//! rather than a display DTO (D5). Fields are documented in the OpenAPI
//! snapshot; `pr_context` has no declared response schema.

use serde_json::Value;

use crate::error::{self, Error};
use crate::http::Client;
use crate::search::urlencode;

/// Query for `ai/repos/{owner}/{name}/tree`. Unset fields are left out so
/// the server defaults apply.
#[derive(Debug, Clone, Default)]
pub struct TreeQuery {
    /// Branch, tag or SHA (default: the default branch).
    pub git_ref: Option<String>,
    /// Subdirectory (default: the root).
    pub path: Option<String>,
    /// Directory levels to descend (server default 1, max 8).
    pub depth: Option<u32>,
    /// Entries per page (server default 200, max 1000).
    pub limit: Option<u64>,
    /// `next_cursor` from the previous page.
    pub cursor: Option<String>,
}

/// Query for `ai/repos/{owner}/{name}/search/code`.
#[derive(Debug, Clone, Default)]
pub struct CodeSearchQuery {
    pub q: String,
    /// Must be the default branch when set; other refs are rejected by the server.
    pub git_ref: Option<String>,
    /// Max results (server default 20, max 100).
    pub limit: Option<u64>,
    /// Results to skip (server default 0).
    pub offset: Option<u64>,
}

/// One code search page: the server's JSON array plus the `x-total-count`
/// header when the server sent it.
#[derive(Debug, Clone)]
pub struct CodeSearchPage {
    pub results: Value,
    pub total: Option<u64>,
}

/// Query for `ai/repos/{owner}/{name}/issues` and `ai/repos/{owner}/{name}/prs`.
/// Unset fields are left out so the server defaults apply.
#[derive(Debug, Clone, Default)]
pub struct AiListQuery {
    /// Issue state (`open` or `closed`; server default `open`) or PR state
    /// (`open`, `closed` or `merged`; server default `open`).
    pub state: Option<String>,
    /// Max results (server default 20).
    pub limit: Option<u64>,
}

/// Query for `ai/repos/{owner}/{name}/pulls/{number}/context`.
#[derive(Debug, Clone, Default)]
pub struct PrContextQuery {
    /// Approximate answer size in tokens (server default 6000, documented
    /// range 500 to 50000). Callers validate the range.
    pub budget: Option<u32>,
}

/// Accessor for the AI aggregate endpoints.
pub struct Ai<'a> {
    client: &'a Client,
}

impl Client {
    /// AI aggregate endpoints.
    pub fn ai(&self) -> Ai<'_> {
        Ai { client: self }
    }
}

impl Ai<'_> {
    /// `GET ai/repos/{owner}/{name}/summary`.
    pub async fn summary(&self, owner: &str, name: &str) -> Result<Value, Error> {
        let path = format!("ai/repos/{}/{}/summary", seg(owner), seg(name));
        self.client.send(crate::Method::GET, &path, None).await
    }

    /// `GET ai/repos/{owner}/{name}/tree`, one page.
    pub async fn tree(&self, owner: &str, name: &str, query: &TreeQuery) -> Result<Value, Error> {
        let path = with_query(
            format!("ai/repos/{}/{}/tree", seg(owner), seg(name)),
            &[
                ("ref", query.git_ref.clone()),
                ("path", query.path.clone()),
                ("depth", query.depth.map(|v| v.to_string())),
                ("limit", query.limit.map(|v| v.to_string())),
                ("cursor", query.cursor.clone()),
            ],
        );
        self.client.send(crate::Method::GET, &path, None).await
    }

    /// `GET ai/repos/{owner}/{name}/search/code`.
    pub async fn search_code(
        &self,
        owner: &str,
        name: &str,
        query: &CodeSearchQuery,
    ) -> Result<CodeSearchPage, Error> {
        let path = with_query(
            format!("ai/repos/{}/{}/search/code", seg(owner), seg(name)),
            &[
                ("q", Some(query.q.clone())),
                ("ref", query.git_ref.clone()),
                ("limit", query.limit.map(|v| v.to_string())),
                ("offset", query.offset.map(|v| v.to_string())),
            ],
        );
        // The total travels in a header, so read the response rather than
        // the body-only helpers.
        let response = self
            .client
            .raw_response(crate::Method::GET, &path, None)
            .await?;
        let total = response
            .headers()
            .get("x-total-count")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok());
        let bytes = response.bytes().await.map_err(Error::Transport)?;
        let results = serde_json::from_slice(&bytes).map_err(|source| Error::Decode {
            body_snippet: error::snippet(&bytes),
            source,
        })?;
        Ok(CodeSearchPage { results, total })
    }

    /// `GET ai/repos/{owner}/{name}/pulls/{number}/context` with the server's
    /// default budget (no `budget` parameter is sent).
    pub async fn pr_context(&self, owner: &str, name: &str, number: i64) -> Result<Value, Error> {
        self.pr_context_with(owner, name, number, &PrContextQuery::default())
            .await
    }

    /// `GET ai/repos/{owner}/{name}/pulls/{number}/context` with the given query.
    pub async fn pr_context_with(
        &self,
        owner: &str,
        name: &str,
        number: i64,
        query: &PrContextQuery,
    ) -> Result<Value, Error> {
        let path = with_query(
            format!(
                "ai/repos/{}/{}/pulls/{number}/context",
                seg(owner),
                seg(name)
            ),
            &[("budget", query.budget.map(|v| v.to_string()))],
        );
        self.client.send(crate::Method::GET, &path, None).await
    }

    /// `GET ai/repos/{owner}/{name}/issues`: the server's array of issue summaries.
    pub async fn issues(
        &self,
        owner: &str,
        name: &str,
        query: &AiListQuery,
    ) -> Result<Value, Error> {
        self.list(owner, name, "issues", query).await
    }

    /// `GET ai/repos/{owner}/{name}/prs`: the server's array of PR summaries.
    pub async fn prs(&self, owner: &str, name: &str, query: &AiListQuery) -> Result<Value, Error> {
        self.list(owner, name, "prs", query).await
    }

    async fn list(
        &self,
        owner: &str,
        name: &str,
        resource: &str,
        query: &AiListQuery,
    ) -> Result<Value, Error> {
        let path = with_query(
            format!("ai/repos/{}/{}/{resource}", seg(owner), seg(name)),
            &[
                ("state", query.state.clone()),
                ("limit", query.limit.map(|v| v.to_string())),
            ],
        );
        self.client.send(crate::Method::GET, &path, None).await
    }
}

fn seg(value: &str) -> String {
    crate::path_segment(value)
}

/// Append the present parameters as a query string.
fn with_query(path: String, params: &[(&str, Option<String>)]) -> String {
    let pairs: Vec<String> = params
        .iter()
        .filter_map(|(key, value)| value.as_ref().map(|v| format!("{key}={}", urlencode(v))))
        .collect();
    if pairs.is_empty() {
        path
    } else {
        format!("{path}?{}", pairs.join("&"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_omits_unset_parameters() {
        assert_eq!(with_query("a".into(), &[("x", None)]), "a");
        assert_eq!(
            with_query(
                "a".into(),
                &[
                    ("q", Some("fn main".into())),
                    ("limit", None),
                    ("offset", Some("20".into()))
                ]
            ),
            "a?q=fn+main&offset=20"
        );
    }
}
