//! `ifg status` — where am I: the current branch's pull request and the
//! latest pipeline runs for it. Pure aggregation of endpoints the other
//! commands already use.

use crate::cli::Cli;
use crate::cmd::bootstrap;
use crate::context::resolve_repo;
use crate::error::AppError;
use crate::output;

pub async fn run(cli: &Cli) -> Result<(), AppError> {
    let ctx = bootstrap(cli)?;
    let repo = resolve_repo(cli.repo.as_deref(), &ctx.host)?;
    let branch = super::pr::current_branch()?;

    // Open PRs whose head is this branch (fork PRs have another head repo,
    // so a straight name match is right for the local checkout case).
    let pulls = ctx.client.pulls(&repo.owner, &repo.name);
    let prs = pulls.list(Some("open")).collect().await?;
    let matching = prs
        .iter()
        .find(|pr| pr.head_branch == branch && pr.head_repo_id.is_none());

    println!("{repo}");
    println!("branch  {branch}");
    match matching {
        Some(pr) => println!(
            "pull    #{:<4} {} [{}] {} → {}{}",
            pr.number,
            pr.title,
            pr.state,
            pr.head_branch,
            pr.base_branch,
            if pr.is_draft { " (draft)" } else { "" }
        ),
        None => println!("pull    (no open pull request for this branch)"),
    }

    let pipelines = ctx
        .client
        .ci(&repo.owner, &repo.name)
        .list()
        .limit(20)
        .collect()
        .await?;
    let runs: Vec<&ifg_client::Pipeline> = pipelines
        .iter()
        .filter(|pipeline| pipeline.ref_name == branch)
        .take(5)
        .collect();
    if runs.is_empty() {
        println!("runs    (no pipeline runs for this branch)");
        return Ok(());
    }
    println!();
    let rows: Vec<Vec<String>> = runs
        .iter()
        .map(|pipeline| {
            vec![
                pipeline.id.to_string(),
                pipeline.status.clone(),
                pipeline.trigger_type.clone(),
                output::human_time(pipeline.created_at.as_deref()),
            ]
        })
        .collect();
    output::print_table(&["RUN", "STATUS", "TRIGGER", "CREATED"], &rows);
    Ok(())
}
