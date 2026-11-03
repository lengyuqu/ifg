//! `ifg`: command-line client for IronForge.
//!
//! M1 surface: `auth login|status|logout|token` and `api`. The flow follows
//! design doc §5.1: parse → resolve context → run command → render → map
//! the error to an exit code. `repo`/`issue`/`pr`/`ci`/`release` land in
//! M2/M3.

mod cli;
mod cmd;
mod config;
mod context;
mod error;
mod git;
mod keyring;
mod output;
mod template;
mod time;
mod version;
mod web;

use clap::{CommandFactory, Parser};
use cli::{Cli, Command};

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    output::init(cli.no_color);
    init_tracing(cli.verbose);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build the tokio runtime");
    let code = runtime.block_on(run(cli));
    let code = u8::try_from(code.max(0)).unwrap_or(1);
    std::process::ExitCode::from(code)
}

async fn run(cli: Cli) -> i32 {
    if let Command::Completions { shell } = &cli.command {
        // Render into a buffer first: piping the script into `head` closes
        // stdout early, and clap_complete panics on write errors.
        let mut command = Cli::command();
        let mut buffer = Vec::new();
        clap_complete::generate(*shell, &mut command, "ifg", &mut buffer);
        use std::io::Write;
        let _ = std::io::stdout().write_all(&buffer);
        return 0;
    }
    let result = match &cli.command {
        Command::Auth { command } => cmd::auth::run(command, &cli).await,
        Command::Api { args } => cmd::api::run(args, &cli).await,
        Command::Repo { command } => cmd::repo::run(command, &cli).await,
        Command::Issue { command } => cmd::issue::run(command, &cli).await,
        Command::Pr { command } => cmd::pr::run(command, &cli).await,
        Command::Ci { command } => cmd::ci::run(command, &cli).await,
        Command::Release { command } => cmd::release::run(command, &cli).await,
        Command::Label { command } => cmd::label::run(command, &cli).await,
        Command::Milestone { command } => cmd::milestone::run(command, &cli).await,
        Command::Status => cmd::status::run(&cli).await,
        Command::Search {
            query,
            search_type,
            limit,
        } => cmd::search::run(&cli, query, search_type, *limit).await,
        Command::Notifications { command } => cmd::notifications::run(command, &cli).await,
        Command::Org { command } => cmd::org::run(command, &cli).await,
        Command::Version => cmd::version::run(&cli).await,
        Command::Completions { .. } => unreachable!("handled before dispatch"),
    };
    match result {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("ifg: {err}");
            err.exit_code()
        }
    }
}

/// `-v` logs every request, `-vv` additionally logs headers (§5.6). Logs go
/// to stderr so `--json` stdout stays parseable. `RUST_LOG` overrides.
fn init_tracing(verbosity: u8) {
    use tracing_subscriber::EnvFilter;
    let default_directive = match verbosity {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_directive));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
}
