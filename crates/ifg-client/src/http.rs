//! HTTP layer: request building, bearer auth, retries and error-body parsing
//! (design doc §4.4).
//!
//! Retries happen only for idempotent methods (`GET`/`HEAD`), on connection
//! errors and on `429`/`502`/`503`/`504`, at most three times with
//! exponential backoff plus jitter. A `Retry-After` response header wins
//! over the computed backoff.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::{self, Error};
use crate::Secret;

/// Default request timeout (design doc §4.1).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Default maximum number of retries for idempotent requests (§4.4).
pub const DEFAULT_MAX_RETRIES: u32 = 3;
/// Default first backoff; doubles on every retry (0.5s, 1s, 2s).
pub const DEFAULT_BACKOFF: Duration = Duration::from_millis(500);
/// Upper bound for a single backoff, so a hostile `Retry-After` cannot stall
/// the CLI for minutes.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// A configured client for one IronForge instance. Cheap to clone: the
/// connection pool and token are shared.
#[derive(Clone)]
pub struct Client {
    base: url::Url,
    http: reqwest::Client,
    token: Option<Secret>,
    retry: RetryPolicy,
}

#[derive(Clone, Debug)]
struct RetryPolicy {
    max_retries: u32,
    initial_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
            initial_backoff: DEFAULT_BACKOFF,
        }
    }
}

impl RetryPolicy {
    /// Backoff before retry `attempt` (0-based), honoring `Retry-After` when
    /// the server sent one. Jitter is up to +50%.
    fn delay(&self, attempt: u32, retry_after: Option<&HeaderValue>) -> Duration {
        if let Some(secs) = retry_after
            .and_then(|v| v.to_str().ok())
            .and_then(parse_retry_after)
        {
            return secs.min(MAX_BACKOFF);
        }
        let shift = attempt.min(6);
        let base = self
            .initial_backoff
            .checked_mul(1u32 << shift)
            .unwrap_or(MAX_BACKOFF)
            .min(MAX_BACKOFF);
        base + jitter(base)
    }
}

/// `Retry-After` as delta seconds; HTTP dates are not supported.
fn parse_retry_after(value: &str) -> Option<Duration> {
    let secs: u64 = value.trim().parse().ok()?;
    Some(Duration::from_secs(secs))
}

/// Pseudo-random extra delay in `[0, base/2)`, seeded from the wall clock so
/// no `rand` dependency is needed.
fn jitter(base: Duration) -> Duration {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0)
        | 1;
    let mut x = nanos;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    let frac = (x % 1000) as f32 / 1000.0;
    base.mul_f32(frac / 2.0)
}

/// Builder for [`Client`] (design doc §4.1).
pub struct ClientBuilder {
    base_url: String,
    token: Option<Secret>,
    user_agent: Option<String>,
    timeout: Option<Duration>,
    retry: RetryPolicy,
}

impl ClientBuilder {
    /// Start configuring a client for `base_url` (scheme + host + optional
    /// sub-path, e.g. `https://forge.example.com`).
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            token: None,
            user_agent: None,
            timeout: None,
            retry: RetryPolicy::default(),
        }
    }

    /// Attach a personal access token; requests are anonymous without one.
    pub fn token(mut self, token: Secret) -> Self {
        self.token = Some(token);
        self
    }

    /// Override the `User-Agent` header (the library default is
    /// `ifg-client/<version>`).
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    /// Per-request timeout; defaults to [`DEFAULT_TIMEOUT`].
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Maximum number of retries for idempotent requests; `0` disables
    /// retries. Mainly a knob for tests and scripts.
    pub fn max_retries(mut self, max_retries: u32) -> Self {
        self.retry.max_retries = max_retries;
        self
    }

    /// First backoff between retries. Tests shrink it to keep suites fast.
    pub fn initial_backoff(mut self, initial_backoff: Duration) -> Self {
        self.retry.initial_backoff = initial_backoff;
        self
    }

    /// Validate the configuration and build the client.
    pub fn build(self) -> Result<Client, Error> {
        let base = normalize_base_url(&self.base_url)?;
        let user_agent = self
            .user_agent
            .unwrap_or_else(|| concat!("ifg-client/", env!("CARGO_PKG_VERSION")).to_string());
        let http = reqwest::Client::builder()
            .user_agent(user_agent)
            .timeout(self.timeout.unwrap_or(DEFAULT_TIMEOUT))
            .build()
            .map_err(Error::Transport)?;
        Ok(Client {
            base,
            http,
            token: self.token,
            retry: self.retry,
        })
    }
}

/// Parse and canonicalize a base URL: http(s) only, no query or fragment,
/// no trailing slash on the path.
fn normalize_base_url(base_url: &str) -> Result<url::Url, Error> {
    let url = url::Url::parse(base_url).map_err(|e| Error::Config(format!("invalid URL: {e}")))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(Error::Config(format!(
            "unsupported scheme '{}': use http or https",
            url.scheme()
        )));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(Error::Config(
            "base URL must not carry a query or fragment".into(),
        ));
    }
    let mut url = url;
    while url.path().len() > 1 && url.path().ends_with('/') {
        let trimmed = url.path().trim_end_matches('/').to_string();
        url.set_path(&trimmed);
    }
    Ok(url)
}

/// Append `path` (relative to `/api/v1/`) to a base URL. A leading `/` and a
/// redundant `api/v1/` prefix are tolerated; the path may already carry a
/// query string. Built by string concatenation and parsed once: `set_path`
/// would percent-encode a query string embedded in the path.
pub(crate) fn join_path(base: &url::Url, path: &str) -> url::Url {
    let mut path = path.trim_start_matches('/');
    if let Some(rest) = path.strip_prefix("api/v1/") {
        path = rest;
    }
    let mut url = base.as_str().to_string();
    if !url.ends_with('/') {
        url.push('/');
    }
    url.push_str("api/v1/");
    url.push_str(path);
    url::Url::parse(&url).expect("base URL and path form a valid URL")
}

impl Client {
    /// Start building a client for `base_url`.
    pub fn builder(base_url: impl Into<String>) -> ClientBuilder {
        ClientBuilder::new(base_url)
    }

    /// Iterate over the pages of a GET list endpoint (design doc §4.3).
    pub fn paginate<T: DeserializeOwned>(
        &self,
        path: impl Into<String>,
    ) -> crate::pagination::Paginator<T> {
        crate::pagination::Paginator::new_with_body(self, reqwest::Method::GET, path, None)
    }

    /// Same as [`Client::paginate`], for list endpoints that take a body.
    pub fn paginate_with_body<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: impl Into<String>,
        body: Value,
    ) -> crate::pagination::Paginator<T> {
        crate::pagination::Paginator::new_with_body(self, method, path, Some(body))
    }

    /// `GET /health` — outside `/api/v1/`. Returns the parsed JSON body
    /// (includes `version`; D10 uses it for server-version checks). A
    /// degraded `503` still parses, because "server is up" is the signal
    /// callers want.
    pub async fn health(&self) -> Result<serde_json::Value, Error> {
        // Build "/health" relative to the base path ("/" or "/forge"),
        // avoiding the double slash that falls into the SPA catch-all.
        let mut path = self.base.path().to_string();
        if !path.ends_with('/') {
            path.push('/');
        }
        path.push_str("health");
        let mut url = self.base.clone();
        url.set_path(&path);
        let response = self.http.get(url).send().await.map_err(Error::Transport)?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(Error::Transport)?;
        match serde_json::from_slice(&bytes) {
            Ok(value) => Ok(value),
            Err(source) => {
                if !status.is_success() {
                    Err(error::api_error(status.as_u16(), &bytes))
                } else {
                    Err(Error::Decode {
                        body_snippet: error::snippet(&bytes),
                        source,
                    })
                }
            }
        }
    }

    /// Issue a request and decode the JSON body into `T`.
    pub async fn send<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<T, Error> {
        let bytes = self.raw(method, path, body).await?;
        serde_json::from_slice(&bytes).map_err(|source| Error::Decode {
            body_snippet: error::snippet(&bytes),
            source,
        })
    }

    /// Issue a request and return the raw response body bytes. This is the
    /// `ifg api` pass-through (D5: the server's bytes, never re-serialized
    /// through a DTO).
    pub async fn raw(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Vec<u8>, Error> {
        let response = self.raw_response(method, path, body).await?;
        let bytes = response.bytes().await.map_err(Error::Transport)?;
        Ok(bytes.to_vec())
    }

    /// Issue a request and return the un-read `reqwest::Response` — for
    /// streaming bodies (release asset downloads). Error bodies are still
    /// parsed into [`Error::Api`].
    pub async fn raw_response(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<reqwest::Response, Error> {
        let url = join_path(&self.base, path);
        let idempotent = matches!(method, reqwest::Method::GET | reqwest::Method::HEAD);
        let mut attempt: u32 = 0;

        loop {
            let started = std::time::Instant::now();
            let mut request = self.http.request(method.clone(), url.clone());
            if let Some(token) = &self.token {
                request = request.bearer_auth(token.expose());
            }
            if let Some(body) = body {
                request = request.json(body);
            }

            let response = request.send().await;
            match response {
                Ok(response) => {
                    let status = response.status();
                    tracing::info!(
                        method = %method,
                        url = %url,
                        status = status.as_u16(),
                        elapsed = ?started.elapsed(),
                        "request"
                    );
                    if is_retryable(status) && idempotent && attempt < self.retry.max_retries {
                        let headers = response.headers().clone();
                        let delay = self.retry.delay(attempt, headers.get("Retry-After"));
                        tracing::info!(
                            status = status.as_u16(),
                            attempt,
                            ?delay,
                            "retrying idempotent request"
                        );
                        drop(response);
                        tokio::time::sleep(delay).await;
                        attempt += 1;
                        continue;
                    }
                    tracing::debug!(headers = ?masked(response.headers()), "response headers");
                    if !status.is_success() {
                        let bytes = response.bytes().await.map_err(Error::Transport)?;
                        return Err(error::api_error(status.as_u16(), &bytes));
                    }
                    return Ok(response);
                }
                Err(e) => {
                    if idempotent && e.is_connect() && attempt < self.retry.max_retries {
                        let delay = self.retry.delay(attempt, None);
                        tracing::info!(
                            error = %e,
                            attempt,
                            ?delay,
                            "retrying connection error"
                        );
                        tokio::time::sleep(delay).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(Error::Transport(e));
                }
            }
        }
    }

    /// Send a streaming request body (release asset upload, design doc
    /// §4.5: never buffer the whole file in memory). Stream bodies cannot be
    /// cloned, so there is no retry: a failure surfaces and the caller
    /// retries explicitly.
    pub(crate) async fn raw_with_stream_body(
        &self,
        method: reqwest::Method,
        path: &str,
        body: reqwest::Body,
        content_type: &str,
        extra_headers: &[(HeaderName, HeaderValue)],
    ) -> Result<reqwest::Response, Error> {
        let url = join_path(&self.base, path);
        let started = std::time::Instant::now();
        let mut request = self.http.request(method, url.clone());
        if let Some(token) = &self.token {
            request = request.bearer_auth(token.expose());
        }
        request = request
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body);
        for (name, value) in extra_headers {
            request = request.header(name, value);
        }
        let response = request.send().await.map_err(Error::Transport)?;
        let status = response.status();
        tracing::info!(
            method = "upload",
            url = %url,
            status = status.as_u16(),
            elapsed = ?started.elapsed(),
            "request"
        );
        if !status.is_success() {
            let bytes = response.bytes().await.map_err(Error::Transport)?;
            return Err(error::api_error(status.as_u16(), &bytes));
        }
        Ok(response)
    }
}

fn is_retryable(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 502 | 503 | 504)
}

/// Header map with the `Authorization` value reduced to `Bearer <prefix>…`,
/// safe to log (design doc §5.6).
fn masked(headers: &HeaderMap) -> HeaderMap {
    let mut copy = headers.clone();
    if let Some(value) = copy.get(AUTHORIZATION).cloned() {
        if let Ok(text) = value.to_str() {
            let redacted = match text.strip_prefix("Bearer ") {
                Some(token) => {
                    // ASCII dots: header values must stay to_str()-able.
                    let visible: String = token.chars().take(8).collect();
                    format!("Bearer {visible}...")
                }
                None => "[REDACTED]".to_string(),
            };
            if let Ok(v) = HeaderValue::from_str(&redacted) {
                copy.insert(AUTHORIZATION, v);
            }
        }
    }
    copy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_path_appends_api_v1_and_tolerates_prefixes() {
        let base = url::Url::parse("https://forge.example.com").unwrap();
        assert_eq!(
            join_path(&base, "users/me").as_str(),
            "https://forge.example.com/api/v1/users/me"
        );
        assert_eq!(
            join_path(&base, "/users/me").as_str(),
            "https://forge.example.com/api/v1/users/me"
        );
        assert_eq!(
            join_path(&base, "/api/v1/users/me").as_str(),
            "https://forge.example.com/api/v1/users/me"
        );
        assert_eq!(
            join_path(&base, "repos?page=2").as_str(),
            "https://forge.example.com/api/v1/repos?page=2"
        );
    }

    #[test]
    fn join_path_keeps_sub_path_deployments() {
        let base = url::Url::parse("https://example.com/forge").unwrap();
        assert_eq!(
            join_path(&base, "users/me").as_str(),
            "https://example.com/forge/api/v1/users/me"
        );
    }

    #[test]
    fn normalize_base_url_rejects_bad_input() {
        assert!(normalize_base_url("ftp://x").is_err());
        assert!(normalize_base_url("not a url").is_err());
        assert!(normalize_base_url("https://x/?q=1").is_err());
        let base = normalize_base_url("https://x/forge/").unwrap();
        assert_eq!(base.path(), "/forge");
    }

    #[test]
    fn masked_authorization_keeps_only_prefix() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer ifp_0123456789abcdef"),
        );
        let masked = masked(&headers);
        assert_eq!(
            masked.get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "Bearer ifp_0123..."
        );
    }

    #[test]
    fn retry_after_is_parsed_in_seconds() {
        assert_eq!(parse_retry_after("2"), Some(Duration::from_secs(2)));
        assert_eq!(parse_retry_after("0"), Some(Duration::from_secs(0)));
        assert_eq!(parse_retry_after("soon"), None);
    }

    #[test]
    fn backoff_grows_and_stays_capped() {
        let policy = RetryPolicy {
            max_retries: 3,
            initial_backoff: Duration::from_millis(500),
        };
        let first = policy.delay(0, None);
        assert!(first >= Duration::from_millis(500));
        assert!(first <= Duration::from_millis(750));
        let second = policy.delay(1, None);
        assert!(second >= Duration::from_secs(1));
        let capped = policy.delay(9, None);
        assert!(capped <= MAX_BACKOFF + MAX_BACKOFF / 2);
        assert_eq!(
            policy.delay(0, Some(&HeaderValue::from_static("7"))),
            Duration::from_secs(7)
        );
    }
}
