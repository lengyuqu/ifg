//! `ifg context …` — read-only AI aggregates: a repository summary, one page
//! of its file tree, code search on the default branch and a pull request
//! context. Each subcommand is one request. `--json` and `--template` pass the
//! server's object through unchanged; the default output is a compact
//! Markdown-style view in which every server-provided string is sanitized.

use ifg_client::{CodeSearchPage, CodeSearchQuery, TreeQuery};
use serde_json::Value;

use crate::cli::{Cli, ContextCommand};
use crate::cmd::{bootstrap, show};
use crate::context::{resolve_repo, RepoContext};
use crate::error::AppError;
use crate::output;

pub async fn run(command: &ContextCommand, cli: &Cli) -> Result<(), AppError> {
    let ctx = bootstrap(cli)?;
    let repo = resolve_repo(cli.repo.as_deref(), &ctx.host)?;
    let ai = ctx.client.ai();
    let structured = cli.json || cli.template.is_some();

    match command {
        ContextCommand::Repo => {
            let value = ai.summary(&repo.owner, &repo.name).await?;
            if structured {
                show(cli, &value);
            } else {
                print_summary(&repo, &value);
            }
        }
        ContextCommand::Tree {
            path,
            git_ref,
            depth,
            limit,
            cursor,
        } => {
            let query = TreeQuery {
                git_ref: git_ref.clone(),
                path: path.clone(),
                depth: *depth,
                limit: *limit,
                cursor: cursor.clone(),
            };
            let value = ai.tree(&repo.owner, &repo.name, &query).await?;
            if structured {
                show(cli, &value);
            } else {
                print_tree(&repo, &value);
            }
        }
        ContextCommand::Search {
            query,
            git_ref,
            limit,
            offset,
        } => {
            let query_args = CodeSearchQuery {
                q: query.clone(),
                git_ref: git_ref.clone(),
                limit: *limit,
                offset: *offset,
            };
            let page = ai.search_code(&repo.owner, &repo.name, &query_args).await?;
            if structured {
                // The search endpoint answers with an array; `show` renders
                // it per item for `--template`, as for other list commands.
                show(cli, &page.results);
            } else {
                print_search(&repo, query, &page)?;
            }
        }
        ContextCommand::Pr { number } => {
            let value = ai.pr_context(&repo.owner, &repo.name, *number).await?;
            if structured {
                show(cli, &value);
            } else {
                print_pr(&repo, *number, &value);
            }
        }
    }
    Ok(())
}

/// A field as sanitized text; absent and `null` fields give `None`.
fn field(value: &Value, key: &str) -> Option<String> {
    match value.get(key)? {
        Value::Null => None,
        other => Some(describe(other)),
    }
}

/// A JSON value as one line of sanitized text. Containers are counted, not
/// dumped, so the human view stays compact.
fn describe(value: &Value) -> String {
    match value {
        Value::Null => "none".to_string(),
        Value::String(text) => output::sanitize(text),
        Value::Array(items) => format!("{} item(s)", items.len()),
        Value::Object(fields) => format!("{} field(s)", fields.len()),
        other => output::sanitize(&other.to_string()),
    }
}

/// One bullet. Multi-line text keeps its line breaks, indented under the label.
fn print_item(label: &str, text: &str) {
    if text.contains('\n') {
        output::line!("- {label}:");
        for line in text.lines() {
            output::line!("  {line}");
        }
    } else {
        output::line!("- {label}: {text}");
    }
}

fn print_field(label: &str, value: &Value, key: &str) {
    if let Some(text) = field(value, key) {
        print_item(label, &text);
    }
}

fn print_summary(repo: &RepoContext, value: &Value) {
    let name = field(value, "full_name").unwrap_or_else(|| repo.to_string());
    output::line!("# {}", output::sanitize(&name));
    print_field("default branch", value, "default_branch");
    print_field("description", value, "description");
    print_field("stars", value, "stars_count");
    print_field("forks", value, "forks_count");
    print_field("created", value, "created_at");
    print_field("updated", value, "updated_at");
}

fn print_tree(repo: &RepoContext, value: &Value) {
    let git_ref = field(value, "ref").unwrap_or_default();
    let path = field(value, "path")
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| "/".to_string());
    output::line!("# {repo} @ {git_ref}:{path}");
    print_field("commit", value, "commit");

    let entries = value["entries"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    if entries.is_empty() {
        output::line!("(no entries)");
    }
    for entry in entries {
        let kind = field(entry, "type").unwrap_or_else(|| "?".to_string());
        let entry_path = field(entry, "path").unwrap_or_default();
        match field(entry, "size") {
            Some(size) => output::line!("- {kind} {entry_path} ({size} bytes)"),
            None => output::line!("- {kind} {entry_path}"),
        }
    }
    if value["truncated"].as_bool() == Some(true) {
        output::line!();
        match field(value, "next_cursor") {
            Some(cursor) => output::line!("More entries: rerun with --cursor {cursor}"),
            None => output::line!("More entries exist beyond this page."),
        }
    }
}

fn print_search(repo: &RepoContext, query: &str, page: &CodeSearchPage) -> Result<(), AppError> {
    let results = page
        .results
        .as_array()
        .ok_or_else(|| AppError::Other("invalid code search response: expected an array".into()))?;
    output::line!("# Code search \"{}\" in {repo}", output::sanitize(query));
    match page.total {
        Some(total) => output::line!("{} of {total} result(s)", results.len()),
        None => output::line!("{} result(s)", results.len()),
    }
    if results.is_empty() {
        output::line!("No matches.");
    }
    for hit in results {
        output::line!();
        let file = field(hit, "file_path")
            .or_else(|| field(hit, "file_name"))
            .unwrap_or_else(|| "?".to_string());
        match field(hit, "language") {
            Some(language) => output::line!("## {file} ({language})"),
            None => output::line!("## {file}"),
        }
        if let Some(snippet) = field(hit, "snippet") {
            for line in snippet.lines() {
                output::line!("    {line}");
            }
        }
    }
    Ok(())
}

/// The PR context schema is not declared in the OpenAPI snapshot, so the
/// human view lists the top-level fields the server sent instead of naming
/// fields that may not exist. `--json` gives the complete object.
fn print_pr(repo: &RepoContext, number: i64, value: &Value) {
    output::line!("# {repo} #{number}");
    let Some(object) = value.as_object() else {
        output::line!("{}", describe(value));
        return;
    };
    for (key, item) in object {
        print_item(&output::sanitize(key), &describe(item));
    }
}
