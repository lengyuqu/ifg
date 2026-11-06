//! Wiki commands; page updates replace content and have no server-side CAS.
use serde_json::Value;
use std::io::Read;

use crate::cli::{Cli, WikiCommand};
use crate::context::resolve_repo;
use crate::error::AppError;
use crate::output;

fn content(body: &Option<String>, file: &Option<String>) -> Result<String, AppError> {
    if let Some(body) = body {
        return Ok(body.clone());
    }
    let file = file
        .as_deref()
        .ok_or_else(|| AppError::Usage("provide --body or --file".into()))?;
    if file == "-" {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|e| AppError::Io(format!("cannot read stdin: {e}")))?;
        Ok(text)
    } else {
        std::fs::read_to_string(file).map_err(|e| AppError::Io(format!("cannot read {file}: {e}")))
    }
}

fn structured(cli: &Cli, value: &Value) {
    if let Some(template) = &cli.template {
        if let Some(items) = value.as_array() {
            output::print_template(items, template);
        } else {
            output::print_template(std::slice::from_ref(value), template);
        }
    } else {
        output::print_json(value);
    }
}

pub async fn run(command: &WikiCommand, cli: &Cli) -> Result<(), AppError> {
    let ctx = super::bootstrap(cli)?;
    let repo = resolve_repo(cli.repo.as_deref(), &ctx.host)?;
    let wiki = ctx.client.wiki(&repo.owner, &repo.name);
    match command {
        WikiCommand::List => {
            let value = wiki.list().await?;
            if cli.json || cli.template.is_some() {
                structured(cli, &value);
            } else {
                let items = value
                    .as_array()
                    .ok_or_else(|| AppError::Other("invalid Wiki page list".into()))?;
                let rows: Vec<Vec<String>> = items
                    .iter()
                    .map(|item| {
                        vec![
                            item["title"].as_str().unwrap_or("").into(),
                            item["updated_at"].as_str().unwrap_or("").into(),
                        ]
                    })
                    .collect();
                if rows.is_empty() {
                    println!("No Wiki pages in {repo}.");
                } else {
                    output::print_table(&["TITLE", "UPDATED"], &rows);
                }
            }
        }
        WikiCommand::View { title } => {
            let value = wiki.view(title).await?;
            if cli.json || cli.template.is_some() {
                structured(cli, &value);
            } else {
                let text = value["content"]
                    .as_str()
                    .ok_or_else(|| AppError::Other("invalid Wiki page content".into()))?;
                output::print_bytes(text.as_bytes());
            }
        }
        WikiCommand::History { title } => {
            let value = wiki.history(title).await?;
            structured(cli, &value);
        }
        WikiCommand::Create {
            title,
            body,
            file,
            message,
        } => {
            let value = wiki
                .create(title, &content(body, file)?, message.as_deref())
                .await?;
            if cli.json || cli.template.is_some() {
                structured(cli, &value);
            } else {
                println!("Created Wiki page {title} in {repo}.");
            }
        }
        WikiCommand::Edit {
            title,
            body,
            file,
            message,
        } => {
            let value = wiki
                .update(title, &content(body, file)?, message.as_deref())
                .await?;
            if cli.json || cli.template.is_some() {
                structured(cli, &value);
            } else {
                println!("Updated Wiki page {title} in {repo}.");
            }
        }
        WikiCommand::Delete { title, yes } => {
            if !yes && !super::confirm(&format!("Delete Wiki page '{title}' from {repo}?"))? {
                println!("Aborted.");
                return Ok(());
            }
            wiki.delete(title).await?;
            if cli.json || cli.template.is_some() {
                structured(cli, &serde_json::json!({"title":title,"deleted":true}));
            } else {
                println!("Deleted Wiki page {title} from {repo}.");
            }
        }
    }
    Ok(())
}
