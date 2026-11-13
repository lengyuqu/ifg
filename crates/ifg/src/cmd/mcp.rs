//! `ifg mcp config` — print an MCP client configuration for IronForge's
//! stdio MCP server (`ironforge-mcp`).
//!
//! The snippet is the only thing written to stdout. It never contains the
//! token: `IRONFORGE_PAT` is the placeholder `${IRONFORGE_PAT}`, which the
//! user exports in the environment their MCP client starts from. The host is
//! resolved exactly like other commands; no request reaches the server and
//! no file is written.

use serde_json::{json, Map, Value};

use crate::cli::{Cli, McpClient, McpCommand};
use crate::cmd::bootstrap;
use crate::error::AppError;
use crate::output;

/// Literal placeholder written in place of the token.
const TOKEN_PLACEHOLDER: &str = "${IRONFORGE_PAT}";

pub fn run(command: &McpCommand, cli: &Cli) -> Result<(), AppError> {
    match command {
        McpCommand::Config {
            client,
            name,
            server_command,
        } => config(*client, name, server_command, cli),
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
