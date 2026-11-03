//! Typed REST client for the IronForge API (`/api/v1/`).
//!
//! Layout (design doc §4): `http` holds the transport (auth header, retries,
//! error-body parsing), `pagination` the page iteration, `users` the first
//! resource group. DTOs are hand-written and only declare fields the client
//! uses; unknown fields are ignored (D4).

mod error;
mod http;
mod pagination;
mod secret;
mod users;

pub use error::Error;
pub use http::{Client, ClientBuilder, DEFAULT_BACKOFF, DEFAULT_MAX_RETRIES, DEFAULT_TIMEOUT};
pub use pagination::{Page, PaginationMeta, Paginator, PER_PAGE};
pub use reqwest::Method;
pub use secret::Secret;
pub use users::{has_scope, TokenInfo, UserProfile, Users};
