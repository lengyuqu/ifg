//! The API surface this client uses, for the contract drift test
//! (design doc §6.1): `tests/contract.rs` checks every entry against the
//! OpenAPI snapshot in `fixtures/openapi/`. Endpoints missing from the
//! snapshot must be registered in `fixtures/openapi/uncovered.txt`.
//!
//! Paths are relative to `/api/v1/` and use the server's `{param}` template
//! spelling so they match the OpenAPI document verbatim.

/// (method, path) pairs used by `ifg-client` and the CLI/e2e flows.
pub const USED_PATHS: &[(&str, &str)] = &[
    // Wiki: revision-by-id is intentionally not used because the server must
    // verify that the requested revision belongs to the authorized page.
    ("GET", "/repos/{owner}/{name}/wiki"),
    ("POST", "/repos/{owner}/{name}/wiki"),
    ("GET", "/repos/{owner}/{name}/wiki/{title}"),
    ("PATCH", "/repos/{owner}/{name}/wiki/{title}"),
    ("DELETE", "/repos/{owner}/{name}/wiki/{title}"),
    ("GET", "/repos/{owner}/{name}/wiki/{title}/history"),
    ("GET", "/collaboration/capabilities"),
    ("GET", "/repos/{owner}/{name}/collaboration/capabilities"),
    // Collaboration P0
    ("GET", "/repos/{owner}/{name}/collaboration/sessions"),
    ("POST", "/repos/{owner}/{name}/collaboration/sessions"),
    (
        "POST",
        "/repos/{owner}/{name}/collaboration/sessions/{id}/heartbeat",
    ),
    (
        "POST",
        "/repos/{owner}/{name}/collaboration/sessions/{id}/end",
    ),
    ("GET", "/repos/{owner}/{name}/collaboration/tasks"),
    ("POST", "/repos/{owner}/{name}/collaboration/tasks"),
    ("GET", "/repos/{owner}/{name}/collaboration/tasks/{id}"),
    ("PATCH", "/repos/{owner}/{name}/collaboration/tasks/{id}"),
    (
        "GET",
        "/repos/{owner}/{name}/collaboration/tasks/{id}/messages",
    ),
    (
        "POST",
        "/repos/{owner}/{name}/collaboration/tasks/{id}/messages",
    ),
    ("PUT", "/repos/{owner}/{name}/collaboration/tasks/{id}/read"),
    (
        "GET",
        "/repos/{owner}/{name}/collaboration/tasks/{id}/handoffs",
    ),
    (
        "POST",
        "/repos/{owner}/{name}/collaboration/tasks/{id}/handoffs",
    ),
    (
        "POST",
        "/repos/{owner}/{name}/collaboration/handoffs/{id}/accept",
    ),
    (
        "POST",
        "/repos/{owner}/{name}/collaboration/handoffs/{id}/cancel",
    ),
    ("GET", "/repos/{owner}/{name}/collaboration/inbox"),
    ("GET", "/repos/{owner}/{name}/collaboration/inbox/user"),
    (
        "GET",
        "/repos/{owner}/{name}/collaboration/tasks/{id}/questions",
    ),
    (
        "POST",
        "/repos/{owner}/{name}/collaboration/tasks/{id}/questions",
    ),
    ("GET", "/repos/{owner}/{name}/collaboration/questions/{id}"),
    (
        "POST",
        "/repos/{owner}/{name}/collaboration/questions/{id}/answer",
    ),
    (
        "POST",
        "/repos/{owner}/{name}/collaboration/questions/{id}/cancel",
    ),
    (
        "GET",
        "/repos/{owner}/{name}/collaboration/tasks/{id}/takeovers",
    ),
    (
        "POST",
        "/repos/{owner}/{name}/collaboration/tasks/{id}/takeovers",
    ),
    (
        "POST",
        "/repos/{owner}/{name}/collaboration/tasks/{id}/reassign",
    ),
    // users
    ("GET", "/users/me"),
    ("GET", "/users/{username}"),
    ("GET", "/users/tokens"),
    ("POST", "/users/register"),
    ("POST", "/users/login"),
    ("POST", "/users/tokens"),
    // repos
    ("POST", "/repos"),
    ("GET", "/repos/explore"),
    ("GET", "/repos/{owner}"),
    ("GET", "/repos/{owner}/{name}"),
    ("PATCH", "/repos/{owner}/{name}"),
    ("DELETE", "/repos/{owner}/{name}"),
    ("GET", "/repos/{owner}/{name}/collaborators"),
    ("POST", "/repos/{owner}/{name}/collaborators"),
    ("PATCH", "/repos/{owner}/{name}/collaborators/{id}"),
    ("DELETE", "/repos/{owner}/{name}/collaborators/{user_id}"),
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
    ("PATCH", "/repos/{owner}/{name}/pulls/{number}"),
    ("GET", "/repos/{owner}/{name}/pulls/{number}/reviewers"),
    ("POST", "/repos/{owner}/{name}/pulls/{number}/reviewers"),
    (
        "DELETE",
        "/repos/{owner}/{name}/pulls/{number}/reviewers/{username}",
    ),
    ("GET", "/repos/{owner}/{name}/pulls/{number}/diff"),
    ("POST", "/repos/{owner}/{name}/pulls/{number}/merge"),
    ("PUT", "/repos/{owner}/{name}/pulls/{number}/auto-merge"),
    ("GET", "/repos/{owner}/{name}/pulls/{number}/reviews"),
    ("POST", "/repos/{owner}/{name}/pulls/{number}/reviews"),
    // ci (M3)
    ("GET", "/repos/{owner}/{name}/pipelines"),
    ("POST", "/repos/{owner}/{name}/pipelines"),
    ("GET", "/repos/{owner}/{name}/pipelines/{id}"),
    ("POST", "/repos/{owner}/{name}/pipelines/{id}/retry"),
    ("POST", "/repos/{owner}/{name}/pipelines/{id}/cancel"),
    ("GET", "/repos/{owner}/{name}/pipelines/{id}/jobs/{job_id}"),
    (
        "POST",
        "/repos/{owner}/{name}/pipelines/{id}/jobs/{job_id}/play",
    ),
    (
        "POST",
        "/repos/{owner}/{name}/pipelines/{pipeline_id}/jobs/{job_id}/rerun",
    ),
    (
        "POST",
        "/repos/{owner}/{name}/pipelines/{pipeline_id}/jobs/{job_id}/approve",
    ),
    // releases (M3)
    ("GET", "/repos/{owner}/{name}/releases"),
    ("POST", "/repos/{owner}/{name}/releases"),
    ("GET", "/repos/{owner}/{name}/releases/tags/{tag}"),
    ("PATCH", "/repos/{owner}/{name}/releases/{id}"),
    ("DELETE", "/repos/{owner}/{name}/releases/{id}"),
    ("GET", "/repos/{owner}/{name}/releases/{id}"),
    ("GET", "/repos/{owner}/{name}/releases/{release_id}/assets"),
    ("POST", "/repos/{owner}/{name}/releases/{release_id}/assets"),
    (
        "GET",
        "/repos/{owner}/{name}/releases/assets/{asset_id}/download",
    ),
    // labels
    ("GET", "/repos/{owner}/{name}/labels"),
    ("POST", "/repos/{owner}/{name}/labels"),
    ("PATCH", "/repos/{owner}/{name}/labels/{id}"),
    ("DELETE", "/repos/{owner}/{name}/labels/{id}"),
    // milestones
    ("GET", "/repos/{owner}/{name}/milestones"),
    ("POST", "/repos/{owner}/{name}/milestones"),
    ("PATCH", "/repos/{owner}/{name}/milestones/{id}"),
    ("DELETE", "/repos/{owner}/{name}/milestones/{id}"),
    // notifications (read is POST, not PATCH — see the snapshot)
    ("GET", "/notifications"),
    ("GET", "/notifications/unread-count"),
    ("POST", "/notifications/{id}/read"),
    ("POST", "/notifications/mark-all-read"),
    ("DELETE", "/notifications/{id}"),
    // orgs
    ("GET", "/orgs"),
    ("POST", "/orgs"),
    ("GET", "/orgs/{name}"),
    ("GET", "/orgs/{name}/members"),
    ("POST", "/orgs/{name}/members"),
    ("DELETE", "/orgs/{name}/members/{user_id}"),
    // search
    ("GET", "/search"),
];
