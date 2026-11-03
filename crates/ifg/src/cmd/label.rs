//! `ifg label …` — repository labels. Names are the user-facing identity:
//! edit/delete resolve the name to the id first.

use crate::cli::{Cli, LabelCommand};
use crate::cmd::bootstrap;
use crate::context::{resolve_repo, RepoContext};
use crate::error::AppError;
use crate::output;

pub async fn run(command: &LabelCommand, cli: &Cli) -> Result<(), AppError> {
    let ctx = bootstrap(cli)?;
    let repo = resolve_repo(cli.repo.as_deref(), &ctx.host)?;
    match command {
        LabelCommand::List => list(&ctx, cli, &repo).await,
        LabelCommand::Create {
            name,
            color,
            description,
        } => create(&ctx, cli, &repo, name, color.clone(), description.clone()).await,
        LabelCommand::Edit {
            name,
            new_name,
            color,
            description,
        } => {
            edit(
                &ctx,
                cli,
                &repo,
                name,
                new_name.clone(),
                color.clone(),
                description.clone(),
            )
            .await
        }
        LabelCommand::Delete { name, yes } => delete(&ctx, &repo, name, *yes).await,
    }
}

async fn list(ctx: &super::CommandContext, cli: &Cli, repo: &RepoContext) -> Result<(), AppError> {
    let labels = ctx.client.labels(&repo.owner, &repo.name);
    if cli.json || cli.template.is_some() {
        let items = labels.list_raw().collect().await?;
        match &cli.template {
            Some(template) => output::print_template(&items, template),
            None => output::print_json(&serde_json::Value::Array(items)),
        }
        return Ok(());
    }
    let rows: Vec<Vec<String>> = labels
        .list()
        .collect()
        .await?
        .iter()
        .map(|label| {
            vec![
                label.name.clone(),
                label.color.clone().unwrap_or_default(),
                label.description.clone().unwrap_or_default(),
            ]
        })
        .collect();
    if rows.is_empty() {
        println!("No labels in {repo}.");
        return Ok(());
    }
    output::print_table(&["NAME", "COLOR", "DESCRIPTION"], &rows);
    Ok(())
}

async fn create(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    name: &str,
    color: Option<String>,
    description: Option<String>,
) -> Result<(), AppError> {
    let options = ifg_client::LabelOptions {
        name: name.to_string(),
        color,
        description,
    };
    let labels = ctx.client.labels(&repo.owner, &repo.name);
    if cli.json || cli.template.is_some() {
        let value = labels.create_raw(&options).await?;
        match &cli.template {
            Some(template) => output::print_template(std::slice::from_ref(&value), template),
            None => output::print_json(&value),
        }
        return Ok(());
    }
    let label = labels.create(&options).await?;
    println!(
        "Created label {} ({})",
        label.name,
        label.color.unwrap_or_default()
    );
    Ok(())
}

async fn edit(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    name: &str,
    new_name: Option<String>,
    color: Option<String>,
    description: Option<String>,
) -> Result<(), AppError> {
    let labels = ctx.client.labels(&repo.owner, &repo.name);
    let existing = find(&labels, name).await?;
    let options = ifg_client::LabelOptions {
        name: new_name.unwrap_or_else(|| existing.name.clone()),
        color: color.or(existing.color.clone()),
        description: description.or(existing.description.clone()),
    };
    if cli.json || cli.template.is_some() {
        let value = labels.update_raw(existing.id, &options).await?;
        match &cli.template {
            Some(template) => output::print_template(std::slice::from_ref(&value), template),
            None => output::print_json(&value),
        }
        return Ok(());
    }
    let updated = labels.update(existing.id, &options).await?;
    println!(
        "Updated label {} ({})",
        updated.name,
        updated.color.unwrap_or_default()
    );
    Ok(())
}

async fn delete(
    ctx: &super::CommandContext,
    repo: &RepoContext,
    name: &str,
    yes: bool,
) -> Result<(), AppError> {
    let labels = ctx.client.labels(&repo.owner, &repo.name);
    let existing = find(&labels, name).await?;
    if !yes && !super::confirm(&format!("Delete label '{}' from {repo}?", existing.name))? {
        println!("Aborted.");
        return Ok(());
    }
    labels.delete(existing.id).await?;
    println!("Deleted label {}.", existing.name);
    Ok(())
}

/// Resolve a label name to the row (names are unique per repository).
async fn find(labels: &ifg_client::Labels<'_>, name: &str) -> Result<ifg_client::Label, AppError> {
    labels
        .list()
        .collect()
        .await?
        .into_iter()
        .find(|label| label.name == name)
        .ok_or_else(|| {
            AppError::Usage(format!(
                "no label '{name}' in this repository; run `ifg label list`"
            ))
        })
}
