//! Subcommand implementations (M1: `auth`, `api`; M2: `repo`, `issue`, `pr`).

pub mod api;
pub mod auth;
pub mod ci;
pub mod issue;
pub mod pr;
pub mod release;
pub mod repo;

use ifg_client::Client;

use crate::cli::Cli;
use crate::config::Config;
use crate::context::{self, ResolvedHost};
use crate::error::AppError;

/// Everything a command needs after flag parsing: the config, the resolved
/// host and a client bound to it.
pub(crate) struct CommandContext {
    pub config: Config,
    pub host: ResolvedHost,
    pub client: Client,
}

pub(crate) fn user_agent() -> String {
    concat!("ifg/", env!("CARGO_PKG_VERSION")).to_string()
}

/// Load the config, resolve the host (§5.2) and build the client.
pub(crate) fn bootstrap(cli: &Cli) -> Result<CommandContext, AppError> {
    let config = crate::config::load()?;
    let host = context::resolve_host(cli.host.as_deref(), cli.repo.as_deref(), &config)?;
    let mut builder = Client::builder(&host.url).user_agent(user_agent());
    if let Some(token) = &host.token {
        builder = builder.token(token.clone());
    }
    let client = builder.build()?;
    Ok(CommandContext {
        config,
        host,
        client,
    })
}
