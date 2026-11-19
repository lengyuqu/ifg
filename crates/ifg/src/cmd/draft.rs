//! `ifg draft …`: agent draft branch namespaces of one repository.

use serde_json::Value;

use super::{bootstrap, show};
use crate::cli::{Cli, DraftCommand};
use crate::context::resolve_repo;
use crate::error::AppError;
use crate::output;

pub async fn run(command: &DraftCommand, cli: &Cli) -> Result<(), AppError> {
    let ctx = bootstrap(cli)?;
    let repo = resolve_repo(cli.repo.as_deref(), &ctx.host)?;
    let drafts = ctx.client.drafts(&repo.owner, &repo.name);
    let structured = cli.json || cli.template.is_some();
    match command {
        DraftCommand::List => {
            let value = drafts.list_raw().await?;
            if structured {
                show(cli, &value);
            } else {
                print_list(&repo.to_string(), &value);
            }
        }
        DraftCommand::Create { prefix, ttl, grace } => {
            let value = drafts.create_raw(prefix, *ttl, *grace).await?;
            if structured {
                show(cli, &value);
            } else {
                output::line!("Reserved {} in {repo}.", text(&value, "prefix"));
                print_deadlines(&value);
            }
        }
        DraftCommand::Extend { id, by } => {
            let value = drafts.extend_raw(*id, *by).await?;
            if structured {
                show(cli, &value);
            } else {
                output::line!("Extended {} in {repo}.", text(&value, "prefix"));
                print_deadlines(&value);
            }
        }
        DraftCommand::Close { id, yes } => {
            if !*yes
                && !super::confirm(&format!(
                    "Close draft namespace {id} of {repo}? Pushes stop now and its branches are deleted after the grace period."
                ))?
            {
                output::line!("Aborted.");
                return Ok(());
            }
            let value = drafts.close_raw(*id).await?;
            if structured {
                show(cli, &value);
            } else {
                output::line!("Closed {} in {repo}.", text(&value, "prefix"));
                output::line!(
                    "- branches are deleted at: {}",
                    text(&value, "delete_due_at")
                );
            }
        }
    }
    Ok(())
}

/// A field as sanitized text; `—` when absent or null.
fn text(value: &Value, key: &str) -> String {
    match value.get(key) {
        None | Some(Value::Null) => "—".to_string(),
        Some(Value::String(text)) => output::sanitize(text),
        Some(other) => output::sanitize(&other.to_string()),
    }
}

fn print_deadlines(value: &Value) {
    output::line!("- state: {}", text(value, "state"));
    output::line!("- pushes accepted until: {}", text(value, "expires_at"));
    output::line!("- branches deleted at: {}", text(value, "delete_due_at"));
}

fn print_list(repo: &str, value: &Value) {
    let namespaces = value.as_array().map(Vec::as_slice).unwrap_or_default();
    if namespaces.is_empty() {
        output::line!("No draft namespaces in {repo}.");
        return;
    }
    let rows: Vec<Vec<String>> = namespaces
        .iter()
        .map(|item| {
            vec![
                text(item, "id"),
                text(item, "prefix"),
                text(item, "state"),
                text(item, "expires_at"),
                text(item, "delete_due_at"),
                text(item, "extended_count"),
            ]
        })
        .collect();
    output::print_table(
        &["ID", "PREFIX", "STATE", "EXPIRES", "DELETED AT", "EXTENDED"],
        &rows,
    );
}
