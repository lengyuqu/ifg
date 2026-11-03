//! Command-line definition (design doc §5): global flags plus the `auth`
//! and `api` subcommands of M1.

use clap::{ArgAction, Parser, Subcommand};

/// Work with IronForge repositories, issues, pull requests, CI and releases
/// from the terminal.
#[derive(Debug, Parser)]
#[command(name = "ifg", version, arg_required_else_help = true)]
pub struct Cli {
    /// Host name from the config, or a base URL like https://forge.example.com
    #[arg(long, global = true, value_name = "NAME|URL")]
    pub host: Option<String>,

    /// Target repository as OWNER/NAME, or a full remote URL
    #[arg(short = 'R', long, global = true, value_name = "OWNER/NAME")]
    pub repo: Option<String>,

    /// Output JSON (server field names) instead of a table
    #[arg(long, global = true)]
    pub json: bool,

    /// Disable colored output (also honors NO_COLOR)
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Verbosity: -v logs each request, -vv also logs headers
    #[arg(short = 'v', long = "verbose", action = ArgAction::Count, global = true, default_value_t = 0)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage credentials for IronForge hosts
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Raw pass-through to the IronForge REST API (paths relative to /api/v1/)
    Api {
        #[command(flatten)]
        args: ApiArgs,
    },
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Log in with a personal access token (validates it against /users/me)
    Login {
        /// Read the token from stdin instead of an interactive prompt
        #[arg(long)]
        with_token: bool,
    },
    /// List configured hosts, token prefixes and usable scopes
    Status,
    /// Remove local credentials (the token stays valid server-side)
    Logout,
    /// Print the token of the resolved host (for scripts)
    Token,
}

#[derive(Debug, clap::Args)]
pub struct ApiArgs {
    /// Path relative to /api/v1/, e.g. "users/me"
    pub path: String,

    /// HTTP method (default GET)
    #[arg(short = 'X', long, default_value = "GET")]
    pub method: String,

    /// Add a field. With GET these become query parameters; with other
    /// methods they form a JSON body of strings
    #[arg(short = 'f', long = "field", value_name = "KEY=VALUE")]
    pub fields: Vec<String>,

    /// Read the JSON request body from a file ("-" for stdin)
    #[arg(long, value_name = "FILE")]
    pub input: Option<String>,

    /// Follow pagination and merge all `data` arrays into one JSON array
    #[arg(long)]
    pub paginate: bool,
}
