//! `ifg milestone …` — repository milestones, addressed by title.

use crate::cli::{Cli, MilestoneCommand};
use crate::cmd::bootstrap;
use crate::context::{resolve_repo, RepoContext};
use crate::error::AppError;
use crate::output;

pub async fn run(command: &MilestoneCommand, cli: &Cli) -> Result<(), AppError> {
    let ctx = bootstrap(cli)?;
    let repo = resolve_repo(cli.repo.as_deref(), &ctx.host)?;
    match command {
        MilestoneCommand::List => list(&ctx, cli, &repo).await,
        MilestoneCommand::Create {
            title,
            description,
            due,
        } => create(&ctx, cli, &repo, title, description.clone(), due.clone()).await,
        MilestoneCommand::Edit {
            title,
            new_title,
            description,
            state,
            due,
        } => {
            edit(
                &ctx,
                cli,
                &repo,
                title,
                new_title.clone(),
                description.clone(),
                state.clone(),
                due.clone(),
            )
            .await
        }
        MilestoneCommand::Delete { title, yes } => delete(&ctx, &repo, title, *yes).await,
    }
}

async fn list(ctx: &super::CommandContext, cli: &Cli, repo: &RepoContext) -> Result<(), AppError> {
    let milestones = ctx.client.milestones(&repo.owner, &repo.name);
    if cli.json || cli.template.is_some() {
        let items = milestones.list_raw().collect().await?;
        match &cli.template {
            Some(template) => output::print_template(&items, template),
            None => output::print_json(&serde_json::Value::Array(items)),
        }
        return Ok(());
    }
    let rows: Vec<Vec<String>> = milestones
        .list()
        .collect()
        .await?
        .iter()
        .map(|milestone| {
            vec![
                milestone.title.clone(),
                milestone.state.clone().unwrap_or_else(|| "open".into()),
                milestone
                    .due_date
                    .clone()
                    .map(|due| due.chars().take(10).collect())
                    .unwrap_or_else(|| "—".into()),
                milestone.description.clone().unwrap_or_default(),
            ]
        })
        .collect();
    if rows.is_empty() {
        println!("No milestones in {repo}.");
        return Ok(());
    }
    output::print_table(&["TITLE", "STATE", "DUE", "DESCRIPTION"], &rows);
    Ok(())
}

async fn create(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    title: &str,
    description: Option<String>,
    due: Option<String>,
) -> Result<(), AppError> {
    let options = ifg_client::MilestoneOptions {
        title: title.to_string(),
        description,
        state: None,
        due_date: due,
    };
    let milestones = ctx.client.milestones(&repo.owner, &repo.name);
    if cli.json || cli.template.is_some() {
        let value = milestones.create_raw(&options).await?;
        match &cli.template {
            Some(template) => output::print_template(std::slice::from_ref(&value), template),
            None => output::print_json(&value),
        }
        return Ok(());
    }
    let milestone = milestones.create(&options).await?;
    println!("Created milestone '{}'.", milestone.title);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn edit(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    title: &str,
    new_title: Option<String>,
    description: Option<String>,
    state: Option<String>,
    due: Option<String>,
) -> Result<(), AppError> {
    let milestones = ctx.client.milestones(&repo.owner, &repo.name);
    let existing = find(&milestones, title).await?;
    let options = ifg_client::MilestoneOptions {
        title: new_title.unwrap_or_else(|| existing.title.clone()),
        description: description.or(existing.description.clone()),
        state: state.or(existing.state.clone()),
        due_date: due.or(existing.due_date.clone()),
    };
    if cli.json || cli.template.is_some() {
        let value = milestones.update_raw(existing.id, &options).await?;
        match &cli.template {
            Some(template) => output::print_template(std::slice::from_ref(&value), template),
            None => output::print_json(&value),
        }
        return Ok(());
    }
    let updated = milestones.update(existing.id, &options).await?;
    println!("Updated milestone '{}'.", updated.title);
    Ok(())
}

async fn delete(
    ctx: &super::CommandContext,
    repo: &RepoContext,
    title: &str,
    yes: bool,
) -> Result<(), AppError> {
    let milestones = ctx.client.milestones(&repo.owner, &repo.name);
    let existing = find(&milestones, title).await?;
    if !yes
        && !super::confirm(&format!(
            "Delete milestone '{}' from {repo}?",
            existing.title
        ))?
    {
        println!("Aborted.");
        return Ok(());
    }
    milestones.delete(existing.id).await?;
    println!("Deleted milestone '{}'.", existing.title);
    Ok(())
}

async fn find(
    milestones: &ifg_client::Milestones<'_>,
    title: &str,
) -> Result<ifg_client::Milestone, AppError> {
    milestones
        .list()
        .collect()
        .await?
        .into_iter()
        .find(|milestone| milestone.title == title)
        .ok_or_else(|| {
            AppError::Usage(format!(
                "no milestone '{title}' in this repository; run `ifg milestone list`"
            ))
        })
}
