//! `ifg repo …` (design doc §4.2): list, view, create, clone.
//! `--ssh` and credential injection land with `auth git-credential` in M3.

use serde_json::Value;

use super::{bootstrap, CommandContext};
use crate::cli::{Cli, RepoCommand};
use crate::context::resolve_repo;
use crate::error::AppError;
use crate::output;

const DEFAULT_LIMIT: u64 = 30;

pub async fn run(command: &RepoCommand, cli: &Cli) -> Result<(), AppError> {
    let ctx = bootstrap(cli)?;
    match command {
        RepoCommand::List { owner, limit } => list(&ctx, cli, owner.clone(), *limit).await,
        RepoCommand::View { target, web } => {
            let repo = resolve_target(cli, &ctx, target.clone())?;
            if *web {
                return crate::web::open(&crate::web::repo_url(&ctx.host.url, &repo));
            }
            view(&ctx, cli, &repo).await
        }
        RepoCommand::Create {
            name,
            description,
            private,
            org,
            init,
            default_branch,
        } => {
            let args = CreateArgs {
                name: name.clone(),
                description: description.clone(),
                private: *private,
                org: org.clone(),
                init: *init,
                default_branch: default_branch.clone(),
            };
            create(&ctx, cli, &args).await
        }
        RepoCommand::Clone { target, dir } => clone(&ctx, target, dir.clone()),
    }
}

/// `repo create` flags, bundled to keep the handler signature sane.
#[derive(Debug, Clone)]
struct CreateArgs {
    name: String,
    description: Option<String>,
    private: bool,
    org: Option<String>,
    init: bool,
    default_branch: Option<String>,
}

fn resolve_target(
    cli: &Cli,
    ctx: &CommandContext,
    arg: Option<String>,
) -> Result<crate::context::RepoContext, AppError> {
    resolve_repo(arg.as_deref().or(cli.repo.as_deref()), &ctx.host)
}

async fn list(
    ctx: &CommandContext,
    cli: &Cli,
    owner: Option<String>,
    limit: Option<u64>,
) -> Result<(), AppError> {
    let owner = match owner {
        Some(owner) => owner,
        None => {
            let key =
                crate::config::host_key_for_url(&ctx.host.url).or_else(|| ctx.host.name.clone());
            let user = key
                .as_deref()
                .and_then(|key| ctx.config.hosts.get(key))
                .and_then(|entry| entry.user.clone());
            match user {
                Some(user) => user,
                None => {
                    return Err(AppError::Usage(
                        "no owner known for this host; pass one: `ifg repo list <owner>`".into(),
                    ))
                }
            }
        }
    };

    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    if cli.json {
        let items: Vec<Value> = ctx
            .client
            .repos()
            .list_raw(&owner)
            .limit(limit)
            .collect()
            .await?;
        output::print_json(&Value::Array(items));
        return Ok(());
    }

    let repos = ctx
        .client
        .repos()
        .list(&owner)
        .limit(limit)
        .collect()
        .await?;
    let rows: Vec<Vec<String>> = repos
        .iter()
        .map(|repo| {
            vec![
                repo.name.clone(),
                visibility(repo.is_private),
                repo.default_branch.clone(),
                repo.stars_count.to_string(),
                shorten(repo.description.as_deref(), 48),
                short_date(repo.updated_at.as_deref()),
            ]
        })
        .collect();
    if rows.is_empty() {
        println!("No repositories for {owner}.");
        return Ok(());
    }
    output::print_table(
        &[
            "NAME",
            "VISIBILITY",
            "BRANCH",
            "STARS",
            "DESCRIPTION",
            "UPDATED",
        ],
        &rows,
    );
    Ok(())
}

async fn view(
    ctx: &CommandContext,
    cli: &Cli,
    repo: &crate::context::RepoContext,
) -> Result<(), AppError> {
    if cli.json {
        let value = ctx.client.repos().get_raw(&repo.owner, &repo.name).await?;
        output::print_json(&value);
        return Ok(());
    }
    let info = ctx.client.repos().get(&repo.owner, &repo.name).await?;
    let rows = vec![
        row("name", info.name.clone()),
        row("owner/repo", repo.to_string()),
        row("description", info.description.clone().unwrap_or_default()),
        row("visibility", visibility(info.is_private)),
        row("default branch", info.default_branch.clone()),
        row("stars", info.stars_count.to_string()),
        row("forks", info.forks_count.to_string()),
        row("created", short_date(info.created_at.as_deref())),
        row("updated", short_date(info.updated_at.as_deref())),
    ];
    output::print_table(&["FIELD", "VALUE"], &rows);
    Ok(())
}

async fn create(ctx: &CommandContext, cli: &Cli, args: &CreateArgs) -> Result<(), AppError> {
    let CreateArgs {
        name,
        description,
        private,
        org,
        init,
        default_branch,
    } = args.clone();
    let owner = org
        .clone()
        .unwrap_or_else(|| crate::config::host_key_for_url(&ctx.host.url).unwrap_or_default());
    let options = ifg_client::CreateRepoOptions {
        name: name.to_string(),
        description,
        is_private: Some(private),
        org,
        auto_init: Some(init),
        default_branch,
        ..Default::default()
    };
    if cli.json {
        let value = ctx.client.repos().create_raw(&options).await?;
        output::print_json(&value);
        return Ok(());
    }
    let repo = ctx.client.repos().create(&options).await?;
    println!("Created {}/{} ({})", owner, repo.name, ctx.host.url);
    if !init {
        println!(
            "Empty repository. Push an existing project with:
  git remote add origin {}/git/{}/{name}.git
  git push -u origin {}",
            ctx.host.url, owner, repo.default_branch
        );
    }
    Ok(())
}

fn clone(ctx: &CommandContext, repo: &str, dir: Option<String>) -> Result<(), AppError> {
    let repo = resolve_repo(Some(repo), &ctx.host)?;
    let url = format!("{}/git/{}.git", ctx.host.url, repo);
    println!("Cloning {url}");
    let status = std::process::Command::new("git")
        .arg("clone")
        .arg(&url)
        .args(dir)
        .status()
        .map_err(|e| AppError::Other(format!("could not run git clone: {e}")))?;
    if !status.success() {
        return Err(AppError::Other(format!("git clone failed for {url}")));
    }
    Ok(())
}

fn visibility(is_private: bool) -> String {
    if is_private {
        "private".into()
    } else {
        "public".into()
    }
}

fn row(field: &str, value: String) -> Vec<String> {
    vec![field.to_string(), value]
}

fn shorten(text: Option<&str>, max: usize) -> String {
    match text {
        None => "—".to_string(),
        Some(text) if text.chars().count() <= max => text.to_string(),
        Some(text) => {
            let cut: String = text.chars().take(max.saturating_sub(1)).collect();
            format!("{cut}…")
        }
    }
}

fn short_date(iso: Option<&str>) -> String {
    crate::output::human_time(iso)
}
