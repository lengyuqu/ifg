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
mod input;
mod keyring;
mod output;
mod template;
mod time;
mod version;
mod web;

use clap::{ArgMatches, CommandFactory, FromArgMatches};
use cli::{Cli, Command, RepoCommand};

fn main() -> std::process::ExitCode {
    // Parse through `ArgMatches` so the leaf command path stays available to
    // `validate_output`; this is what `Cli::parse()` does internally, except
    // that a refusal goes through `parse_failure` instead of exiting here.
    let matches = match Cli::command().try_get_matches() {
        Ok(matches) => matches,
        Err(error) => return parse_failure(error),
    };
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(error) => return parse_failure(error.format(&mut Cli::command())),
    };
    let json = cli.json;
    input::set(input::resolve(
        cli.no_input,
        std::env::var(input::ENV_VAR).ok().as_deref(),
    ));
    output::init(cli.no_color);
    init_tracing(cli.verbose);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build the tokio runtime");
    let mut code = runtime.block_on(run(cli, &matches));
    if let Err(error) = output::finish() {
        eprintln!("ifg: could not write stdout: {error}");
        if json {
            let error = error::AppError::Io(format!("could not write stdout: {error}"));
            eprintln!("{}", error.json_report());
        }
        if code == 0 {
            code = error::EXIT_GENERAL;
        }
    }
    let code = u8::try_from(code.max(0)).unwrap_or(1);
    std::process::ExitCode::from(code)
}

/// A command line clap refused (or `--help`/`--version`). Output and exit
/// code match `Cli::parse()`; under `--json` a failure also ends with the
/// error object on stderr.
fn parse_failure(error: clap::Error) -> std::process::ExitCode {
    let _ = error.print();
    if error.use_stderr() && json_requested() {
        let report = error::AppError::Usage(error.to_string().trim_end().to_owned());
        eprintln!("{}", report.json_report());
    }
    let code = u8::try_from(error.exit_code().max(0)).unwrap_or(error::EXIT_USAGE as u8);
    std::process::ExitCode::from(code)
}

/// Whether `--json` appears before any `--`. Clap has not produced a `Cli`
/// yet, so this is a textual check; it only decides whether an extra line is
/// printed, never which exit code is used.
fn json_requested() -> bool {
    std::env::args_os()
        .skip(1)
        .take_while(|arg| arg != "--")
        .any(|arg| arg == "--json")
}

/// Print a failure for the user. The `ifg:` line and the optional `hint:`
/// line are the human output; under `--json` the error object follows as the
/// last stderr line.
fn report_error(error: &error::AppError, json: bool) {
    eprintln!("ifg: {error}");
    if let Some(hint) = error.hint() {
        eprintln!("hint: {hint}");
    }
    if json {
        eprintln!("{}", error.json_report());
    }
}

/// The repository target a `repo` subcommand received as a positional
/// argument. Only full URLs matter here: `OWNER/NAME` does not name an
/// instance, so it must keep using the ambient host.
fn positional_target(command: &Command) -> Option<String> {
    let Command::Repo { command } = command else {
        return None;
    };
    match command {
        RepoCommand::View { target, .. }
        | RepoCommand::Edit { target, .. }
        | RepoCommand::Delete { target, .. } => target.clone(),
        RepoCommand::Clone { target, .. } => Some(target.clone()),
        _ => None,
    }
}

async fn run(mut cli: Cli, matches: &ArgMatches) -> i32 {
    let json = cli.json;
    // The schema describes this binary only: no config, token or network.
    if let Command::Schema { command_path } = &cli.command {
        return match cmd::schema::run(command_path) {
            Ok(()) => 0,
            Err(error) => {
                report_error(&error, json);
                error.exit_code()
            }
        };
    }
    if let Err(error) = cmd::validate_output(&cli, matches) {
        report_error(&error, json);
        return error.exit_code();
    }
    // An idempotency key is refused here, before any request, when the command
    // cannot use it.
    match cmd::validate_idempotency(&cli, matches) {
        Ok(key) => cli.idempotency = key,
        Err(error) => {
            report_error(&error, json);
            return error.exit_code();
        }
    }
    // A full URL in a positional target (`repo view <URL>`) must decide the
    // instance before `bootstrap` picks a host, so hand it to the resolver.
    let target = positional_target(&cli.command);
    cli.target_hint = target;
    if let Command::Completions { shell } = &cli.command {
        // Render into a buffer first: piping the script into `head` closes
        // stdout early, and clap_complete panics on write errors.
        let mut command = Cli::command();
        let mut buffer = Vec::new();
        clap_complete::generate(*shell, &mut command, "ifg", &mut buffer);
        output::print_bytes(&buffer);
        return 0;
    }
    let result = match &cli.command {
        Command::Auth { command } => cmd::auth::run(command, &cli).await,
        Command::Api { args } => cmd::api::run(args, &cli).await,
        Command::Repo { command } => cmd::repo::run(command, &cli).await,
        Command::Issue { command } => cmd::issue::run(command, &cli).await,
        Command::Pr { command } => cmd::pr::run(command, &cli).await,
        Command::Ci { command } => cmd::ci::run(command, &cli).await,
        Command::Events { command } => cmd::events::run(command, &cli).await,
        Command::Collab {
            command,
            session_file,
        } => cmd::collab::run(command, session_file.as_deref(), &cli).await,
        Command::Release { command } => cmd::release::run(command, &cli).await,
        Command::Label { command } => cmd::label::run(command, &cli).await,
        Command::Milestone { command } => cmd::milestone::run(command, &cli).await,
        Command::Wiki { command } => cmd::wiki::run(command, &cli).await,
        Command::Status => cmd::status::run(&cli).await,
        Command::Search {
            query,
            search_type,
            limit,
        } => cmd::search::run(&cli, query, search_type, *limit).await,
        Command::Context { command } => cmd::ai_context::run(command, &cli).await,
        Command::Notifications { command } => cmd::notifications::run(command, &cli).await,
        Command::Org { command } => cmd::org::run(command, &cli).await,
        Command::Version => cmd::version::run(&cli).await,
        Command::Capabilities { has } => cmd::capabilities::run(has.as_deref(), &cli).await,
        Command::Mcp { command } => cmd::mcp::run(command, &cli),
        Command::Doctor => cmd::doctor::run(&cli).await,
        Command::Completions { .. } => unreachable!("handled before dispatch"),
        Command::Schema { .. } => unreachable!("handled before dispatch"),
    };
    match result {
        Ok(()) => 0,
        Err(err) => {
            report_error(&err, json);
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
