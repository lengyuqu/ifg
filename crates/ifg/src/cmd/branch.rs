//! `ifg branch …`: repository branches.

use super::bootstrap;
use crate::cli::{BranchCommand, Cli};
use crate::context::resolve_repo;
use crate::error::AppError;
use crate::output;

pub async fn run(command: &BranchCommand, cli: &Cli) -> Result<(), AppError> {
    let BranchCommand::Delete {
        name,
        expected_sha,
        dry_run,
        yes,
    } = command;
    let ctx = bootstrap(cli)?;
    let repo = resolve_repo(cli.repo.as_deref(), &ctx.host)?;
    // A dry run deletes nothing, so there is nothing to confirm.
    if !*dry_run
        && !*yes
        && !super::confirm(&format!(
            "Delete branch '{}' from {repo}?",
            output::sanitize(name)
        ))?
    {
        output::line!("Aborted.");
        return Ok(());
    }
    let value = ctx
        .client
        .branches(&repo.owner, &repo.name)
        .delete_raw(name, expected_sha.as_deref(), *dry_run)
        .await?;
    if cli.json || cli.template.is_some() {
        super::show(cli, &value);
        return Ok(());
    }
    if *dry_run {
        output::line!("Dry run: nothing was deleted. The server would do this:");
        output::print_json(&value);
        return Ok(());
    }
    let sha = value["sha"]
        .as_str()
        .map(|sha| output::sanitize(&sha.chars().take(12).collect::<String>()));
    match sha {
        Some(sha) => output::line!("Deleted branch {} ({sha}).", output::sanitize(name)),
        None => output::line!("Deleted branch {}.", output::sanitize(name)),
    }
    Ok(())
}
