//! `ifg pr …` (design doc §4.4, minus `checkout` which lands in M3).

use ifg_client::{CreatePrOptions, MergeStrategy, ReviewAction};
use serde_json::Value;

use super::bootstrap;
use crate::cli::{Cli, PrCommand};
use crate::context::{resolve_repo, RepoContext};
use crate::error::AppError;
use crate::output;

const DEFAULT_LIMIT: u64 = 30;

pub async fn run(command: &PrCommand, cli: &Cli) -> Result<(), AppError> {
    let ctx = bootstrap(cli)?;
    let repo = resolve_repo(cli.repo.as_deref(), &ctx.host)?;
    match command {
        PrCommand::List { state, limit } => list(&ctx, cli, &repo, state.clone(), *limit).await,
        PrCommand::View { number } => view(&ctx, cli, &repo, *number).await,
        PrCommand::Diff { number } => diff(&ctx, &repo, *number).await,
        PrCommand::Create {
            base,
            head,
            title,
            body,
            draft,
        } => {
            let args = CreateArgs {
                base: base.clone(),
                head: head.clone(),
                title: title.clone(),
                body: body.clone(),
                draft: *draft,
            };
            create(&ctx, cli, &repo, &args).await
        }
        PrCommand::Merge {
            number,
            merge: _,
            squash,
            rebase,
            auto,
        } => merge(&ctx, cli, &repo, *number, *squash, *rebase, *auto).await,
        PrCommand::Review {
            number,
            approve,
            request_changes,
            body,
        } => {
            review(
                &ctx,
                &repo,
                *number,
                *approve,
                *request_changes,
                body.clone(),
            )
            .await
        }
        PrCommand::Checkout { number } => checkout(&ctx, &repo, *number).await,
    }
}

async fn list(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    state: String,
    limit: Option<u64>,
) -> Result<(), AppError> {
    let state_filter: Option<&str> = if state == "all" {
        None
    } else {
        Some(state.as_str())
    };
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    if cli.json {
        let items: Vec<Value> = ctx
            .client
            .pulls(&repo.owner, &repo.name)
            .list_raw(state_filter)
            .limit(limit)
            .collect()
            .await?;
        output::print_json(&Value::Array(items));
        return Ok(());
    }
    let prs = ctx
        .client
        .pulls(&repo.owner, &repo.name)
        .list(state_filter)
        .limit(limit)
        .collect()
        .await?;
    let rows: Vec<Vec<String>> = prs
        .iter()
        .map(|pr| {
            vec![
                format!("#{}", pr.number),
                pr.state.clone(),
                pr.title.clone(),
                pr.branch_label(),
                short_date(pr.updated_at.as_deref()),
            ]
        })
        .collect();
    if rows.is_empty() {
        println!("No {state} pull requests in {repo}.");
        return Ok(());
    }
    output::print_table(&["#", "STATE", "TITLE", "HEAD -> BASE", "UPDATED"], &rows);
    Ok(())
}

async fn view(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    number: i64,
) -> Result<(), AppError> {
    let pulls = ctx.client.pulls(&repo.owner, &repo.name);
    if cli.json {
        let value = pulls.get_raw(number).await?;
        output::print_json(&value);
        return Ok(());
    }
    let pr = pulls.get(number).await?;
    let rows = vec![
        vec!["title".into(), pr.title.clone()],
        vec!["state".into(), pr.state.clone()],
        vec!["draft".into(), pr.is_draft.to_string()],
        vec!["branches".into(), pr.branch_label()],
        vec![
            "head".into(),
            pr.head_sha.clone().unwrap_or_else(|| "—".into()),
        ],
        vec![
            "merge commit".into(),
            pr.merge_commit_sha.clone().unwrap_or_else(|| "—".into()),
        ],
        vec!["created".into(), short_date(pr.created_at.as_deref())],
        vec!["merged".into(), short_date(pr.merged_at.as_deref())],
        vec!["body".into(), pr.body.clone().unwrap_or_default()],
    ];
    output::print_table(&["FIELD", "VALUE"], &rows);
    Ok(())
}

async fn diff(
    ctx: &super::CommandContext,
    repo: &RepoContext,
    number: i64,
) -> Result<(), AppError> {
    let raw = ctx
        .client
        .pulls(&repo.owner, &repo.name)
        .diff(number)
        .await?;
    // The server returns a structured JSON diff; render the patches. Older or
    // proxied deployments may answer with a plain text patch — print those
    // verbatim.
    match serde_json::from_str::<Value>(&raw) {
        Ok(value) if value.get("files_changed").is_some() => {
            let stats = &value["stats"];
            println!(
                "{} file(s) changed, {} insertion(s), {} deletion(s)",
                stats["files_changed"].as_i64().unwrap_or(0),
                stats["total_additions"].as_i64().unwrap_or(0),
                stats["total_deletions"].as_i64().unwrap_or(0),
            );
            for file in value["files_changed"].as_array().unwrap_or(&Vec::new()) {
                println!(
                    "\n{} ({})",
                    file["path"].as_str().unwrap_or("?"),
                    file["status"].as_str().unwrap_or("?")
                );
                if let Some(patch) = file["patch"].as_str() {
                    print!("{patch}");
                }
            }
            Ok(())
        }
        Ok(_) | Err(_) => {
            print!("{raw}");
            Ok(())
        }
    }
}

/// `pr create` flags, bundled to keep the handler signature sane.
#[derive(Debug, Clone)]
struct CreateArgs {
    base: Option<String>,
    head: Option<String>,
    title: Option<String>,
    body: Option<String>,
    draft: bool,
}

async fn create(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    args: &CreateArgs,
) -> Result<(), AppError> {
    let CreateArgs {
        base,
        head,
        title,
        body,
        draft,
    } = args.clone();
    let head = match head {
        Some(head) => head,
        None => current_branch()?,
    };

    let base = match base {
        Some(base) => base,
        None => {
            ctx.client
                .repos()
                .get(&repo.owner, &repo.name)
                .await?
                .default_branch
        }
    };

    // Head must exist on the server: warn about unpushed local commits
    // (review finding: `@{upstream}` alone is not sufficient).
    if title.is_none() {
        ensure_current_branch_pushed(&head)?;
    }

    let title = match title {
        Some(title) => title,
        None => {
            head_commit_subject().ok_or_else(|| AppError::Usage("PR title required (-t)".into()))?
        }
    };

    let options = CreatePrOptions {
        title,
        body,
        head,
        base,
        draft,
    };
    let pulls = ctx.client.pulls(&repo.owner, &repo.name);
    if cli.json {
        let value = pulls.create_raw(&options).await?;
        output::print_json(&value);
        return Ok(());
    }
    let pr = pulls.create(&options).await?;
    println!(
        "Created {repo}#{}: {} ({} -> {})",
        pr.number, pr.title, pr.head_branch, pr.base_branch
    );
    Ok(())
}

/// Current branch via `git symbolic-ref --short HEAD`.
fn current_branch() -> Result<String, AppError> {
    let output = std::process::Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .map_err(|e| AppError::Other(format!("could not run git: {e}")))?;
    if !output.status.success() {
        return Err(AppError::Usage(
            "not inside a git work tree; pass --head BRANCH".into(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// First commit subject of HEAD, as a default PR title.
fn head_commit_subject() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["log", "-1", "--pretty=%s"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let subject = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if subject.is_empty() {
        None
    } else {
        Some(subject)
    }
}

/// Fail with a helpful message when local commits are not pushed. Checks the
/// branch's upstream exists and is not behind the local HEAD.
fn ensure_current_branch_pushed(branch: &str) -> Result<(), AppError> {
    let upstream = std::process::Command::new("git")
        .args([
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ])
        .output();
    let has_upstream = matches!(&upstream, Ok(out) if out.status.success());
    if !has_upstream {
        return Err(AppError::Other(format!(
            "branch '{branch}' has no upstream; push it first: git push -u origin {branch}"
        )));
    }
    let ahead = std::process::Command::new("git")
        .args(["rev-list", "--count", "@{upstream}..HEAD"])
        .output();
    if let Ok(out) = ahead {
        if out.status.success() {
            let count = String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse::<u64>()
                .unwrap_or(0);
            if count > 0 {
                return Err(AppError::Other(format!(
                    "{count} local commit(s) on '{branch}' are not pushed; \
                     push first: git push"
                )));
            }
            return Ok(());
        }
    }
    Ok(())
}

async fn merge(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    number: i64,
    squash: bool,
    rebase: bool,
    auto: bool,
) -> Result<(), AppError> {
    let strategy = if squash {
        MergeStrategy::Squash
    } else if rebase {
        MergeStrategy::Rebase
    } else {
        // --merge is the default; the flag exists for explicitness
        MergeStrategy::Merge
    };
    let pulls = ctx.client.pulls(&repo.owner, &repo.name);
    if auto {
        let value = pulls.enable_auto_merge(number, strategy).await?;
        output::print_json(&value);
        println!(
            "Auto-merge enabled for {repo}#{number} ({}).",
            strategy.as_str()
        );
        return Ok(());
    }
    if cli.json {
        let value = pulls.merge_raw(number, strategy).await?;
        output::print_json(&value);
        return Ok(());
    }
    let outcome = pulls.merge(number, strategy).await?;
    match outcome.merge_commit_sha {
        Some(sha) => println!(
            "Merged {repo}#{number} ({}): {}",
            outcome.strategy.as_deref().unwrap_or(strategy.as_str()),
            &sha[..sha.len().min(12)]
        ),
        None => println!("Merged {repo}#{number}."),
    }
    Ok(())
}

async fn review(
    ctx: &super::CommandContext,
    repo: &RepoContext,
    number: i64,
    approve: bool,
    request_changes: bool,
    body: Option<String>,
) -> Result<(), AppError> {
    let action = if approve {
        ReviewAction::Approve
    } else if request_changes {
        ReviewAction::RequestChanges
    } else {
        ReviewAction::Comment
    };
    let value = ctx
        .client
        .pulls(&repo.owner, &repo.name)
        .review_raw(number, action, body.as_deref())
        .await?;
    output::print_json(&value);
    Ok(())
}

fn short_date(iso: Option<&str>) -> String {
    match iso {
        Some(iso) => iso.chars().take(10).collect(),
        None => "—".to_string(),
    }
}

/// `pr checkout <number>`: fetch the PR head branch (from the repo's clone
/// URL, authenticated with a PAT when we have one) and create `pr-<number>`.
/// Same-repo PRs only — the server does not maintain `refs/pull/<n>/head`
/// and fork head repos have no by-id lookup (design doc §7.2).
async fn checkout(
    ctx: &super::CommandContext,
    repo: &RepoContext,
    number: i64,
) -> Result<(), AppError> {
    let pr = ctx
        .client
        .pulls(&repo.owner, &repo.name)
        .get(number)
        .await?;
    if pr.head_repo_id.is_some() {
        return Err(AppError::Other(
            "fork PR checkout is not supported yet (no refs/pull and no head-repo lookup; §7.2)"
                .into(),
        ));
    }
    let branch = pr.head_branch.clone();
    let local = format!("pr-{number}");

    let clone_url = format!("{}/git/{}/{}.git", ctx.host.url, repo.owner, repo.name);
    let mut fetch = std::process::Command::new("git");
    if let Some(token) = &ctx.host.token {
        // Bearer PAT over git-http; `-c` is a git *global* option (before the
        // subcommand) and keeps the token out of any stored remote URL.
        fetch.arg("-c").arg(format!(
            "http.extraheader=Authorization: Bearer {}",
            token.expose()
        ));
    }
    fetch.args(["fetch", &clone_url, &branch]);
    let output = fetch
        .output()
        .map_err(|e| AppError::Other(format!("could not run git fetch: {e}")))?;
    if !output.status.success() {
        return Err(AppError::Other(format!(
            "git fetch {clone_url} {branch} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let status = std::process::Command::new("git")
        .args(["switch", "-c", &local, "FETCH_HEAD"])
        .status()
        .map_err(|e| AppError::Other(format!("could not run git switch: {e}")))?;
    if !status.success() {
        return Err(AppError::Other(format!(
            "could not create branch {local} (it may already exist; try `git switch {local}`)"
        )));
    }
    println!("{repo}#{number} '{branch}' checked out as {local}");
    Ok(())
}
