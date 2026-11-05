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

pub const EXIT_GENERAL: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_AUTH: i32 = 4;
pub const EXIT_NOT_FOUND: i32 = 8;

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
            })
        };
        let unauthorized = api(401, "UNAUTHORIZED");
        let forbidden = api(403, "FORBIDDEN");
        let missing = api(404, "NOT_FOUND");
        assert_eq!(unauthorized.exit_code(), EXIT_AUTH);
        assert_eq!(forbidden.exit_code(), EXIT_AUTH);
        assert_eq!(missing.exit_code(), EXIT_NOT_FOUND);
    }
}
