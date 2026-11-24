//! `ifg commit …`: commits written by the server, without a checkout.
//!
//! Every subcommand makes exactly one write request, pinned to the commit the
//! branch must still be at. Machine output (`--json`, `--template`) is the
//! server's response as sent; the human output is one summary line plus the
//! files touched. A dry run prints the server's preview as JSON, because its
//! shape differs from the real answer and carries what was not evaluated.

use ifg_client::{PickKind, PickRequest};
use serde_json::Value;

use super::bootstrap;
use crate::cli::{Cli, CommitCommand, CommitTarget};
use crate::context::resolve_repo;
use crate::error::AppError;
use crate::output;

/// Largest file `--files` / `--patch-file` will read. The server caps a patch
/// at 1 MiB; this bound only stops an accidental multi-gigabyte read.
const MAX_INPUT_BYTES: u64 = 16 * 1024 * 1024;

pub async fn run(command: &CommitCommand, cli: &Cli) -> Result<(), AppError> {
    // Read local input before bootstrap: a bad file is a usage error that must
    // not wait on config or the network.
    let input = match command {
        CommitCommand::Create { files, .. } => Some(Input::Files(read_files(files)?)),
        CommitCommand::ApplyPatch { patch_file, .. } => {
            Some(Input::Patch(read_source(patch_file, "patch")?))
        }
        _ => None,
    };
    let ctx = bootstrap(cli)?;
    let repo = resolve_repo(cli.repo.as_deref(), &ctx.host)?;
    let commits = ctx.client.commits(&repo.owner, &repo.name);
    let (value, target, kind) = match (command, input) {
        (
            CommitCommand::Create {
                target, message, ..
            },
            Some(Input::Files(files)),
        ) => (
            commits
                .batch_raw(
                    &target.branch,
                    &target.expected_sha,
                    message,
                    &files,
                    target.dry_run,
                )
                .await?,
            target,
            Kind::Batch,
        ),
        (
            CommitCommand::ApplyPatch {
                target, message, ..
            },
            Some(Input::Patch(patch)),
        ) => (
            commits
                .apply_patch_raw(
                    &target.branch,
                    &target.expected_sha,
                    &patch,
                    message.as_deref(),
                    target.dry_run,
                )
                .await?,
            target,
            Kind::Patch,
        ),
        (
            CommitCommand::CherryPick {
                sha,
                target,
                mainline,
                message,
            },
            _,
        ) => (
            pick(
                &commits,
                PickKind::CherryPick,
                sha,
                target,
                *mainline,
                message,
            )
            .await?,
            target,
            Kind::Pick(PickKind::CherryPick),
        ),
        (
            CommitCommand::Revert {
                sha,
                target,
                mainline,
                message,
            },
            _,
        ) => (
            pick(&commits, PickKind::Revert, sha, target, *mainline, message).await?,
            target,
            Kind::Pick(PickKind::Revert),
        ),
        _ => unreachable!("input is read for exactly the commands that need it"),
    };
    report(cli, &value, target, kind);
    Ok(())
}

enum Input {
    Files(Value),
    Patch(String),
}

#[derive(Clone, Copy)]
enum Kind {
    Batch,
    Patch,
    Pick(PickKind),
}

async fn pick(
    commits: &ifg_client::Commits<'_>,
    kind: PickKind,
    sha: &str,
    target: &CommitTarget,
    mainline: Option<u32>,
    message: &Option<String>,
) -> Result<Value, ifg_client::Error> {
    commits
        .pick_raw(&PickRequest {
            kind,
            sha,
            branch: &target.branch,
            expected_sha: &target.expected_sha,
            mainline,
            message: message.as_deref(),
            dry_run: target.dry_run,
        })
        .await
}

fn report(cli: &Cli, value: &Value, target: &CommitTarget, kind: Kind) {
    if cli.json || cli.template.is_some() {
        super::show(cli, value);
        return;
    }
    if value["dry_run"].as_bool().unwrap_or(target.dry_run) {
        output::line!("Dry run: nothing was written. The server would do this:");
        output::print_json(value);
        return;
    }
    let branch = text(value, "branch", &target.branch);
    let new = short(value, "commit_sha");
    let was = short(value, "tip_before");
    match kind {
        Kind::Batch => {
            let files = value["files"].as_array().map_or(0, Vec::len);
            output::line!("Committed {files} file(s) to {branch}: {new} (was {was})");
            for file in value["files"].as_array().into_iter().flatten() {
                let path = text(file, "path", "?");
                let op = text(file, "operation", "?");
                match file["from"].as_str() {
                    Some(from) => output::line!("  {op}  {} -> {path}", output::sanitize(from)),
                    None => output::line!("  {op}  {path}"),
                }
            }
        }
        Kind::Patch => {
            let files = value["files_changed"].as_u64().unwrap_or(0);
            output::line!("Committed {files} file(s) to {branch}: {new} (was {was})");
            for change in value["changes"].as_array().into_iter().flatten() {
                output::line!(
                    "  {}  {}",
                    text(change, "status", "?"),
                    text(change, "path", "?")
                );
            }
        }
        Kind::Pick(kind) => {
            let verb = match kind {
                PickKind::CherryPick => "Cherry-picked",
                PickKind::Revert => "Reverted",
            };
            let source = short(value, "source_sha");
            output::line!("{verb} {source} onto {branch}: {new} (was {was})");
        }
    }
}

/// A server string, sanitized for the terminal; `fallback` when absent.
fn text(value: &Value, key: &str, fallback: &str) -> String {
    output::sanitize(value[key].as_str().unwrap_or(fallback))
}

/// The first 12 characters of a SHA field, or `—` when it is absent.
fn short(value: &Value, key: &str) -> String {
    match value[key].as_str() {
        Some(sha) => output::sanitize(&sha.chars().take(12).collect::<String>()),
        None => "—".to_string(),
    }
}

/// Read a file, or stdin for `-`. `what` names the input in errors.
fn read_source(source: &str, what: &str) -> Result<String, AppError> {
    use std::io::Read;
    let mut text = String::new();
    if source == "-" {
        std::io::stdin()
            .take(MAX_INPUT_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(|e| AppError::Io(format!("cannot read stdin: {e}")))?;
    } else {
        std::fs::File::open(source)
            .and_then(|file| file.take(MAX_INPUT_BYTES + 1).read_to_string(&mut text))
            .map_err(|e| AppError::Io(format!("cannot read {source}: {e}")))?;
    }
    if text.len() as u64 > MAX_INPUT_BYTES {
        return Err(AppError::Usage(format!(
            "the {what} input is larger than {} MiB",
            MAX_INPUT_BYTES / (1024 * 1024)
        )));
    }
    Ok(super::strip_bom(&text).to_string())
}

/// Read and check the `--files` document: a non-empty JSON array of objects.
/// Everything else (operation names, shas, paths) is the server's to judge.
fn read_files(source: &str) -> Result<Value, AppError> {
    let text = read_source(source, "files")?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| AppError::Usage(format!("the files input is not valid JSON: {e}")))?;
    check_files(&value)?;
    Ok(value)
}

fn check_files(value: &Value) -> Result<(), AppError> {
    let Some(items) = value.as_array() else {
        return Err(AppError::Usage(
            "the files input must be a JSON array of operations".into(),
        ));
    };
    if items.is_empty() {
        return Err(AppError::Usage(
            "the files input has no operations; a commit needs at least one".into(),
        ));
    }
    if let Some(index) = items.iter().position(|item| !item.is_object()) {
        return Err(AppError::Usage(format!(
            "operation {index} of the files input is not a JSON object"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn files_must_be_a_non_empty_array_of_objects() {
        assert!(check_files(&json!([{"operation": "create"}])).is_ok());
        assert!(check_files(&json!({"files": []})).is_err());
        assert!(check_files(&json!([])).is_err());
        assert!(check_files(&json!([1])).is_err());
        assert!(check_files(&json!("x")).is_err());
    }

    #[test]
    fn short_sha_is_sanitized_and_cut() {
        let value = json!({"a": "0123456789abcdef0123", "b": "\u{1b}[2Jabc"});
        assert_eq!(short(&value, "a"), "0123456789ab");
        assert!(!short(&value, "b").contains('\u{1b}'));
        assert_eq!(short(&value, "missing"), "—");
    }

    #[test]
    fn files_input_with_utf8_bom_parses() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all("\u{FEFF}[{\"operation\":\"create\"}]".as_bytes())
            .unwrap();
        let value = read_files(file.path().to_str().unwrap()).unwrap();
        assert_eq!(value.as_array().unwrap().len(), 1);
    }
}
