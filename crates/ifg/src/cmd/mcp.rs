//! `ifg mcp config` — print an MCP client configuration for IronForge's
//! stdio MCP server (`ironforge-mcp`).
//!
//! The snippet is the only thing written to stdout. It never contains the
//! token: `IRONFORGE_PAT` is the placeholder `${IRONFORGE_PAT}`, which the
//! user exports in the environment their MCP client starts from. The host is
//! resolved exactly like other commands; no request reaches the server and
//! no file is written.
//!
//! `ifg mcp doctor` checks whether that server can be used from this machine.
//! It reads the server's `capabilities.mcp_transport` and never runs the
//! server binary, and it prints the same check items as `ifg doctor`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::cli::{Cli, McpClient, McpCommand};
use crate::cmd::doctor::{
    account_check, failed_count, host_check, print_rows, print_structured, server_check,
    token_check, Check,
};
use crate::cmd::{bootstrap, CommandContext};
use crate::error::AppError;
use crate::output;

/// Literal placeholder written in place of the token.
const TOKEN_PLACEHOLDER: &str = "${IRONFORGE_PAT}";

/// Binary started when neither `--command` nor the server names one. Same
/// default as the `command` in `mcp config`.
const DEFAULT_SERVER_BINARY: &str = "ironforge-mcp";

/// Environment variable the MCP server reads its token from.
const TOKEN_ENV: &str = "IRONFORGE_PAT";

/// Hint for a server that does not declare MCP support.
const TOO_OLD_HINT: &str =
    "the server is too old to declare MCP support: upgrade IronForge, then rerun";

pub async fn run(command: &McpCommand, cli: &Cli) -> Result<(), AppError> {
    match command {
        McpCommand::Config {
            client,
            name,
            server_command,
        } => config(*client, name, server_command, cli),
        McpCommand::Doctor { server_command } => doctor(server_command.as_deref(), cli).await,
    }
}

fn config(client: McpClient, name: &str, server_command: &str, cli: &Cli) -> Result<(), AppError> {
    // Only the URL reaches the output; the token stays inside `ctx` unprinted.
    let ctx = bootstrap(cli)?;
    output::print_json(&render(client, name, server_command, &ctx.host.url));
    eprintln!(
        "ifg: the snippet refers to ${{IRONFORGE_PAT}} and does not contain the token. \
         Export it in the shell that starts your MCP client, e.g. \
         export IRONFORGE_PAT=\"$(ifg auth token)\"."
    );
    Ok(())
}

/// Build the snippet. Every client currently shares one shape; keep the
/// per-client match so formats can diverge without touching callers.
fn render(client: McpClient, name: &str, server_command: &str, url: &str) -> Value {
    let entry = json!({
        "command": server_command,
        "env": {
            "IRONFORGE_URL": url,
            "IRONFORGE_PAT": TOKEN_PLACEHOLDER,
        },
    });
    let mut servers = Map::new();
    servers.insert(name.to_string(), entry);
    match client {
        McpClient::ClaudeCode | McpClient::Cursor | McpClient::Generic => {
            json!({ "mcpServers": servers })
        }
    }
}

/// `ifg mcp doctor`: every check runs, in order. Only the capabilities
/// checks depend on the server; the binary and environment checks always run.
async fn doctor(server_command: Option<&str>, cli: &Cli) -> Result<(), AppError> {
    let mut checks = Vec::new();
    // The capabilities body, kept only when the server declares MCP support.
    let mut capabilities = None;

    match bootstrap(cli) {
        Ok(ctx) => {
            checks.push(host_check(cli, &ctx.host));
            checks.push(token_check(&ctx.host));
            checks.push(server_check(&ctx).await);
            if ctx.host.token.is_some() {
                checks.push(account_check(&ctx).await.0);
            }
            let (check, body) = capabilities_check(&ctx).await;
            checks.push(check);
            capabilities = body;
        }
        Err(error) => checks.push(Check::fail(
            "config",
            error.to_string(),
            "run `ifg auth login --host <url>` (or export IRONFORGE_URL/IRONFORGE_PAT)",
        )),
    }

    let declared_binary = capabilities
        .as_ref()
        .and_then(|body| body["mcp_transport"]["binary"].as_str())
        .filter(|name| !name.is_empty());
    if let Some(body) = &capabilities {
        checks.push(transport_check(&body["mcp_transport"]));
        checks.push(tools_check(body));
    }
    checks.push(binary_check(server_command, declared_binary));
    checks.push(token_env_check());

    let failed = failed_count(&checks);
    if cli.json || cli.template.is_some() {
        print_structured(cli, &checks);
    } else {
        output::line!("ifg {} — MCP setup check", env!("CARGO_PKG_VERSION"));
        print_rows(&checks);
    }

    if failed > 0 {
        return Err(AppError::Other(format!(
            "{failed} mcp doctor check(s) failed"
        )));
    }
    Ok(())
}

/// `GET /collaboration/capabilities`. The body is returned only when it
/// carries `mcp_transport`. A 404, or a body without it, means the server
/// cannot declare MCP support, so the transport and tool checks are skipped.
async fn capabilities_check(ctx: &CommandContext) -> (Check, Option<Value>) {
    match ctx.client.capabilities().await {
        Ok(body) if body["mcp_transport"].is_object() => {
            let contract = body["contract_version"].as_str().unwrap_or("unknown");
            let server = body["server_version"].as_str().unwrap_or("unknown");
            (
                Check::ok(
                    "capabilities",
                    format!("readable (contract {contract}, server {server})"),
                ),
                Some(body),
            )
        }
        Ok(_) => (
            Check::fail(
                "capabilities",
                "readable, but the response has no mcp_transport",
                TOO_OLD_HINT,
            ),
            None,
        ),
        Err(error) if error.is_not_found() => (
            Check::fail(
                "capabilities",
                "GET /collaboration/capabilities returned 404",
                TOO_OLD_HINT,
            ),
            None,
        ),
        Err(error) => (
            Check::fail(
                "capabilities",
                error.to_string(),
                "run `ifg doctor` to check the host and token",
            ),
            None,
        ),
    }
}

/// The server must declare stdio, the only transport `ironforge-mcp` has.
fn transport_check(transport: &Value) -> Check {
    let declared: Vec<&str> = transport["transports"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let listed = if declared.is_empty() {
        "none".to_string()
    } else {
        declared.join(", ")
    };
    if declared.contains(&"stdio") {
        Check::ok("transport", format!("stdio (server declares: {listed})"))
    } else {
        Check::warn(
            "transport",
            format!("stdio not declared (server declares: {listed})"),
            "ironforge-mcp runs over stdio, which this server does not declare; check the IronForge server version and deployment",
        )
    }
}

/// `tool_count` against the distinct names in `capabilities[].mcp_tools`.
/// The authoritative list is MCP `tools/list`, which REST cannot provide, so
/// a mismatch is a warning and never a failure.
fn tools_check(body: &Value) -> Check {
    let named: BTreeSet<&str> = body["capabilities"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|capability| capability["mcp_tools"].as_array())
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let named = named.len() as u64;
    let authoritative = "authoritative list: MCP tools/list";
    let hint = "compare with the tools your MCP client lists (from tools/list); this mismatch does not stop MCP use";
    match body["mcp_transport"]["tool_count"].as_u64() {
        Some(count) if count == named => Check::ok(
            "tools",
            format!("{count} (matches the {named} tools named in capabilities; {authoritative})"),
        ),
        Some(count) => Check::warn(
            "tools",
            format!(
                "tool_count {count}, but capabilities name {named} distinct tools; {authoritative}"
            ),
            hint,
        ),
        None => Check::warn(
            "tools",
            format!(
                "tool_count is missing; capabilities name {named} distinct tools; {authoritative}"
            ),
            hint,
        ),
    }
}

/// The binary is looked up, never executed. `--command` wins over the name
/// the server declares, which wins over the default.
fn binary_check(requested: Option<&str>, declared: Option<&str>) -> Check {
    let (name, source) = match (requested, declared) {
        (Some(name), _) => (name, "--command"),
        (None, Some(name)) => (name, "declared by the server"),
        (None, None) => (DEFAULT_SERVER_BINARY, "default"),
    };
    match locate_binary(name) {
        Lookup::Found(path) => {
            Check::ok("binary", format!("{name} → {} ({source})", path.display()))
        }
        Lookup::NotExecutable(path) => Check::fail(
            "binary",
            format!("{} exists but is not executable ({source})", path.display()),
            "make it executable (chmod +x), or install ironforge-mcp and pass its path with --command",
        ),
        Lookup::Missing => Check::fail(
            "binary",
            format!("{name} not found ({source})"),
            "install ironforge-mcp so it is on PATH, or pass its location with --command",
        ),
    }
}

/// `IRONFORGE_PAT` must be in the environment the MCP client starts the
/// server from; ifg's stored token does not reach it. The value is never shown.
fn token_env_check() -> Check {
    if std::env::var(TOKEN_ENV).is_ok_and(|value| !value.is_empty()) {
        Check::ok(TOKEN_ENV, "set (value not shown)")
    } else {
        Check::warn(
            TOKEN_ENV,
            "not set in this environment; the MCP server reads its token from IRONFORGE_PAT",
            set_token_hint(),
        )
    }
}

/// Shell command that puts the stored token into `IRONFORGE_PAT`, in the
/// syntax of the platform's default shell (PowerShell on Windows).
#[cfg(windows)]
fn set_token_hint() -> &'static str {
    "$env:IRONFORGE_PAT = (ifg auth token)"
}

#[cfg(not(windows))]
fn set_token_hint() -> &'static str {
    "export IRONFORGE_PAT=\"$(ifg auth token)\""
}

enum Lookup {
    Found(PathBuf),
    NotExecutable(PathBuf),
    Missing,
}

/// Find `name` the way a shell would, without running it. A name with a
/// directory part is a path; a bare name is searched in `PATH`.
fn locate_binary(name: &str) -> Lookup {
    let direct = Path::new(name);
    let candidates: Vec<PathBuf> = if direct.components().count() > 1 {
        executable_variants(direct.to_path_buf())
    } else {
        std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .flat_map(|dir| executable_variants(dir.join(name)))
            .collect()
    };
    let mut blocked = None;
    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        if is_executable(&candidate) {
            return Lookup::Found(candidate);
        }
        blocked.get_or_insert(candidate);
    }
    match blocked {
        Some(path) => Lookup::NotExecutable(path),
        None => Lookup::Missing,
    }
}

/// Windows runs `name.exe` for a bare `name`; elsewhere the name is the file.
fn executable_variants(path: PathBuf) -> Vec<PathBuf> {
    if cfg!(windows) && path.extension().is_none() {
        vec![path.with_extension("exe")]
    } else {
        vec![path]
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(_path: &Path) -> bool {
    true
}
