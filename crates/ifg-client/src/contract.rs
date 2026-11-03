//! The API surface this client uses, for the contract drift test
//! (design doc §6.1): `tests/contract.rs` checks every entry against the
//! OpenAPI snapshot in `fixtures/openapi/`. Endpoints missing from the
//! snapshot must be registered in `fixtures/openapi/uncovered.txt`.
//!
//! Paths are relative to `/api/v1/` and use the server's `{param}` template
//! spelling so they match the OpenAPI document verbatim.

/// (method, path) pairs used by `ifg-client` and the CLI/e2e flows.
pub const USED_PATHS: &[(&str, &str)] = &[
    // users
    ("GET", "/users/me"),
    ("GET", "/users/tokens"),
    ("POST", "/users/register"),
    ("POST", "/users/login"),
    ("POST", "/users/tokens"),
    // repos
    ("POST", "/repos"),
    ("GET", "/repos/explore"),
    ("GET", "/repos/{owner}"),
    ("GET", "/repos/{owner}/{name}"),
    ("DELETE", "/repos/{owner}/{name}"),
    // issues
    ("GET", "/repos/{owner}/{name}/issues"),
    ("POST", "/repos/{owner}/{name}/issues"),
    ("GET", "/repos/{owner}/{name}/issues/{number}"),
    ("PATCH", "/repos/{owner}/{name}/issues/{number}"),
    ("GET", "/repos/{owner}/{name}/issues/{number}/comments"),
    ("POST", "/repos/{owner}/{name}/issues/{number}/comments"),
    // pulls
    ("GET", "/repos/{owner}/{name}/pulls"),
    ("POST", "/repos/{owner}/{name}/pulls"),
    ("GET", "/repos/{owner}/{name}/pulls/{number}"),
    ("GET", "/repos/{owner}/{name}/pulls/{number}/diff"),
    ("POST", "/repos/{owner}/{name}/pulls/{number}/merge"),
    ("PUT", "/repos/{owner}/{name}/pulls/{number}/auto-merge"),
    ("GET", "/repos/{owner}/{name}/pulls/{number}/reviews"),
    ("POST", "/repos/{owner}/{name}/pulls/{number}/reviews"),
];
