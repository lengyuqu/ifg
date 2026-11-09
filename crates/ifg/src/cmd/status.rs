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
            output::sanitize(&pr.title),
            pr.state,
            pr.head_branch,
            pr.base_branch,
            if pr.is_draft { " (draft)" } else { "" }
        ),
        None => println!("pull    (no open pull request for this branch)"),
    }

    // Pipelines have no server-side filter (`Ci::list`), so scan pages until
    // five runs for this branch are found. A single 20-row page used to hide
    // the branch whenever other branches were busier.
    const WANTED: usize = 5;
    const SCAN_LIMIT: usize = 500;
    let mut paginator = ctx.client.ci(&repo.owner, &repo.name).list();
    let mut runs: Vec<ifg_client::Pipeline> = Vec::new();
    let mut scanned = 0usize;
    while let Some(page) = paginator.next_page().await? {
        scanned += page.data.len();
        runs.extend(
            page.data
                .into_iter()
                .filter(|pipeline| pipeline.ref_name == branch),
        );
        if runs.len() >= WANTED || scanned >= SCAN_LIMIT {
            break;
        }
    }
    runs.truncate(WANTED);
    if runs.is_empty() {
        println!("runs    (no pipeline runs for this branch in the latest {scanned})");
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
