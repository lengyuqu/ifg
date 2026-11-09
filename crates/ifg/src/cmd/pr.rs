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
                cli,
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
        output::line!("No {state} pull requests in {repo}.");
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
            output::line!("\nNo inline review comments.");
        } else {
            output::line!();
            for comment in &list {
                let location = match (&comment.path, comment.line) {
                    (Some(path), Some(line)) => format!("{path}:{line}"),
                    (Some(path), None) => path.clone(),
                    _ => "general".to_string(),
                };
                output::line!(
                    "{} (author {}, {}):\n  {}\n",
                    output::sanitize(&location),
                    comment
                        .author_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "—".into()),
                    short_date(comment.created_at.as_deref()),
                    output::sanitize(comment.body.trim()).replace('\n', "\n  ")
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
            output::line!(
                "{} file(s) changed, {} insertion(s), {} deletion(s)",
                stats["files_changed"].as_i64().unwrap_or(0),
                stats["total_additions"].as_i64().unwrap_or(0),
                stats["total_deletions"].as_i64().unwrap_or(0),
            );
            for file in value["files_changed"].as_array().unwrap_or(&Vec::new()) {
                output::line!(
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
    // Only a head taken from the current checkout must be pushed; an explicit
    // `--head` names a branch the server may already know. The check used to
    // depend on whether a title was given, so `-t` bypassed it entirely.
    let (head, head_is_local) = match head {
        Some(head) => (head, false),
        None => (current_branch()?, true),
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

    if head_is_local {
        ensure_branch_pushed(ctx, repo, &head)?;
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
    output::line!(
        "Created {repo}#{}: {} ({} -> {})",
        pr.number,
        pr.title,
        pr.head_branch,
        pr.base_branch
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

/// Fail unless `branch` is on the remote that corresponds to the target
/// repository.
///
/// Checking any `refs/remotes/*/<branch>` was not enough: a `backup` remote (or
/// a stale `origin`) could satisfy it while the target repository stayed
/// behind. Only remotes whose URL parses to `repo` on the resolved host count,
/// and an upstream pointing somewhere else is refused too.
fn ensure_branch_pushed(
    ctx: &super::CommandContext,
    repo: &RepoContext,
    branch: &str,
) -> Result<(), AppError> {
    let remotes = matching_remotes(ctx, repo);
    if remotes.is_empty() {
        return Err(AppError::Other(format!(
            "no git remote points at {repo} on {}; add one or pass --head",
            ctx.host.url
        )));
    }

    if let Some((upstream_remote, upstream)) = upstream_ref(branch) {
        if !remotes.iter().any(|name| name == &upstream_remote) {
            return Err(AppError::Other(format!(
                "branch '{branch}' tracks {upstream}, which is not {repo} on {}; \
                 push it to {repo} first",
                ctx.host.url
            )));
        }
        // The API receives the local branch's name as `head`. An upstream
        // such as origin/published does not prove origin/feature has HEAD.
        let target = format!("{upstream_remote}/{branch}");
        let target_ref = format!("refs/remotes/{target}");
        match rev_count(&format!("{target_ref}..HEAD")) {
            Some(0) => Ok(()),
            Some(count) => Err(AppError::Other(format!(
                "{count} local commit(s) on '{branch}' are not pushed to {target}; \
                 push first: git push {upstream_remote} {branch}"
            ))),
            // Cannot tell: refuse rather than create a PR that may lack commits.
            None => Err(AppError::Other(format!(
                "could not compare '{branch}' with {target}; \
                 push first: git push {upstream_remote} {branch}"
            ))),
        }
    } else if remotes
        .iter()
        .any(|name| remote_tracking_matches_head(name, branch))
    {
        // Pushed without `-u`: no upstream, but the right remote has HEAD.
        Ok(())
    } else {
        Err(AppError::Other(format!(
            "branch '{branch}' is not pushed to {repo} on {}; run: git push {} {branch}",
            ctx.host.url,
            remotes.first().map(String::as_str).unwrap_or("origin")
        )))
    }
}

/// Remotes whose URL resolves to `repo` on the resolved host.
fn matching_remotes(ctx: &super::CommandContext, repo: &RepoContext) -> Vec<String> {
    crate::git::git_remotes()
        .into_iter()
        .filter(|(_, remote)| {
            remote.owner == repo.owner
                && remote.name == repo.name
                && crate::context::remote_matches_url(remote, &ctx.host.url)
        })
        .map(|(name, _)| name)
        .collect()
}

/// The configured upstream's remote and display name. Git supplies the remote
/// explicitly because remote names may themselves contain `/`.
fn upstream_ref(branch: &str) -> Option<(String, String)> {
    let output = std::process::Command::new("git")
        .args([
            "for-each-ref",
            "--format=%(upstream:remotename)%00%(upstream:short)",
            &format!("refs/heads/{branch}"),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let (remote, name) = text.trim_end().split_once('\0')?;
    (!remote.is_empty() && !name.is_empty()).then(|| (remote.to_string(), name.to_string()))
}

fn rev_count(range: &str) -> Option<u64> {
    let output = std::process::Command::new("git")
        .args(["rev-list", "--count", range])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

fn git_rev_parse(reference: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", reference])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!sha.is_empty()).then_some(sha)
}

/// Whether `<remote>/<branch>` already points at HEAD.
fn remote_tracking_matches_head(remote: &str, branch: &str) -> bool {
    let Some(head) = git_rev_parse("HEAD") else {
        return false;
    };
    git_rev_parse(&format!("refs/remotes/{remote}/{branch}")).as_deref() == Some(head.as_str())
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
        // Machine-readable modes keep stdout pure: the confirmation goes to
        // stderr, because `--json` output must parse as one JSON document.
        if cli.json || cli.template.is_some() {
            match &cli.template {
                Some(template) => output::print_template(std::slice::from_ref(&value), template),
                None => output::print_json(&value),
            }
            eprintln!(
                "Auto-merge enabled for {repo}#{number} ({}).",
                strategy.as_str()
            );
            return Ok(());
        }
        output::line!(
            "Auto-merge enabled for {repo}#{number} ({}).",
            strategy.as_str()
        );
        output::print_json(&value);
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
        Some(sha) => output::line!(
            "Merged {repo}#{number} ({}): {}",
            outcome.strategy.as_deref().unwrap_or(strategy.as_str()),
            &sha[..sha.len().min(12)]
        ),
        None => output::line!("Merged {repo}#{number}."),
    }
    Ok(())
}

async fn review(
    ctx: &super::CommandContext,
    cli: &Cli,
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
    super::show(cli, &value);
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
    let target_url = format!("{}/git/{}/{}.git", ctx.host.url, repo.owner, repo.name);

    // Fork PRs: fetch from the head repository (needs the enrichment added
    // upstream — the PR row alone only knows head_repo_id).
    let (clone_url, origin_label) = match pr.head_repo_id {
        None => (Some(target_url.clone()), repo.to_string()),
        Some(_) => match (pr.head_repo_owner.as_deref(), pr.head_repo_name.as_deref()) {
            (Some(owner), Some(name)) => (
                Some(format!("{}/git/{owner}/{name}.git", ctx.host.url)),
                format!("{owner}/{name}"),
            ),
            // The fork may have been deleted. Its PR mirror is still owned
            // by the target repository and can be fetched without enrichment.
            _ => (None, "unavailable fork".into()),
        },
    };

    // Try the head branch first (always fresh). When it is gone — deleted
    // after merge, the usual case for re-checkouts — fall back to the
    // server's refs/pull/<n>/head mirror.
    let branch_error = match clone_url {
        Some(url) => fetch_ref(ctx, &url, &branch).err(),
        None => Some(AppError::Other(
            "fork head repository is unavailable".into(),
        )),
    };
    if let Some(branch_error) = branch_error {
        let pull_ref = format!("refs/pull/{number}/head");
        match fetch_ref(ctx, &target_url, &pull_ref) {
            Ok(()) => eprintln!(
                "ifg: note: branch '{branch}' is gone; fetched {pull_ref} instead (server mirror)"
            ),
            Err(mirror_error) => {
                return Err(AppError::Other(format!(
                    "{branch_error}; target repository mirror is unavailable: {mirror_error}; \
                 this server may not publish fork PR head mirrors"
                )))
            }
        }
    }

    if branch_exists(&local) {
        // Existing branch: switch and fast-forward. A diverged branch keeps
        // the user's commits — we refuse rather than reset.
        git_success(&["switch", &local])?;
        match git_success(&["merge", "--ff-only", "FETCH_HEAD"]) {
            Ok(()) => output::line!("{repo}#{number} updated ({origin_label}:{branch} → {local})"),
            Err(_) => {
                return Err(AppError::Other(format!(
                    "local branch {local} has diverged from {origin_label}:{branch}; \
                     resolve it by hand (git log {local}) or delete the branch and re-run"
                )))
            }
        }
    } else {
        git_success(&["switch", "-c", &local, "FETCH_HEAD"])?;
        output::line!("{repo}#{number} '{branch}' checked out as {local} (from {origin_label})");
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
