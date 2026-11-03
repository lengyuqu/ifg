//! CLI-level error type and exit codes (design doc §5.5).
//!
//! `0` success, `1` general error, `2` usage error (clap's default),
//! `4` authentication needed (401/403 or no local token), `8` not found.
//! `8` is an `ifg` extension; `gh` has no equivalent.

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
            AppError::Config(_) | AppError::Other(_) => EXIT_GENERAL,
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
