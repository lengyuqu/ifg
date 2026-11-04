//! `ifg doctor` — check the whole setup in one command.
//!
//! Answers the questions that usually take a debugging session: which host
//! am I talking to, is the token accepted, does it carry the scopes the
//! commands need, is the server new enough, and can logs be streamed.
//!
//! Exit code is 1 when any check **fails** (warnings do not fail the run), so
//! scripts can gate on it.

use serde_json::{json, Value};

use crate::cli::Cli;
use crate::cmd::bootstrap;
use crate::context::{resolve_repo, ResolvedHost};
use crate::error::AppError;
use crate::output;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Warn,
    Fail,
}

impl Status {
    fn mark(self) -> &'static str {
        match self {
            Status::Ok => "ok  ",
            Status::Warn => "warn",
            Status::Fail => "FAIL",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "fail",
        }
    }
}

struct Check {
    name: &'static str,
    status: Status,
    detail: String,
    hint: Option<String>,
}

impl Check {
    fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: Status::Ok,
            detail: detail.into(),
            hint: None,
        }
    }

    fn warn(name: &'static str, detail: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            name,
            status: Status::Warn,
            detail: detail.into(),
            hint: Some(hint.into()),
        }
    }

    fn fail(name: &'static str, detail: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            name,
            status: Status::Fail,
            detail: detail.into(),
            hint: Some(hint.into()),
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "check": self.name,
            "status": self.status.as_str(),
            "detail": self.detail,
            "hint": self.hint,
        })
    }
}

pub async fn run(cli: &Cli) -> Result<(), AppError> {
    let mut checks = Vec::new();

    let ctx = match bootstrap(cli) {
        Ok(ctx) => Some(ctx),
        Err(error) => {
            checks.push(Check::fail(
                "config",
                error.to_string(),
                "run `ifg auth login --host <url>` (or export IRONFORGE_URL/IRONFORGE_PAT)",
            ));
            None
        }
    };

    if let Some(ctx) = &ctx {
        checks.push(host_check(cli, &ctx.host));
        checks.push(token_check(&ctx.host));

        match ctx.client.health().await {
            Ok(health) => checks.push(version_check(&health)),
            Err(error) => checks.push(Check::fail(
                "server",
                format!("{} — {error}", ctx.host.url),
                "is IronForge running and reachable from here?",
            )),
        }

        if ctx.host.token.is_some() {
            let (account, username) = account_check(ctx).await;
            checks.push(account);
            // Without a known account the authenticated scope probe would
            // just repeat the account failure above.
            if let Some(username) = username {
                checks.push(scope_check(ctx, &username).await);
            }
            checks.push(streaming_check(ctx).await);
        }
        checks.push(repo_check(cli, ctx).await);
        checks.push(git_helper_check());
    }

    let failed = checks
        .iter()
        .filter(|check| check.status == Status::Fail)
        .count();

    if cli.json || cli.template.is_some() {
        let items: Vec<Value> = checks.iter().map(Check::to_json).collect();
        match &cli.template {
            Some(template) => output::print_template(&items, template),
            None => output::print_json(&Value::Array(items)),
        }
    } else {
        println!(
            "ifg {} (min server {})",
            env!("CARGO_PKG_VERSION"),
            crate::version::MIN_SERVER_VERSION
        );
        let width = checks
            .iter()
            .map(|check| check.name.len())
            .max()
            .unwrap_or(0);
        for check in &checks {
            println!(
                "  {:<width$}  {}  {}",
                check.name,
                check.status.mark(),
                check.detail,
                width = width
            );
            if let Some(hint) = &check.hint {
                println!("  {:<width$}  {:<4}  → {hint}", "", "", width = width);
            }
        }
        println!();
        if failed == 0 {
            println!("all good.");
        } else {
            println!("{failed} check(s) failed.");
        }
    }

    if failed > 0 {
        return Err(AppError::Other(format!("{failed} doctor check(s) failed")));
    }
    Ok(())
}

fn host_check(cli: &Cli, host: &ResolvedHost) -> Check {
    let source = if cli.host.is_some() {
        "--host".to_string()
    } else if std::env::var("IRONFORGE_URL").is_ok_and(|value| !value.is_empty()) {
        "IRONFORGE_URL".to_string()
    } else if host.name.is_some() {
        "config".to_string()
    } else {
        "git remote".to_string()
    };
    Check::ok("host", format!("{} ({source})", host.url))
}

fn token_check(host: &ResolvedHost) -> Check {
    if host.token.is_some() {
        Check::ok("token", "present")
    } else {
        Check::fail(
            "token",
            "no token for this host",
            format!(
                "run `ifg auth login --host {}` or export IRONFORGE_PAT",
                host.url
            ),
        )
    }
}

fn version_check(health: &Value) -> Check {
    let version = health["version"].as_str().unwrap_or("unknown");
    let phase = health["phase"].as_u64();
    let detail = match phase {
        Some(phase) => format!("{version} (phase {phase})"),
        None => version.to_string(),
    };
    match crate::version::outdated(version) {
        None => Check::ok("server", detail),
        Some(message) => Check::fail("server", detail, message),
    }
}

async fn account_check(ctx: &crate::cmd::CommandContext) -> (Check, Option<String>) {
    match ctx.client.users().me().await {
        Ok(profile) => {
            let mut detail = profile.username.clone();
            if let Some(display) = &profile.display_name {
                detail.push_str(&format!(" ({display})"));
            }
            if profile.is_admin {
                detail.push_str(" [admin]");
            }
            (Check::ok("account", detail), Some(profile.username))
        }
        Err(error) => (
            Check::fail(
                "account",
                error.to_string(),
                "the token was rejected: create a new one in Settings → Tokens",
            ),
            None,
        ),
    }
}

/// `repo` scope is exercised by listing the caller's repositories: unlike
/// `/search` (which answers anonymously), `/repos/{owner}` demands the scope,
/// so a bad token or a `user`-only token actually fails here.
async fn scope_check(ctx: &crate::cmd::CommandContext, username: &str) -> Check {
    match ctx.client.repos().list(username).limit(1).collect().await {
        Ok(_) => Check::ok("scope", "repo scope works"),
        Err(error) => Check::fail(
            "scope",
            error.to_string(),
            "recreate the token with the `repo` scope",
        ),
    }
}

/// A PAT must authenticate the WebSocket handshake; a missing job then answers
/// 404. 401 means "the token works over HTTP but not here" — exactly what
/// `ci logs --follow` would report as "streaming unavailable".
async fn streaming_check(ctx: &crate::cmd::CommandContext) -> Check {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let base = ctx
        .host
        .url
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    let url = format!("{base}/api/v1/ws/job/0?since=0");
    let Ok(mut request) = url.as_str().into_client_request() else {
        return Check::warn(
            "streaming",
            "could not build the WebSocket URL",
            "report this: it is a client bug",
        );
    };
    if let Some(token) = &ctx.host.token {
        let header = format!("Bearer {}", token.expose());
        if let Ok(value) = tokio_tungstenite::tungstenite::http::HeaderValue::from_str(&header) {
            request.headers_mut().insert("authorization", value);
        }
    }
    match tokio_tungstenite::connect_async(request).await {
        Ok((mut socket, _)) => {
            let _ = socket.close(None).await;
            Check::ok("streaming", "WebSocket available")
        }
        Err(error) => {
            let text = error.to_string();
            if text.contains("404") {
                Check::ok("streaming", "WebSocket available")
            } else if text.contains("401") || text.contains("403") {
                Check::fail(
                    "streaming",
                    text,
                    "token rejected on the WebSocket route: check the reverse proxy forwards `Authorization`",
                )
            } else {
                Check::warn(
                    "streaming",
                    text,
                    "`ci logs --follow` will fall back to polling (all other commands work)",
                )
            }
        }
    }
}

async fn repo_check(cli: &Cli, ctx: &crate::cmd::CommandContext) -> Check {
    match resolve_repo(cli.repo.as_deref(), &ctx.host) {
        Ok(repo) => match ctx.client.repos().get(&repo.owner, &repo.name).await {
            Ok(_) => Check::ok("repo", format!("{repo}")),
            Err(error) => Check::fail(
                "repo",
                format!("{repo} — {error}"),
                "pass `-R OWNER/NAME`, or check the token's access to this repository",
            ),
        },
        Err(_) => Check::warn(
            "repo",
            "not inside a repository (and no -R)",
            "repository commands need `-R OWNER/NAME` or a checkout with an `origin` remote",
        ),
    }
}

/// The credential helper keeps `git clone`/`push` on the same token; a missing
/// helper is a warning, not an error (git may use its own store).
fn git_helper_check() -> Check {
    let output = std::process::Command::new("git")
        .args(["config", "--get-all", "credential.helper"])
        .output();
    let Ok(output) = output else {
        return Check::warn("git", "git not found in PATH", "install git");
    };
    let configured = String::from_utf8_lossy(&output.stdout).to_lowercase();
    if configured.contains("ifg") {
        Check::ok("git", "credential helper → ifg")
    } else {
        let helper = if cfg!(windows) {
            "!ifg.exe auth git-credential"
        } else {
            "ifg auth git-credential"
        };
        Check::warn(
            "git",
            "ifg is not the git credential helper",
            format!("git config --global --add credential.helper '{helper}' (optional)"),
        )
    }
}
