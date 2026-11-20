//! `ifg notifications …` — the current user's inbox.

use serde_json::Value;

use crate::cli::{Cli, NotificationsCommand, PrefsCommand};
use crate::cmd::bootstrap;
use crate::context::resolve_repo;
use crate::error::AppError;
use crate::output;

const DEFAULT_LIMIT: u64 = 30;

pub async fn run(command: &Option<NotificationsCommand>, cli: &Cli) -> Result<(), AppError> {
    let ctx = bootstrap(cli)?;
    let notifications = ctx.client.notifications();
    match command {
        None | Some(NotificationsCommand::List { .. }) => {
            let (unread, limit) = match command {
                Some(NotificationsCommand::List { unread, limit }) => (*unread, *limit),
                _ => (false, None),
            };
            let limit = limit.unwrap_or(DEFAULT_LIMIT).max(1);
            // The endpoint is paginated; asking for 150 used to return the
            // first page of 100 only. `print_template` also used to treat the
            // envelope as an array and print nothing.
            if cli.json || cli.template.is_some() {
                let items: Vec<Value> = notifications
                    .list_raw(unread)
                    .limit(limit)
                    .collect()
                    .await?;
                match &cli.template {
                    Some(template) => output::print_template(&items, template),
                    None => output::print_json(&Value::Array(items)),
                }
                return Ok(());
            }
            let items = notifications.list(unread).limit(limit).collect().await?;
            if items.is_empty() {
                output::line!("No notifications.");
                return Ok(());
            }
            let rows: Vec<Vec<String>> = items
                .iter()
                .map(|item| {
                    vec![
                        item.id.to_string(),
                        if item.is_read {
                            "read".into()
                        } else {
                            "unread".into()
                        },
                        item.event_type.clone(),
                        item.title.clone(),
                        output::human_time(item.created_at.as_deref()),
                    ]
                })
                .collect();
            output::print_table(&["ID", "STATE", "TYPE", "TITLE", "CREATED"], &rows);
            match notifications.unread_count().await {
                Ok(count) if count > 0 => eprintln!("({count} unread)"),
                Ok(_) => {}
                Err(error) => eprintln!("ifg: warning: could not read the unread count: {error}"),
            }
            Ok(())
        }
        Some(NotificationsCommand::Read { id, all }) => {
            if *all {
                let value = notifications.mark_all_read().await?;
                if cli.json || cli.template.is_some() {
                    super::show(cli, &value);
                } else {
                    output::line!("Marked every notification as read.");
                }
                return Ok(());
            }
            let Some(id) = id else {
                return Err(AppError::Usage("pass a notification id or --all".into()));
            };
            let value = notifications.mark_read(*id).await?;
            if cli.json || cli.template.is_some() {
                super::show(cli, &value);
            } else {
                output::line!("Marked notification {id} as read.");
            }
            Ok(())
        }
        Some(NotificationsCommand::Delete { id }) => {
            let value = notifications.delete(*id).await?;
            if cli.json || cli.template.is_some() {
                super::show(cli, &value);
            } else {
                output::line!("Deleted notification {id}.");
            }
            Ok(())
        }
        Some(NotificationsCommand::Prefs { command }) => {
            prefs(&ctx, &notifications, command, cli).await
        }
        Some(NotificationsCommand::Count) => {
            let count = notifications.unread_count().await?;
            if cli.json || cli.template.is_some() {
                super::show(cli, &serde_json::json!({"unread_count":count}));
            } else {
                output::line!("{count}");
            }
            Ok(())
        }
    }
}

/// `notifications prefs …`. A change is scoped to a repository only when `-R`
/// is given: the repository is never inferred from the git remote, so a
/// global change cannot turn into a repository one by accident.
async fn prefs(
    ctx: &super::CommandContext,
    notifications: &ifg_client::Notifications<'_>,
    command: &PrefsCommand,
    cli: &Cli,
) -> Result<(), AppError> {
    let structured = cli.json || cli.template.is_some();
    match command {
        PrefsCommand::List => {
            let value = notifications.preferences_raw().await?;
            if structured {
                super::show(cli, &value);
            } else {
                print_prefs(&value);
            }
        }
        PrefsCommand::Set {
            event,
            channel,
            on,
            off,
        } => {
            // clap guarantees exactly one of the two.
            let enabled = *on && !*off;
            // The server stores any event name it is given, so a typo would
            // silently create a setting nothing reads. Check the name against
            // the catalogue it lists first.
            let current = notifications.preferences_raw().await?;
            let known = event_types(&current);
            if !known.is_empty() && !known.iter().any(|name| name == event) {
                return Err(AppError::Usage(format!(
                    "unknown event type '{}'; known types: {}",
                    output::sanitize(event),
                    known
                        .iter()
                        .map(|name| output::sanitize(name))
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            let (repo_id, scope) = match cli.repo.as_deref() {
                Some(arg) => {
                    let repo = resolve_repo(Some(arg), &ctx.host)?;
                    // Only the id matters, so read the raw object rather than
                    // decode every field of the repository.
                    let found = ctx.client.repos().get_raw(&repo.owner, &repo.name).await?;
                    let id = found["id"].as_i64().ok_or_else(|| {
                        AppError::Other("the server did not return the repository id".into())
                    })?;
                    (Some(id), format!("repository {repo}"))
                }
                None => (None, "all repositories".to_string()),
            };
            let overrides = serde_json::json!([{
                "repo_id": repo_id,
                "event_type": event,
                "channel": channel,
                "enabled": enabled,
            }]);
            let value = notifications.set_preferences_raw(&overrides).await?;
            if structured {
                super::show(cli, &value);
            } else {
                output::line!(
                    "Turned {} {} for {}: {scope}.",
                    if enabled { "on" } else { "off" },
                    output::sanitize(event),
                    output::sanitize(channel),
                );
            }
        }
    }
    Ok(())
}

fn print_prefs(value: &Value) {
    let default = match value["default_enabled"].as_bool() {
        Some(true) => "on",
        Some(false) => "off",
        None => "unknown",
    };
    output::line!("Settings you have not changed are {default}.");
    let rows: Vec<Vec<String>> = value["preferences"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|row| {
            let cell = |key: &str| match &row[key] {
                Value::String(text) => text.clone(),
                Value::Null => "—".to_string(),
                other => other.to_string(),
            };
            vec![
                match row["repo_id"].as_i64() {
                    Some(id) => format!("repo #{id}"),
                    None => "all repositories".to_string(),
                },
                cell("event_type"),
                cell("channel"),
                match row["enabled"].as_bool() {
                    Some(true) => "on".into(),
                    Some(false) => "off".into(),
                    None => "—".into(),
                },
                match row["explicit"].as_bool() {
                    Some(true) => "set".into(),
                    _ => "default".into(),
                },
            ]
        })
        .collect();
    if rows.is_empty() {
        output::line!("No settings.");
        return;
    }
    output::print_table(&["SCOPE", "EVENT", "CHANNEL", "STATE", "SOURCE"], &rows);
}

/// The distinct event type names in a preferences answer, in server order.
fn event_types(value: &Value) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for row in value["preferences"].as_array().into_iter().flatten() {
        if let Some(name) = row["event_type"].as_str() {
            if !names.iter().any(|seen| seen == name) {
                names.push(name.to_string());
            }
        }
    }
    names
}
