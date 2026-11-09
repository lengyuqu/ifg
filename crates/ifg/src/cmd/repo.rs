//! `ifg repo …` (design doc §4.2): list, view, create, clone.
//! `--ssh` and credential injection land with `auth git-credential` in M3.

use serde_json::Value;

use super::{bootstrap, CommandContext};
use crate::cli::{Cli, RepoCollaboratorsCommand, RepoCommand};
use crate::context::{resolve_repo, RepoContext};
use crate::error::AppError;
use crate::output;

const DEFAULT_LIMIT: u64 = 30;

pub async fn run(command: &RepoCommand, cli: &Cli) -> Result<(), AppError> {
    let ctx = bootstrap(cli)?;
    match command {
        RepoCommand::Collaborators { command } => {
            let repo = resolve_repo(cli.repo.as_deref(), &ctx.host)?;
            collaborators(&ctx, cli, &repo, command).await
        }
        RepoCommand::List { owner, limit } => list(&ctx, cli, owner.clone(), *limit).await,
        RepoCommand::View { target, web } => {
            let repo = resolve_target(cli, &ctx, target.clone())?;
            if *web {
                return crate::web::open(&crate::web::repo_url(&ctx.host.url, &repo));
            }
            view(&ctx, cli, &repo).await
        }
        RepoCommand::Edit {
            target,
            description,
            clear_description,
            visibility,
            default_branch,
            expected_version,
        } => {
            let repo = resolve_target(cli, &ctx, target.clone())?;
            if description.is_none()
                && !clear_description
                && visibility.is_none()
                && default_branch.is_none()
            {
                return Err(AppError::Other(
                    "provide at least one setting to change".into(),
                ));
            }
            let repos = ctx.client.repos();
            let version = match expected_version {
                Some(v) => v.clone(),
                None => repos
                    .get(&repo.owner, &repo.name)
                    .await?
                    .settings_version
                    .ok_or_else(|| {
                        AppError::Other(
                            "server does not expose a repository settings version".into(),
                        )
                    })?,
            };
            let mut body = serde_json::json!({"expected_version": version});
            if *clear_description {
                body["description"] = Value::Null;
            }
            if let Some(description) = description {
                body["description"] = serde_json::json!(description);
            }
            if let Some(visibility) = visibility {
                body["is_private"] = serde_json::json!(visibility == "private");
            }
            if let Some(branch) = default_branch {
                body["default_branch"] = serde_json::json!(branch);
            }
            let updated = repos
                .update_settings(&repo.owner, &repo.name, &body)
                .await?;
            if cli.json || cli.template.is_some() {
                super::show(cli, &updated);
            } else {
                output::line!("Updated repository settings for {repo}.");
            }
            Ok(())
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
        RepoCommand::Clone { target, dir, ssh } => clone(&ctx, target, dir.clone(), *ssh),
        RepoCommand::Delete { target, yes } => {
            let repo = resolve_target(cli, &ctx, target.clone())?;
            delete(&ctx, &repo, *yes).await
        }
    }
}

async fn collaborators(
    ctx: &CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    command: &RepoCollaboratorsCommand,
) -> Result<(), AppError> {
    let repos = ctx.client.repos();
    match command {
        RepoCollaboratorsCommand::List => {
            if cli.json || cli.template.is_some() {
                let value = repos.collaborators_raw(&repo.owner, &repo.name).await?;
                if let Some(template) = &cli.template {
                    let items = value
                        .as_array()
                        .ok_or_else(|| AppError::Other("invalid collaborator list".into()))?;
                    output::print_template(items, template);
                } else {
                    output::print_json(&value);
                }
            } else {
                let members = repos.collaborators(&repo.owner, &repo.name).await?;
                let rows: Vec<Vec<String>> = members
                    .iter()
                    .map(|m| {
                        vec![
                            m.user_id.to_string(),
                            m.id.to_string(),
                            m.permission.clone(),
                        ]
                    })
                    .collect();
                if rows.is_empty() {
                    output::line!("No direct collaborators in {repo}.");
                } else {
                    output::print_table(&["USER ID", "RECORD ID", "PERMISSION"], &rows);
                }
            }
        }
        RepoCollaboratorsCommand::Add {
            username,
            permission,
        } => {
            let value = repos
                .add_collaborator(&repo.owner, &repo.name, username, permission)
                .await?;
            show_collaborator(cli, &value);
        }
        RepoCollaboratorsCommand::Set {
            username,
            permission,
        } => {
            let user = ctx.client.users().get(username).await?;
            let members = repos.collaborators(&repo.owner, &repo.name).await?;
            let member = members
                .iter()
                .find(|m| m.user_id == user.id)
                .ok_or_else(|| {
                    AppError::NotFound(format!("{username} is not a direct collaborator in {repo}"))
                })?;
            let value = repos
                .update_collaborator(&repo.owner, &repo.name, member.id, permission)
                .await?;
            show_collaborator(cli, &value);
        }
        RepoCollaboratorsCommand::Remove { username, yes } => {
            let user = ctx.client.users().get(username).await?;
            let members = repos.collaborators(&repo.owner, &repo.name).await?;
            if !members.iter().any(|m| m.user_id == user.id) {
                return Err(AppError::NotFound(format!(
                    "{username} is not a direct collaborator in {repo}"
                )));
            }
            if !*yes && !super::confirm(&format!("Remove {username} from {repo}?"))? {
                return Err(AppError::Other("cancelled".into()));
            }
            repos
                .remove_collaborator(&repo.owner, &repo.name, user.id)
                .await?;
            if cli.json || cli.template.is_some() {
                show_collaborator(
                    cli,
                    &serde_json::json!({"username":username,"user_id":user.id,"removed":true}),
                );
            } else {
                output::line!("Removed {username} from {repo}.");
            }
        }
    }
    Ok(())
}

fn show_collaborator(cli: &Cli, value: &Value) {
    if let Some(template) = &cli.template {
        output::print_template(std::slice::from_ref(value), template);
    } else {
        output::print_json(value);
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

/// The login name cached for the current host, when `auth login` stored one.
fn cached_user(ctx: &CommandContext) -> Option<String> {
    let key = crate::config::host_key_for_url(&ctx.host.url).or_else(|| ctx.host.name.clone());
    key.as_deref()
        .and_then(|key| ctx.config.hosts.get(key))
        .filter(|entry| crate::config::same_base_url(&entry.url, &ctx.host.url))
        .and_then(|entry| entry.user.clone())
}

/// A cached username is tied to the stored credential, not an overriding PAT.
async fn account_name(ctx: &CommandContext) -> Result<String, AppError> {
    if crate::context::env_pat().is_none() {
        if let Some(user) = cached_user(ctx) {
            return Ok(user);
        }
    }
    Ok(ctx.client.users().me().await?.username)
}

async fn list(
    ctx: &CommandContext,
    cli: &Cli,
    owner: Option<String>,
    limit: Option<u64>,
) -> Result<(), AppError> {
    let owner = match owner {
        Some(owner) => owner,
        None => account_name(ctx).await.inspect_err(|_| {
            eprintln!("ifg: pass an explicit owner with `ifg repo list OWNER` if the token cannot read /users/me");
        })?,
    };

    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    if cli.json || cli.template.is_some() {
        let items: Vec<Value> = ctx
            .client
            .repos()
            .list_raw(&owner)
            .limit(limit)
            .collect()
            .await?;
        match &cli.template {
            Some(template) => output::print_template(&items, template),
            None => output::print_json(&Value::Array(items)),
        }
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
        output::line!("No repositories for {owner}.");
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
    if cli.json || cli.template.is_some() {
        let value = ctx.client.repos().get_raw(&repo.owner, &repo.name).await?;
        match &cli.template {
            Some(template) => output::print_template(std::slice::from_ref(&value), template),
            None => output::print_json(&value),
        }
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
    // A personal repository belongs to the account, not to the host: the old
    // fallback printed `Created forge.example.com/demo` and a remote URL under
    // `/git/forge.example.com/`, which does not exist.
    let options = ifg_client::CreateRepoOptions {
        name: name.to_string(),
        description,
        is_private: Some(private),
        org: org.clone(),
        auto_init: Some(init),
        default_branch,
        ..Default::default()
    };
    if cli.json || cli.template.is_some() {
        let value = ctx.client.repos().create_raw(&options).await?;
        match &cli.template {
            Some(template) => output::print_template(std::slice::from_ref(&value), template),
            None => output::print_json(&value),
        }
        return Ok(());
    }
    let repo = ctx.client.repos().create(&options).await?;
    // The owner name is display-only. Resolving it must never gate the create:
    // a token with `repo` but not `user` scope is enough to create a
    // repository, and `/users/me` would answer 403.
    match display_owner(ctx, org.as_deref()).await {
        Owner::Known(owner) => {
            output::line!("Created {owner}/{} ({})", repo.name, ctx.host.url);
            if !init {
                output::line!(
                    "Empty repository. Push an existing project with:
  git remote add origin {}/git/{owner}/{name}.git
  git push -u origin {}",
                    ctx.host.url,
                    repo.default_branch
                );
            }
        }
        Owner::Unknown => {
            // Never invent a name or print a remote URL under the host name.
            output::line!("Created {} ({})", repo.name, ctx.host.url);
            eprintln!(
                "ifg: note: could not read the account name (GET /users/me failed); \
                 the repository belongs to the token's user"
            );
            if !init {
                output::line!(
                    "Empty repository. Push an existing project with (replace OWNER with your account):
  git remote add origin {}/git/OWNER/{name}.git
  git push -u origin {}",
                    ctx.host.url, repo.default_branch
                );
            }
        }
    }
    Ok(())
}

/// Where the display owner for `repo create` came from.
enum Owner {
    Known(String),
    /// The account could not be read; the create already succeeded.
    Unknown,
}

/// The account a personal repository will belong to, for display purposes.
///
/// A PAT from `IRONFORGE_PAT` may belong to a different account than the one
/// cached in `hosts.toml`, so the cache is only trusted when the stored token
/// is what is actually authenticating. Failure to read `/users/me` degrades to
/// [`Owner::Unknown`] instead of failing the command.
async fn display_owner(ctx: &CommandContext, org: Option<&str>) -> Owner {
    if let Some(org) = org {
        return Owner::Known(org.to_string());
    }
    match account_name(ctx).await {
        Ok(username) => Owner::Known(username),
        Err(_) => Owner::Unknown,
    }
}

fn clone(
    ctx: &CommandContext,
    target: &str,
    dir: Option<String>,
    ssh: bool,
) -> Result<(), AppError> {
    let repo = resolve_repo(Some(target), &ctx.host)?;
    let url = if ssh {
        ssh_clone_url(&ctx.host.url, &repo)?
    } else {
        format!("{}/git/{}.git", ctx.host.url, repo)
    };
    output::line!("Cloning {url}");
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

/// `ssh://git@HOST:PORT/OWNER/NAME` — the server's SSH clone form
/// (IronForge serves git over SSH with the `git` user; port defaults to
/// 2222, override with `IRONFORGE_SSH_PORT`).
fn ssh_clone_url(host_url: &str, repo: &RepoContext) -> Result<String, AppError> {
    let parsed = url::Url::parse(host_url)
        .map_err(|e| AppError::Other(format!("invalid host URL {host_url}: {e}")))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::Other("host URL has no host".into()))?;
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]") // IPv6 literal
    } else {
        host.to_string()
    };
    let port = std::env::var("IRONFORGE_SSH_PORT").unwrap_or_else(|_| "2222".into());
    let port_part = if port.is_empty() {
        String::new()
    } else {
        format!(":{port}")
    };
    Ok(format!(
        "ssh://git@{host}{port_part}/{}/{}",
        repo.owner, repo.name
    ))
}

/// `repo delete`: the server soft-deletes and the name becomes reusable.
async fn delete(ctx: &CommandContext, repo: &RepoContext, yes: bool) -> Result<(), AppError> {
    if !yes
        && !super::confirm(&format!(
            "Delete {repo}? This removes the repository for everyone."
        ))?
    {
        output::line!("Aborted.");
        return Ok(());
    }
    ctx.client.repos().delete(&repo.owner, &repo.name).await?;
    output::line!("Deleted {repo}.");
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
