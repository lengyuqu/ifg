//! CLI-level error type and exit codes (design doc §5.5).
//!
//! `0` success, `1` general error, `2` usage error (clap's default),
//! `4` authentication needed (401/403 or no local token), `8` not found.
//! `8` is an `ifg` extension; `gh` has no equivalent.
//!
//! An empty server response is not the only "not found": a tag or name the
//! server answers 200 for but never lists is the same outcome to the user,
//! so commands raise `NotFound` themselves instead of folding it into `Other`.

use ifg_client::Error as ClientError;
use serde_json::{json, Value};

pub const EXIT_GENERAL: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_AUTH: i32 = 4;
pub const EXIT_NOT_FOUND: i32 = 8;

/// `kind` of the machine-readable error object printed under `--json`
/// (see docs/json-compatibility.md). The vocabulary is fixed for 1.x.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Usage,
    Auth,
    NotFound,
    Conflict,
    Validation,
    RateLimited,
    Server,
    Network,
    Other,
}

impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Usage => "usage",
            ErrorKind::Auth => "auth",
            ErrorKind::NotFound => "not_found",
            ErrorKind::Conflict => "conflict",
            ErrorKind::Validation => "validation",
            ErrorKind::RateLimited => "rate_limited",
            ErrorKind::Server => "server",
            ErrorKind::Network => "network",
            ErrorKind::Other => "other",
        }
    }

    /// Only network failures, 429 and 5xx are worth retrying. The CLI does
    /// not retry POST itself, so this tells the caller what to try again.
    pub fn retryable(self) -> bool {
        matches!(
            self,
            ErrorKind::Network | ErrorKind::RateLimited | ErrorKind::Server
        )
    }
}

/// Classify a client error. Auth and not-found follow the same predicates as
/// the exit code, so the kind and the exit code never disagree.
fn client_kind(err: &ClientError) -> ErrorKind {
    if matches!(err, ClientError::Transport(_)) {
        return ErrorKind::Network;
    }
    if err.is_unauthorized() || err.is_forbidden() {
        return ErrorKind::Auth;
    }
    if err.is_not_found() {
        return ErrorKind::NotFound;
    }
    match err.status() {
        Some(409) => ErrorKind::Conflict,
        Some(422) => ErrorKind::Validation,
        Some(429) => ErrorKind::RateLimited,
        Some(500..=599) => ErrorKind::Server,
        _ => ErrorKind::Other,
    }
}

/// Error type for the whole CLI.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Client(#[from] ClientError),
    #[error("{0}")]
    Config(#[from] crate::config::ConfigError),
    /// Bad command usage; exit code 2.
    #[error("{0}")]
    Usage(String),
    /// Authentication is needed but there was no server round trip
    /// (e.g. no local token); exit code 4.
    #[error("{0}")]
    Auth(String),
    /// The requested object does not exist; exit code 8. Raised by commands
    /// for a name the server answered 200 for but never listed (the 404 path
    /// is already mapped in the `Client` arm).
    #[error("{0}")]
    NotFound(String),
    /// A filesystem operation failed; exit code 1. Kept separate from
    /// `Other` so I/O sites are greppable and get a uniform prefix.
    #[error("io error: {0}")]
    Io(String),
    /// Everything else; exit code 1.
    #[error("{0}")]
    Other(String),
}

impl AppError {
    /// Classification for the `--json` error object.
    pub fn kind(&self) -> ErrorKind {
        match self {
            AppError::Usage(_) => ErrorKind::Usage,
            AppError::Auth(_) => ErrorKind::Auth,
            AppError::NotFound(_) => ErrorKind::NotFound,
            AppError::Client(err) => client_kind(err),
            AppError::Config(_) | AppError::Io(_) | AppError::Other(_) => ErrorKind::Other,
        }
    }

    /// Server `error.reason` of a failed API call, if the server sent one.
    pub fn reason(&self) -> Option<&str> {
        match self {
            AppError::Client(err) => err.reason(),
            _ => None,
        }
    }

    /// Server `error.hint` of a failed API call, if the server sent one.
    pub fn hint(&self) -> Option<&str> {
        match self {
            AppError::Client(err) => err.hint(),
            _ => None,
        }
    }

    /// The `{"error": {...}}` object printed as the last stderr line under
    /// `--json`. `message` is the same text as the human `ifg:` line.
    pub fn json_report(&self) -> Value {
        let kind = self.kind();
        let http_status = match self {
            AppError::Client(err) => err.status(),
            _ => None,
        };
        json!({
            "error": {
                "kind": kind.as_str(),
                "http_status": http_status,
                "reason": self.reason(),
                "hint": self.hint(),
                "message": self.to_string(),
                "retryable": kind.retryable(),
            }
        })
    }

    /// Process exit code for this error (design doc §5.5).
    pub fn exit_code(&self) -> i32 {
        match self {
            AppError::Usage(_) => EXIT_USAGE,
            AppError::Auth(_) => EXIT_AUTH,
            AppError::NotFound(_) => EXIT_NOT_FOUND,
            AppError::Io(_) | AppError::Config(_) | AppError::Other(_) => EXIT_GENERAL,
            AppError::Client(err) => {
                if err.is_unauthorized() || err.is_forbidden() {
                    EXIT_AUTH
                } else if err.is_not_found() {
                    EXIT_NOT_FOUND
                } else {
                    EXIT_GENERAL
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Constructor, expected exit code and expected message per variant
    /// (design doc §5.5). `Client` and `Config` need live sources, so they are
    /// covered by `client_errors_keep_their_codes`.
    fn cases() -> Vec<(AppError, i32, &'static str)> {
        vec![
            (AppError::Usage("bad flag".into()), EXIT_USAGE, "bad flag"),
            (AppError::Auth("no token".into()), EXIT_AUTH, "no token"),
            (
                AppError::NotFound("release 'v1' not found".into()),
                EXIT_NOT_FOUND,
                "release 'v1' not found",
            ),
            (
                AppError::Io("could not read notes.md".into()),
                EXIT_GENERAL,
                "io error: could not read notes.md",
            ),
            (AppError::Other("boom".into()), EXIT_GENERAL, "boom"),
        ]
    }

    #[test]
    fn exit_codes_are_mapped_per_variant() {
        for (error, expected, _) in cases() {
            assert_eq!(error.exit_code(), expected, "{error:?}");
        }
    }

    #[test]
    fn messages_reach_the_user() {
        for (error, _, expected) in cases() {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn client_errors_keep_their_codes() {
        let api = |status: u16, code: &str| {
            AppError::Client(ClientError::Api {
                status,
                code: Some(code.into()),
                message: "no".into(),
                request_id: None,
                reason: None,
                hint: None,
                details: None,
            })
        };
        let unauthorized = api(401, "UNAUTHORIZED");
        let forbidden = api(403, "FORBIDDEN");
        let missing = api(404, "NOT_FOUND");
        assert_eq!(unauthorized.exit_code(), EXIT_AUTH);
        assert_eq!(forbidden.exit_code(), EXIT_AUTH);
        assert_eq!(missing.exit_code(), EXIT_NOT_FOUND);
    }

    fn api_status(status: u16) -> AppError {
        AppError::Client(ClientError::Api {
            status,
            code: None,
            message: "m".into(),
            request_id: None,
            reason: None,
            hint: None,
            details: None,
        })
    }

    #[test]
    fn statuses_classify_into_kinds() {
        let cases = [
            (401, ErrorKind::Auth),
            (403, ErrorKind::Auth),
            (404, ErrorKind::NotFound),
            (409, ErrorKind::Conflict),
            (422, ErrorKind::Validation),
            (429, ErrorKind::RateLimited),
            (500, ErrorKind::Server),
            (502, ErrorKind::Server),
            (503, ErrorKind::Server),
            (400, ErrorKind::Other),
            (418, ErrorKind::Other),
        ];
        for (status, expected) in cases {
            assert_eq!(api_status(status).kind(), expected, "HTTP {status}");
        }
    }

    #[test]
    fn local_failures_classify_into_kinds() {
        assert_eq!(AppError::Usage("x".into()).kind(), ErrorKind::Usage);
        assert_eq!(AppError::Auth("x".into()).kind(), ErrorKind::Auth);
        assert_eq!(AppError::NotFound("x".into()).kind(), ErrorKind::NotFound);
        assert_eq!(AppError::Io("x".into()).kind(), ErrorKind::Other);
        assert_eq!(AppError::Other("x".into()).kind(), ErrorKind::Other);
    }

    #[test]
    fn only_network_429_and_5xx_are_retryable() {
        let retryable: Vec<ErrorKind> = [
            ErrorKind::Usage,
            ErrorKind::Auth,
            ErrorKind::NotFound,
            ErrorKind::Conflict,
            ErrorKind::Validation,
            ErrorKind::RateLimited,
            ErrorKind::Server,
            ErrorKind::Network,
            ErrorKind::Other,
        ]
        .into_iter()
        .filter(|kind| kind.retryable())
        .collect();
        assert_eq!(
            retryable,
            vec![
                ErrorKind::RateLimited,
                ErrorKind::Server,
                ErrorKind::Network
            ]
        );
    }

    #[test]
    fn json_report_carries_reason_hint_and_status() {
        let err = AppError::Client(ClientError::Api {
            status: 409,
            code: Some("CONFLICT".into()),
            message: "branch exists".into(),
            request_id: None,
            reason: Some("branch_exists".into()),
            hint: Some("choose another name".into()),
            details: None,
        });
        assert_eq!(
            err.json_report(),
            json!({"error": {
                "kind": "conflict",
                "http_status": 409,
                "reason": "branch_exists",
                "hint": "choose another name",
                "message": "API error (HTTP 409): branch exists",
                "retryable": false,
            }})
        );
    }

    #[test]
    fn json_report_nulls_fields_without_http_context() {
        let report = AppError::Usage("bad flag".into()).json_report();
        assert_eq!(
            report,
            json!({"error": {
                "kind": "usage",
                "http_status": null,
                "reason": null,
                "hint": null,
                "message": "bad flag",
                "retryable": false,
            }})
        );
    }
}
