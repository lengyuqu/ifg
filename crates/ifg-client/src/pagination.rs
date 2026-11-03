//! Pagination (design doc §4.3).
//!
//! Requests always ask for `per_page=100` (the server-side cap). Iteration
//! follows `pagination.has_next`; a page that claims `has_next` while
//! returning no data stops the iteration to avoid an infinite loop.

use std::marker::PhantomData;

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;

use crate::error::Error;
use crate::http::Client;

/// Items per page requested by the client (server caps at 100).
pub const PER_PAGE: u64 = 100;

/// Pagination metadata in list responses, mirroring the server field names.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PaginationMeta {
    pub page: u64,
    pub per_page: u64,
    pub total: u64,
    pub total_pages: u64,
    pub has_next: bool,
    pub has_prev: bool,
}

/// One page of a paginated list response.
#[derive(Debug, Clone, Deserialize)]
pub struct Page<T> {
    pub data: Vec<T>,
    pub pagination: PaginationMeta,
}

/// Iterates over the pages of a list endpoint.
///
/// M1 exposes `next_page`/`collect`; the `Stream`-based `take(n)` adapter
/// from the design lands with the M2 resource commands.
pub struct Paginator<T> {
    client: Client,
    method: reqwest::Method,
    path: String,
    body: Option<Value>,
    next_page: u64,
    exhausted: bool,
    fetched: u64,
    limit: Option<u64>,
    marker: PhantomData<fn() -> T>,
}

impl<T: DeserializeOwned> Paginator<T> {
    pub(crate) fn new_with_body(
        client: &Client,
        method: reqwest::Method,
        path: impl Into<String>,
        body: Option<Value>,
    ) -> Self {
        Self {
            client: client.clone(),
            method,
            path: path.into(),
            body,
            next_page: 1,
            exhausted: false,
            fetched: 0,
            limit: None,
            marker: PhantomData,
        }
    }

    /// Stop after `n` items; no further page is fetched once the limit is
    /// reached (the current page may overshoot, it is truncated then).
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Fetch the next page, or `None` once iteration is done. Endpoints that
    /// answer with a plain JSON array (not wrapped in `data`/`pagination`,
    /// e.g. `GET /users/tokens`) are treated as a single page.
    pub async fn next_page(&mut self) -> Result<Option<Page<T>>, Error> {
        if self.exhausted {
            return Ok(None);
        }
        if let Some(limit) = self.limit {
            if self.fetched >= limit {
                return Ok(None);
            }
        }
        let path = with_page_params(&self.path, self.next_page);
        let bytes = self
            .client
            .raw(self.method.clone(), &path, self.body.as_ref())
            .await?;
        let page = decode_page(&bytes, self.next_page)?;
        if page.data.is_empty() {
            if page.pagination.has_next {
                tracing::warn!(
                    path = %self.path,
                    page = page.pagination.page,
                    "server claims has_next with empty data; stopping to avoid an infinite loop"
                );
            }
            self.exhausted = true;
        } else {
            self.fetched += page.data.len() as u64;
            self.next_page += 1;
            self.exhausted = !page.pagination.has_next;
        }
        Ok(Some(page))
    }

    /// Collect items from all remaining pages, honoring [`Paginator::limit`].
    pub async fn collect(mut self) -> Result<Vec<T>, Error> {
        let mut items = Vec::new();
        while let Some(page) = self.next_page().await? {
            items.extend(page.data);
            if let Some(limit) = self.limit {
                if items.len() as u64 >= limit {
                    items.truncate(limit as usize);
                    break;
                }
            }
        }
        Ok(items)
    }
}

/// Append `page`/`per_page` to a path that may already carry a query string.
pub(crate) fn with_page_params(path: &str, page: u64) -> String {
    let separator = if path.contains('?') { '&' } else { '?' };
    format!("{path}{separator}page={page}&per_page={PER_PAGE}")
}

/// Decode a list response: a `Page<T>` when the endpoint paginates, a plain
/// `Vec<T>` as a single synthetic page otherwise.
fn decode_page<T: DeserializeOwned>(bytes: &[u8], page: u64) -> Result<Page<T>, Error> {
    match serde_json::from_slice::<Page<T>>(bytes) {
        Ok(page) => Ok(page),
        Err(page_err) => match serde_json::from_slice::<Vec<T>>(bytes) {
            Ok(data) => {
                let total = data.len() as u64;
                Ok(Page {
                    data,
                    pagination: PaginationMeta {
                        page,
                        per_page: PER_PAGE,
                        total,
                        total_pages: 1,
                        has_next: false,
                        has_prev: page > 1,
                    },
                })
            }
            Err(_) => Err(Error::Decode {
                body_snippet: crate::error::snippet(bytes),
                source: page_err,
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_params_after_existing_query() {
        assert_eq!(with_page_params("repos", 2), "repos?page=2&per_page=100");
        assert_eq!(
            with_page_params("issues?state=open", 1),
            "issues?state=open&page=1&per_page=100"
        );
    }

    #[test]
    fn deserializes_server_pagination_meta() {
        let json = r#"{
            "data": [{"id": 1}],
            "pagination": {"page": 1, "per_page": 100, "total": 1, "total_pages": 1,
                           "has_next": false, "has_prev": false}
        }"#;
        let page: Page<serde_json::Value> = serde_json::from_str(json).unwrap();
        assert_eq!(page.data.len(), 1);
        assert!(!page.pagination.has_next);
        assert_eq!(page.pagination.page, 1);
    }
}
