//! `ifg pr …` (design doc §4.4, minus `checkout` which lands in M3).

use ifg_client::{CreatePrOptions, MergeStrategy, ReviewAction, UpdatePrOptions};
use serde_json::Value;

use super::bootstrap;
use crate::cli::{Cli, PrCommand, PrReviewersCommand};
use crate::context::{resolve_repo, RepoContext};
use crate::error::AppError;
use crate::output;

const DEFAULT_LIMIT: u64 = 30;

pub async fn run(command: &PrCommand, cli: &Cli) -> Result<(), AppError> {
    let ctx = bootstrap(cli)?;
    let repo = resolve_repo(cli.repo.as_deref(), &ctx.host)?;
    match command {
        PrCommand::List { state, limit } => list(&ctx, cli, &repo, state.clone(), *limit).await,
        PrCommand::View {
            number,
            web,
            comments,
        } => {
            if *web {
                return crate::web::open(&crate::web::pr_url(&ctx.host.url, &repo, *number));
            }
            view(&ctx, cli, &repo, *number, *comments).await
        }
        PrCommand::Diff { number } => diff(&ctx, &repo, *number).await,
        PrCommand::Create {
            base,
            head,
            title,
            body,
            draft,
            editor,
        } => {
            let args = CreateArgs {
                base: base.clone(),
                head: head.clone(),
                title: title.clone(),
                body: body.clone(),
                draft: *draft,
                editor: *editor,
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
        PrCommand::Edit {
            number,
            title,
            body,
            draft,
            ready,
        } => {
            if title.is_none() && body.is_none() && !draft && !ready {
                return Err(AppError::Usage(
                    "provide --title, --body, --draft or --ready".into(),
                ));
            }
            let options = UpdatePrOptions {
                title: title.clone(),
                body: body.clone(),
                draft: if *draft {
                    Some(true)
                } else if *ready {
                    Some(false)
                } else {
                    None
                },
                ..Default::default()
            };
            let value = ctx
                .client
                .pulls(&repo.owner, &repo.name)
                .update_raw(*number, &options)
                .await?;
            show_value(cli, &value);
            Ok(())
        }
        PrCommand::Close { number } | PrCommand::Reopen { number } => {
            let state = if matches!(command, PrCommand::Close { .. }) {
                "closed"
            } else {
                "open"
            };
            let options = UpdatePrOptions {
                state: Some(state.into()),
                ..Default::default()
            };
            let value = ctx
                .client
                .pulls(&repo.owner, &repo.name)
                .update_raw(*number, &options)
                .await?;
            show_value(cli, &value);
            Ok(())
        }
        PrCommand::Comment { number, body } => {
            if body.trim().is_empty() {
                return Err(AppError::Usage("comment body must not be empty".into()));
            }
            let value = ctx
                .client
                .pulls(&repo.owner, &repo.name)
                .review_raw(*number, ReviewAction::Comment, Some(body))
                .await?;
            show_value(cli, &value);
            Ok(())
        }
        PrCommand::Reviews { number } => {
            let value = ctx
                .client
                .pulls(&repo.owner, &repo.name)
                .reviews_raw(*number)
                .await?;
            show_value(cli, &value);
            Ok(())
        }
        PrCommand::Reviewers { command } => {
            let pulls = ctx.client.pulls(&repo.owner, &repo.name);
            let value = match command {
                PrReviewersCommand::List { number } => pulls.reviewers(*number).await?,
                PrReviewersCommand::Add { number, username } => {
                    pulls.request_reviewer(*number, username).await?
                }
                PrReviewersCommand::Remove { number, username } => {
                    pulls.remove_reviewer(*number, username).await?;
                    serde_json::json!({"removed":username})
                }
            };
            show_value(cli, &value);
            Ok(())
        }
    }
}

fn show_value(cli: &Cli, value: &Value) {
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
    if cli.json || cli.template.is_some() {
        let items: Vec<Value> = ctx
            .client
            .pulls(&repo.owner, &repo.name)
            .list_raw(state_filter)
            .limit(limit)
            .collect()
            .await?;
        match &cli.template {
            Some(template) => output::print_template(&items, template),
            None => output::print_json(&Value::Array(items)),
        }
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
    comments: bool,
) -> Result<(), AppError> {
    let pulls = ctx.client.pulls(&repo.owner, &repo.name);
    if cli.json || cli.template.is_some() {
        let mut value = pulls.get_raw(number).await?;
        if comments {
            let list = pulls.comments_raw(number).await?;
            value["comments"] = list;
        }
        match &cli.template {
            Some(template) => output::print_template(std::slice::from_ref(&value), template),
            None => output::print_json(&value),
        }
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

    if comments {
        let list = pulls.comments(number).await?;
        if list.is_empty() {
            println!("\nNo inline review comments.");
        } else {
            println!();
            for comment in &list {
                let location = match (&comment.path, comment.line) {
                    (Some(path), Some(line)) => format!("{path}:{line}"),
                    (Some(path), None) => path.clone(),
                    _ => "general".to_string(),
                };
                println!(
                    "{} (author {}, {}):\n  {}\n",
                    location,
                    comment
                        .author_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "—".into()),
                    short_date(comment.created_at.as_deref()),
                    comment.body.trim().replace('\n', "\n  ")
                );
            }
        }
    }
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
                    output::print_diff(patch);
                }
            }
            Ok(())
        }
        Ok(_) | Err(_) => {
            output::print_diff(&raw);
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
    editor: bool,
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
        editor,
    } = args.clone();
    // `--editor` replaces -t/-b with a composed file.
    let (title, body) = if editor {
        let (composed_title, composed_body) = super::compose_in_editor(
            title.as_deref().unwrap_or(""),
            body.as_deref().unwrap_or(""),
        )?;
        (Some(composed_title), Some(composed_body))
    } else {
        (title, body)
    };
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
    if cli.json || cli.template.is_some() {
        let value = pulls.create_raw(&options).await?;
        match &cli.template {
            Some(template) => output::print_template(std::slice::from_ref(&value), template),
            None => output::print_json(&value),
        }
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
pub(crate) fn current_branch() -> Result<String, AppError> {
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
    if cli.json || cli.template.is_some() {
        let value = pulls.merge_raw(number, strategy).await?;
        match &cli.template {
            Some(template) => output::print_template(std::slice::from_ref(&value), template),
            None => output::print_json(&value),
        }
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
    crate::output::human_time(iso)
}

/// `pr checkout <number>` (design doc §7.2): fetch the PR head branch — from
/// the fork when the PR came from one — and bring `pr-<number>` up to date.
/// Re-running on an existing branch fast-forwards it instead of failing.
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
    let branch = pr.head_branch.clone();
    let local = format!("pr-{number}");

    // Fork PRs: fetch from the head repository (needs the enrichment added
    // upstream — the PR row alone only knows head_repo_id).
    let (clone_url, origin_label) = match pr.head_repo_id {
        None => (
            format!("{}/git/{}/{}.git", ctx.host.url, repo.owner, repo.name),
            repo.to_string(),
        ),
        Some(_) => match (pr.head_repo_owner.as_deref(), pr.head_repo_name.as_deref()) {
            (Some(owner), Some(name)) => (
                format!("{}/git/{owner}/{name}.git", ctx.host.url),
                format!("{owner}/{name}"),
            ),
            _ => {
                return Err(AppError::Other(
                    "fork PR checkout needs the head repository, which this server did not                      report (upgrade IronForge to a build with head_repo_owner)"
                        .into(),
                ))
            }
        },
    };

    // Try the head branch first (always fresh). When it is gone — deleted
    // after merge, the usual case for re-checkouts — fall back to the
    // server's refs/pull/<n>/head mirror.
    let branch_error = fetch_ref(ctx, &clone_url, &branch).err();
    if let Some(branch_error) = branch_error {
        let pull_ref = format!("refs/pull/{number}/head");
        match fetch_ref(ctx, &clone_url, &pull_ref) {
            Ok(()) => eprintln!(
                "ifg: note: branch '{branch}' is gone; fetched {pull_ref} instead (server mirror)"
            ),
            Err(_) => return Err(branch_error),
        }
    }

    if branch_exists(&local) {
        // Existing branch: switch and fast-forward. A diverged branch keeps
        // the user's commits — we refuse rather than reset.
        git_success(&["switch", &local])?;
        match git_success(&["merge", "--ff-only", "FETCH_HEAD"]) {
            Ok(()) => println!("{repo}#{number} updated ({origin_label}:{branch} → {local})"),
            Err(_) => {
                return Err(AppError::Other(format!(
                    "local branch {local} has diverged from {origin_label}:{branch}; \
                     resolve it by hand (git log {local}) or delete the branch and re-run"
                )))
            }
        }
    } else {
        git_success(&["switch", "-c", &local, "FETCH_HEAD"])?;
        println!("{repo}#{number} '{branch}' checked out as {local} (from {origin_label})");
    }
    Ok(())
}

/// `git fetch <url> <refspec>` with the PAT passed through GIT_CONFIG_* so it
/// never lands in argv or a stored remote URL.
fn fetch_ref(ctx: &super::CommandContext, url: &str, refspec: &str) -> Result<(), AppError> {
    // The refspec comes from the server's PR row; git would read a leading
    // `-` as an option instead of a ref.
    if refspec.starts_with('-') {
        return Err(AppError::Other(format!(
            "refusing to fetch a refspec that parses as an option: {refspec}"
        )));
    }
    let mut fetch = std::process::Command::new("git");
    fetch.args(["fetch", url, refspec]);
    if let Some(token) = &ctx.host.token {
        let existing = std::env::var("GIT_CONFIG_COUNT").ok();
        for (key, value) in crate::git::bearer_header_env(token.expose(), existing.as_deref()) {
            fetch.env(key, value);
        }
    }
    let output = fetch
        .output()
        .map_err(|e| AppError::Other(format!("could not run git fetch: {e}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(AppError::Other(format!(
            "git fetch {url} {refspec} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

/// Whether a local branch with this name exists.
fn branch_exists(name: &str) -> bool {
    std::process::Command::new("git")
        .args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{name}"),
        ])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Run a git command, erroring with its stderr when it fails.
fn git_success(args: &[&str]) -> Result<(), AppError> {
    let output = std::process::Command::new("git")
        .args(args)
        .output()
        .map_err(|e| AppError::Other(format!("could not run git: {e}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(AppError::Other(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}
