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
mod output;

use clap::Parser;
use cli::{Cli, Command};

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
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
    let result = match &cli.command {
        Command::Auth { command } => cmd::auth::run(command, &cli).await,
        Command::Api { args } => cmd::api::run(args, &cli).await,
        Command::Repo { command } => cmd::repo::run(command, &cli).await,
        Command::Issue { command } => cmd::issue::run(command, &cli).await,
        Command::Pr { command } => cmd::pr::run(command, &cli).await,
        Command::Ci { command } => cmd::ci::run(command, &cli).await,
        Command::Release { command } => cmd::release::run(command, &cli).await,
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
