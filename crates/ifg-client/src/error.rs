//! Error model (design doc §4.2).
//!
//! IronForge error responses are nested:
//! `{"error": {"code": "NOT_FOUND", "message": "…", "request_id": null}}`.
//! Responses that are not valid JSON (an HTML 502 page from a reverse proxy,
//! for instance) fall back to a body snippet with `code = None`; the CLI
//! then reports the status code only.

use serde::Deserialize;

/// Everything that can go wrong in a request.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The server answered with a non-success status.
    #[error("API error (HTTP {status}): {message}")]
    Api {
        status: u16,
        code: Option<String>,
        message: String,
        request_id: Option<String>,
    },
    /// Connection, TLS or timeout failures.
    #[error("transport error: {0}")]
    Transport(#[from] reqwest::Error),
    /// The response body could not be decoded into the expected type.
    #[error("could not decode response ({body_snippet}): {source}")]
    Decode {
        body_snippet: String,
        #[source]
        source: serde_json::Error,
    },
    /// Invalid client configuration (malformed base URL, …).
    #[error("invalid configuration: {0}")]
    Config(String),
}

impl Error {
    /// HTTP status of an [`Error::Api`], if any.
    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Api { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// Machine-readable server error code (`error.code`), if any.
    pub fn code(&self) -> Option<&str> {
        match self {
            Error::Api { code, .. } => code.as_deref(),
            _ => None,
        }
    }

    /// The request needs authentication (401/403 or the matching codes).
    pub fn is_unauthorized(&self) -> bool {
        self.status() == Some(401) || self.code() == Some("UNAUTHORIZED")
    }

    /// The authenticated user lacks permission (403 or `FORBIDDEN`).
    pub fn is_forbidden(&self) -> bool {
        self.status() == Some(403) || self.code() == Some("FORBIDDEN")
    }

    /// The addressed resource does not exist (404 or `NOT_FOUND`).
    pub fn is_not_found(&self) -> bool {
        self.status() == Some(404) || self.code() == Some("NOT_FOUND")
    }
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Deserialize)]
struct ErrorBody {
    #[serde(default)]
    code: Option<String>,
    message: String,
    #[serde(default)]
    request_id: Option<String>,
}

/// Maximum number of body bytes quoted in snippets and `Decode` messages.
const SNIPPET_BYTES: usize = 200;

/// Build an [`Error::Api`] from a non-success response body, preferring the
/// nested `error` object and falling back to a raw snippet (§4.2).
pub(crate) fn api_error(status: u16, body: &[u8]) -> Error {
    match serde_json::from_slice::<ErrorEnvelope>(body) {
        Ok(envelope) => Error::Api {
            status,
            code: envelope.error.code,
            message: envelope.error.message,
            request_id: envelope.error.request_id,
        },
        Err(_) => Error::Api {
            status,
            code: None,
            message: snippet(body),
            request_id: None,
        },
    }
}

/// First [`SNIPPET_BYTES`] bytes of a body, lossy-decoded and trimmed.
pub(crate) fn snippet(body: &[u8]) -> String {
    let cut = &body[..body.len().min(SNIPPET_BYTES)];
    String::from_utf8_lossy(cut).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_error_body() {
        let body =
            br#"{"error":{"code":"NOT_FOUND","message":"repo not found","request_id":null}}"#;
        let err = api_error(404, body);
        let Error::Api {
            status,
            code,
            message,
            request_id,
        } = &err
        else {
            panic!("expected Api error, got {err:?}");
        };
        assert_eq!(*status, 404);
        assert_eq!(code.as_deref(), Some("NOT_FOUND"));
        assert_eq!(message, "repo not found");
        assert_eq!(*request_id, None);
        assert!(err.is_not_found());
    }

    #[test]
    fn non_json_body_falls_back_to_snippet() {
        let body = b"<html>502 Bad Gateway</html>";
        let err = api_error(502, body);
        let Error::Api { status, code, .. } = &err else {
            panic!("expected Api error, got {err:?}");
        };
        assert_eq!(*status, 502);
        assert_eq!(*code, None);
        let Error::Api { message, .. } = &err else {
            unreachable!()
        };
        assert!(message.contains("502 Bad Gateway"), "message: {message}");
    }

    #[test]
    fn snippet_truncates_to_200_bytes() {
        let body = vec![b'x'; 500];
        assert_eq!(snippet(&body).len(), 200);
    }
}
