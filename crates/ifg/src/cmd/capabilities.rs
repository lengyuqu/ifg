//! `ifg capabilities`: what the connected server supports, read from its
//! collaboration contract. The server response is the only source; nothing
//! is cached, so every run makes exactly one request.

use serde_json::Value;

use crate::cli::Cli;
use crate::cmd::{bootstrap, show};
use crate::context::{resolve_repo, RepoContext};
use crate::error::AppError;
use crate::output;

pub async fn run(has: Option<&str>, cli: &Cli) -> Result<(), AppError> {
    let ctx = bootstrap(cli)?;
    // Only an explicit `-R` selects the repository endpoint. Sitting inside a
    // checkout does not, so the default stays instance-wide.
    let repo = match cli.repo.as_deref() {
        Some(target) => Some(resolve_repo(Some(target), &ctx.host)?),
        None => None,
    };
    let fetched = match &repo {
        Some(repo) => ctx.client.repo_capabilities(&repo.owner, &repo.name).await,
        None => ctx.client.capabilities().await,
    };
    let value = fetched.map_err(|error| {
        if error.is_not_found() {
            // An old server has no such route; that is not an empty list.
            AppError::NotFound(not_found_message(repo.as_ref()))
        } else {
            AppError::Client(error)
        }
    })?;

    // `--has` is a shell-facing check: it prints nothing on stdout, whatever
    // the output flags say.
    if let Some(name) = has {
        return if advertises(&value, name) {
            Ok(())
        } else {
            Err(AppError::Other(format!(
                "server does not advertise capability `{name}`"
            )))
        };
    }
    if cli.json || cli.template.is_some() {
        show(cli, &value);
    } else {
        print_human(&value, repo.as_ref());
    }
    Ok(())
}

/// Whether the capability list names `name` exactly (`capabilities[].name`).
fn advertises(value: &Value, name: &str) -> bool {
    value["capabilities"]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["name"].as_str() == Some(name)))
}

fn not_found_message(repo: Option<&RepoContext>) -> String {
    match repo {
        None => {
            "server does not expose capabilities (GET /collaboration/capabilities returned 404); \
                 it may predate this command"
                .to_string()
        }
        Some(repo) => format!(
            "server does not expose capabilities for {}/{} (returned 404): \
             the repository may not exist, or the server predates this command",
            repo.owner, repo.name
        ),
    }
}

fn print_human(value: &Value, repo: Option<&RepoContext>) {
    field("server", &text(&value["server_version"]));
    field("contract", &text(&value["contract_version"]));
    if let Some(commit) = value["build_commit"].as_str() {
        let dirty = match value["build_dirty"].as_bool() {
            Some(true) => " (modified source)",
            Some(false) => "",
            None => " (source state unknown)",
        };
        field("build", &format!("{}{dirty}", output::sanitize(commit)));
    }
    if repo.is_some() {
        field("repository", &text(&value["repo"]));
        let access = |key: &str| match value["caller"][key].as_bool() {
            Some(true) => "yes",
            Some(false) => "no",
            None => "unknown",
        };
        field(
            "access",
            &format!(
                "repo_collab_read {}, repo_write {}",
                access("repo_collab_read"),
                access("repo_write")
            ),
        );
    }
    match value["api_capabilities"].as_array() {
        Some(items) if items.is_empty() => field("api behaviors", "none"),
        Some(items) => {
            let names: Vec<String> = items
                .iter()
                .filter_map(Value::as_str)
                .map(output::sanitize)
                .collect();
            field("api behaviors", &names.join(", "));
        }
        None => field("api behaviors", "unknown"),
    }

    output::line!();
    let rows: Vec<Vec<String>> = value["capabilities"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    vec![
                        text(&item["name"]),
                        text(&item["version"]),
                        text(&item["status"]),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    output::print_table(&["NAME", "VERSION", "STATUS"], &rows);
}

fn field(label: &str, text: &str) {
    output::line!("{:<15}{}", format!("{label}:"), text);
}

/// A server string for display, or `unknown` when the field is absent.
fn text(value: &Value) -> String {
    value
        .as_str()
        .map(output::sanitize)
        .unwrap_or_else(|| "unknown".to_string())
}
