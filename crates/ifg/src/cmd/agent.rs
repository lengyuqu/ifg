//! `ifg agent …`: the agent accounts a person owns and their tokens.
//!
//! Output is the server's objects as sent (`--json`, `--template`) or compact
//! tables and one-line summaries with every server string sanitized. The one
//! secret in this module, a new token, goes to a private file and nowhere
//! else: it is never printed, logged or put in a JSON answer.

use std::path::Path;

use ifg_client::AgentTokenRequest;
use serde_json::Value;

use super::{bootstrap, show};
use crate::cli::{AgentCommand, AgentTokenCommand, Cli};
use crate::error::AppError;
use crate::output;

pub async fn run(command: &AgentCommand, cli: &Cli) -> Result<(), AppError> {
    let ctx = bootstrap(cli)?;
    let agents = ctx.client.agents();
    let structured = cli.json || cli.template.is_some();
    match command {
        AgentCommand::List => {
            let value = agents.list_raw().await?;
            if structured {
                show(cli, &value);
            } else {
                print_list(&value);
            }
        }
        AgentCommand::Create { name, description } => {
            let value = agents.create_raw(name, description.as_deref()).await?;
            if structured {
                show(cli, &value);
            } else {
                let username = text(&value, "username");
                output::line!("Created agent {username}.");
                output::line!(
                    "Issue it a token with: ifg agent token create {username} --name <name> --token-file <file>"
                );
            }
        }
        AgentCommand::View { username } => {
            let value = agents.get_raw(username).await?;
            if structured {
                show(cli, &value);
            } else {
                print_view(&value);
            }
        }
        AgentCommand::Edit {
            username,
            description,
            activate,
            deactivate,
        } => {
            let is_active = match (*activate, *deactivate) {
                (true, _) => Some(true),
                (_, true) => Some(false),
                _ => None,
            };
            let value = agents
                .update_raw(username, description.as_deref(), is_active)
                .await?;
            if structured {
                show(cli, &value);
            } else {
                output::line!("Updated agent {}.", text(&value, "username"));
                print_view(&value);
            }
        }
        AgentCommand::Token { command } => token(&agents, command, cli, structured).await?,
    }
    Ok(())
}

async fn token(
    agents: &ifg_client::Agents<'_>,
    command: &AgentTokenCommand,
    cli: &Cli,
    structured: bool,
) -> Result<(), AppError> {
    match command {
        AgentTokenCommand::List { username } => {
            let value = agents.tokens_raw(username).await?;
            if structured {
                show(cli, &value);
            } else {
                print_tokens(&value);
            }
        }
        AgentTokenCommand::Create {
            username,
            name,
            scopes,
            allow_repo,
            allow_ref,
            expires_at,
            token_file,
        } => {
            let request = AgentTokenRequest {
                name: name.clone(),
                scopes: scopes.clone(),
                repos: allow_repo.clone(),
                refs: allow_ref.clone(),
                expires_at: expires_at.clone(),
            };
            let mut value = create_token(agents, username, &request, Path::new(token_file)).await?;
            if structured {
                value["token_file"] = Value::String(token_file.clone());
                show(cli, &value);
            } else {
                output::line!(
                    "Created token {} ({}) for {}.",
                    text(&value, "id"),
                    text(&value, "name"),
                    output::sanitize(username)
                );
                output::line!("The secret is in {token_file} (mode 600); it is not shown again.");
            }
        }
        AgentTokenCommand::Revoke { username, id, yes } => {
            if !*yes
                && !super::confirm(&format!(
                    "Revoke token {id} of agent {}? Anything using it stops working.",
                    output::sanitize(username)
                ))?
            {
                output::line!("Aborted.");
                return Ok(());
            }
            let value = agents.delete_token_raw(username, *id).await?;
            if structured {
                show(cli, &value);
            } else {
                output::line!(
                    "Revoked token {id} of agent {}.",
                    output::sanitize(username)
                );
            }
        }
    }
    Ok(())
}

/// Issue a token and put its secret into `path`, a new file only the owner
/// can read. The file is created first so a bad path fails before a token
/// exists; it is removed again if the server refuses. The returned object has
/// the secret removed.
async fn create_token(
    agents: &ifg_client::Agents<'_>,
    username: &str,
    request: &AgentTokenRequest,
    path: &Path,
) -> Result<Value, AppError> {
    use std::io::Write;

    let mut file = crate::secure_file::create_private(path).map_err(|error| {
        AppError::Io(format!(
            "cannot create token file {} (it must not exist yet): {error}",
            path.display()
        ))
    })?;
    let mut value = match agents.create_token_raw(username, request).await {
        Ok(value) => value,
        Err(error) => {
            drop(file);
            let _ = std::fs::remove_file(path);
            return Err(error.into());
        }
    };
    let secret = value
        .as_object_mut()
        .and_then(|object| object.remove("token"))
        .and_then(|token| token.as_str().map(str::to_owned));
    let Some(secret) = secret else {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(AppError::Other(
            "the server did not return a token secret".into(),
        ));
    };
    let saved = file
        .write_all(secret.as_bytes())
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all());
    if let Err(error) = saved {
        let id = value["id"].as_i64().unwrap_or_default();
        return Err(AppError::Io(format!(
            "token {id} was created but its secret could not be saved to {}: {error}; \
             revoke it with 'ifg agent token revoke {username} {id}'",
            path.display()
        )));
    }
    Ok(value)
}

/// A field as sanitized text; `—` when absent or null.
fn text(value: &Value, key: &str) -> String {
    match value.get(key) {
        None | Some(Value::Null) => "—".to_string(),
        Some(Value::String(text)) => output::sanitize(text),
        Some(other) => output::sanitize(&other.to_string()),
    }
}

fn yes_no(value: &Value, key: &str) -> String {
    match value[key].as_bool() {
        Some(true) => "yes".into(),
        Some(false) => "no".into(),
        None => "—".into(),
    }
}

fn items(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or_default()
}

fn print_list(value: &Value) {
    let agents = items(value);
    if agents.is_empty() {
        output::line!("No agents. Create one with: ifg agent create <name>");
        return;
    }
    let rows: Vec<Vec<String>> = agents
        .iter()
        .map(|agent| {
            vec![
                text(agent, "username"),
                yes_no(agent, "is_active"),
                text(agent, "description"),
                output::human_time(agent["created_at"].as_str()),
            ]
        })
        .collect();
    output::print_table(&["USERNAME", "ACTIVE", "DESCRIPTION", "CREATED"], &rows);
}

fn print_view(agent: &Value) {
    output::line!("{}", text(agent, "username"));
    output::line!("- active: {}", yes_no(agent, "is_active"));
    output::line!("- description: {}", text(agent, "description"));
    output::line!("- owner id: {}", text(agent, "owner_user_id"));
    output::line!("- created: {}", text(agent, "created_at"));
}

fn print_tokens(value: &Value) {
    let tokens = items(value);
    if tokens.is_empty() {
        output::line!("No tokens.");
        return;
    }
    let list = |token: &Value, key: &str| match token[key].as_array() {
        Some(entries) => entries
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        None => "any".to_string(),
    };
    let rows: Vec<Vec<String>> = tokens
        .iter()
        .map(|token| {
            vec![
                text(token, "id"),
                text(token, "name"),
                text(token, "scopes"),
                list(token, "repos"),
                list(token, "refs"),
                output::human_time(token["expires_at"].as_str()),
                output::human_time(token["last_used_at"].as_str()),
            ]
        })
        .collect();
    output::print_table(
        &[
            "ID",
            "NAME",
            "SCOPES",
            "REPOS",
            "REFS",
            "EXPIRES",
            "LAST USED",
        ],
        &rows,
    );
}
