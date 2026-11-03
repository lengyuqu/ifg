//! `ifg issue …` (design doc §4.3).

use ifg_client::{CreateIssueOptions, IssueListFilter, UpdateIssueOptions};
use serde_json::Value;

use super::bootstrap;
use crate::cli::{Cli, IssueCommand};
use crate::context::{resolve_repo, RepoContext};
use crate::error::AppError;
use crate::output;

const DEFAULT_LIMIT: u64 = 30;

pub async fn run(command: &IssueCommand, cli: &Cli) -> Result<(), AppError> {
    let ctx = bootstrap(cli)?;
    let repo = resolve_repo(cli.repo.as_deref(), &ctx.host)?;
    match command {
        IssueCommand::List {
            state,
            label,
            assignee,
            limit,
        } => {
            list(
                &ctx,
                cli,
                &repo,
                state.clone(),
                label.clone(),
                assignee.clone(),
                *limit,
            )
            .await
        }
        IssueCommand::View {
            number,
            comments,
            web,
        } => {
            if *web {
                return crate::web::open(&crate::web::issue_url(&ctx.host.url, &repo, *number));
            }
            view(&ctx, cli, &repo, *number, *comments).await
        }
        IssueCommand::Create {
            title,
            body,
            body_file,
            label,
        } => {
            create(
                &ctx,
                cli,
                &repo,
                title.clone(),
                body.clone(),
                body_file.clone(),
                label.clone(),
            )
            .await
        }
        IssueCommand::Comment { number, body } => comment(&ctx, cli, &repo, *number, body).await,
        IssueCommand::Close { number } => set_state(&ctx, cli, &repo, *number, "closed").await,
        IssueCommand::Reopen { number } => set_state(&ctx, cli, &repo, *number, "open").await,
        IssueCommand::Edit {
            number,
            title,
            body,
        } => edit(&ctx, cli, &repo, *number, title.clone(), body.clone()).await,
    }
}

fn filter_of(state: &str, label: &Option<String>, assignee: &Option<String>) -> IssueListFilter {
    IssueListFilter {
        state: if state == "all" {
            None
        } else {
            Some(state.to_string())
        },
        labels: label.clone(),
        assignee: assignee.clone(),
    }
}

async fn list(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    state: String,
    label: Option<String>,
    assignee: Option<String>,
    limit: Option<u64>,
) -> Result<(), AppError> {
    let filter = filter_of(&state, &label, &assignee);
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    if cli.json {
        let items: Vec<Value> = ctx
            .client
            .issues(&repo.owner, &repo.name)
            .list_raw(&filter)
            .limit(limit)
            .collect()
            .await?;
        output::print_json(&Value::Array(items));
        return Ok(());
    }

    let issues = ctx
        .client
        .issues(&repo.owner, &repo.name)
        .list(&filter)
        .limit(limit)
        .collect()
        .await?;
    let rows: Vec<Vec<String>> = issues
        .iter()
        .map(|issue| {
            vec![
                format!("#{}", issue.number),
                issue.state.clone(),
                issue.title.clone(),
                issue.label_list().join(","),
                issue.author.clone().unwrap_or_else(|| "—".into()),
                short_date(issue.updated_at.as_deref()),
            ]
        })
        .collect();
    if rows.is_empty() {
        println!("No {state} issues in {repo}.");
        return Ok(());
    }
    output::print_table(
        &["#", "STATE", "TITLE", "LABELS", "AUTHOR", "UPDATED"],
        &rows,
    );
    Ok(())
}

async fn view(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    number: i64,
    comments: bool,
) -> Result<(), AppError> {
    let issues = ctx.client.issues(&repo.owner, &repo.name);
    if cli.json {
        let value = issues.get_raw(number).await?;
        output::print_json(&value);
        return Ok(());
    }
    let issue = issues.get(number).await?;
    let rows = vec![
        vec!["title".into(), issue.title.clone()],
        vec!["state".into(), issue.state.clone()],
        vec![
            "author".into(),
            issue.author.clone().unwrap_or_else(|| "—".into()),
        ],
        vec!["labels".into(), issue.label_list().join(", ")],
        vec!["assignees".into(), issue.assignees.join(", ")],
        vec!["created".into(), short_date(issue.created_at.as_deref())],
        vec!["body".into(), issue.body.clone().unwrap_or_default()],
    ];
    output::print_table(&["FIELD", "VALUE"], &rows);

    if comments {
        let list = issues.comments(number).await?;
        if list.is_empty() {
            println!("No comments.");
        }
        for comment in list {
            println!(
                "\n{} at {}: {}",
                comment.author.unwrap_or_else(|| "—".into()),
                short_date(comment.created_at.as_deref()),
                comment.body
            );
        }
    }
    Ok(())
}

async fn create(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    title: Option<String>,
    body: Option<String>,
    body_file: Option<String>,
    label: Vec<String>,
) -> Result<(), AppError> {
    let title = title.ok_or_else(|| AppError::Usage("issue title required (-t)".into()))?;
    let body = match (body, body_file) {
        (Some(_), Some(_)) => {
            return Err(AppError::Usage(
                "-b and --body-file are mutually exclusive".into(),
            ))
        }
        (Some(body), None) => Some(body),
        (None, Some(file)) => Some(
            std::fs::read_to_string(&file)
                .map_err(|e| AppError::Other(format!("could not read {file}: {e}")))?,
        ),
        (None, None) => None,
    };
    let options = CreateIssueOptions {
        title,
        body,
        labels: (!label.is_empty()).then_some(label),
        assignees: None,
    };
    let issues = ctx.client.issues(&repo.owner, &repo.name);
    if cli.json {
        let value = issues.create_raw(&options).await?;
        output::print_json(&value);
        return Ok(());
    }
    let issue = issues.create(&options).await?;
    println!("Created {repo}#{}: {}", issue.number, issue.title);
    Ok(())
}

async fn comment(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    number: i64,
    body: &str,
) -> Result<(), AppError> {
    let issues = ctx.client.issues(&repo.owner, &repo.name);
    if cli.json {
        let value = issues.add_comment_raw(number, body).await?;
        output::print_json(&value);
        return Ok(());
    }
    issues.add_comment(number, body).await?;
    println!("Commented on {repo}#{number}.");
    Ok(())
}

async fn set_state(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    number: i64,
    state: &str,
) -> Result<(), AppError> {
    let options = UpdateIssueOptions {
        state: Some(state.to_string()),
        ..Default::default()
    };
    let issues = ctx.client.issues(&repo.owner, &repo.name);
    if cli.json {
        let value = issues.update_raw(number, &options).await?;
        output::print_json(&value);
        return Ok(());
    }
    let issue = issues.update(number, &options).await?;
    println!("{repo}#{} is {}.", issue.number, issue.state);
    Ok(())
}

async fn edit(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    number: i64,
    title: Option<String>,
    body: Option<String>,
) -> Result<(), AppError> {
    if title.is_none() && body.is_none() {
        return Err(AppError::Usage(
            "nothing to edit: pass --title or --body".into(),
        ));
    }
    let options = UpdateIssueOptions {
        title,
        body,
        state: None,
    };
    let issues = ctx.client.issues(&repo.owner, &repo.name);
    if cli.json {
        let value = issues.update_raw(number, &options).await?;
        output::print_json(&value);
        return Ok(());
    }
    let issue = issues.update(number, &options).await?;
    println!("Updated {repo}#{}.", issue.number);
    Ok(())
}

fn short_date(iso: Option<&str>) -> String {
    crate::output::human_time(iso)
}
