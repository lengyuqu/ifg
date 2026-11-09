//! `ifg notifications …` — the current user's inbox.

use serde_json::Value;

use crate::cli::{Cli, NotificationsCommand};
use crate::cmd::bootstrap;
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
                println!("No notifications.");
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
                notifications.mark_all_read().await?;
                println!("Marked every notification as read.");
                return Ok(());
            }
            let Some(id) = id else {
                return Err(AppError::Usage("pass a notification id or --all".into()));
            };
            notifications.mark_read(*id).await?;
            println!("Marked notification {id} as read.");
            Ok(())
        }
        Some(NotificationsCommand::Delete { id }) => {
            notifications.delete(*id).await?;
            println!("Deleted notification {id}.");
            Ok(())
        }
        Some(NotificationsCommand::Count) => {
            println!("{}", notifications.unread_count().await?);
            Ok(())
        }
    }
}
