//! `ifg context …` — AI aggregates: a repository summary, one page
//! of its file tree, a ranked code map, symbol lookup, code search on the
//! default branch, a pull request context and the issue and pull request
//! summary lists, plus `index`, the one write: it rebuilds the code search
//! and symbol index. Each subcommand is one request. `--json` and `--template` pass the server's object through
//! unchanged; the default output is a compact Markdown-style view in which
//! every server-provided string is sanitized.

use ifg_client::{
    AiListQuery, CodeSearchPage, CodeSearchQuery, MapQuery, PrContextQuery, SymbolsQuery, TreeQuery,
};
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
        ContextCommand::Map {
            path,
            git_ref,
            budget,
        } => {
            let query = MapQuery {
                git_ref: git_ref.clone(),
                path: path.clone(),
                budget: *budget,
            };
            let value = ai.map(&repo.owner, &repo.name, &query).await?;
            if structured {
                show(cli, &value);
            } else {
                print_map(&repo, &value);
            }
        }
        ContextCommand::Symbols {
            name,
            path,
            git_ref,
            commit,
            limit,
        } => {
            let query = SymbolsQuery {
                name: name.clone(),
                path: path.clone(),
                git_ref: git_ref.clone(),
                commit_sha: commit.clone(),
                limit: *limit,
            };
            let value = ai.symbols(&repo.owner, &repo.name, &query).await?;
            if structured {
                show(cli, &value);
            } else {
                print_symbols(&repo, &value);
            }
        }
        ContextCommand::Index { commit } => {
            let value = ai.index(&repo.owner, &repo.name, commit.as_deref()).await?;
            if structured {
                show(cli, &value);
            } else {
                print_index(&repo, &value);
            }
        }
        ContextCommand::Pr { number, budget } => {
            let query = PrContextQuery { budget: *budget };
            let value = ai
                .pr_context_with(&repo.owner, &repo.name, *number, &query)
                .await?;
            if structured {
                show(cli, &value);
            } else {
                print_pr(&repo, *number, &value);
            }
        }
        ContextCommand::Issues { state, limit } => {
            let query = AiListQuery {
                state: state.clone(),
                limit: *limit,
            };
            let value = ai.issues(&repo.owner, &repo.name, &query).await?;
            if structured {
                show(cli, &value);
            } else {
                print_issues(&repo, &value)?;
            }
        }
        ContextCommand::Prs { state, limit } => {
            let query = AiListQuery {
                state: state.clone(),
                limit: *limit,
            };
            let value = ai.prs(&repo.owner, &repo.name, &query).await?;
            if structured {
                show(cli, &value);
            } else {
                print_prs(&repo, &value)?;
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

/// The first 12 characters of a SHA field, sanitized; `none` when absent.
fn short_sha(value: &Value, key: &str) -> String {
    match value.get(key).and_then(Value::as_str) {
        Some(sha) => output::sanitize(&sha.chars().take(12).collect::<String>()),
        None => "none".to_string(),
    }
}

/// The repository map: one line per directory, then each file with its
/// declarations indented by nesting, then what the map left out.
fn print_map(repo: &RepoContext, value: &Value) {
    let git_ref = field(value, "ref").unwrap_or_default();
    output::line!("# {repo} @ {git_ref} ({})", short_sha(value, "commit_sha"));
    let tokens = field(value, "estimated_tokens").unwrap_or_else(|| "?".into());
    let budget = field(value, "budget").unwrap_or_else(|| "?".into());
    output::line!("{tokens} of {budget} tokens");
    if value["truncated"].as_bool() == Some(true) {
        output::line!(
            "Trimmed to fit the budget: rerun with a larger --budget or a narrower path."
        );
    }

    let directories = value["directories"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    if !directories.is_empty() {
        output::line!();
        output::line!("## Directories");
        for dir in directories {
            let path = field(dir, "path")
                .filter(|path| !path.is_empty())
                .unwrap_or_else(|| "/".to_string());
            let languages: Vec<String> = dir["languages"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(output::sanitize)
                .collect();
            output::line!(
                "- {path}: {} file(s) [{}]",
                field(dir, "files").unwrap_or_else(|| "?".into()),
                languages.join(", ")
            );
        }
    }

    let files = value["files"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    if files.is_empty() {
        output::line!();
        output::line!("No source files mapped.");
    }
    for file in files {
        output::line!();
        let path = field(file, "path").unwrap_or_else(|| "?".into());
        match field(file, "language") {
            Some(language) => output::line!("## {path} ({language})"),
            None => output::line!("## {path}"),
        }
        if let Some(notice) = field(file, "notice") {
            output::line!(
                "  note: {}",
                notice.split_whitespace().collect::<Vec<_>>().join(" ")
            );
        }
        print_map_items(
            file["items"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default(),
            1,
        );
        if file["items_omitted"].as_bool() == Some(true) {
            output::line!("  … more declarations omitted for the budget");
        }
    }

    let skipped = value["skipped"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    if !skipped.is_empty() {
        output::line!();
        output::line!("## Skipped ({})", skipped.len());
        for item in skipped {
            output::line!(
                "- {} ({})",
                field(item, "path").unwrap_or_else(|| "?".into()),
                field(item, "reason").unwrap_or_else(|| "?".into())
            );
        }
    }
}

fn print_map_items(items: &[Value], depth: usize) {
    for item in items {
        // The signature already starts with the declaration keyword.
        let signature = field(item, "signature")
            .or_else(|| field(item, "name"))
            .unwrap_or_else(|| "?".into());
        output::line!(
            "{}{}  (L{}-{})",
            "  ".repeat(depth),
            signature.split_whitespace().collect::<Vec<_>>().join(" "),
            field(item, "start_line").unwrap_or_else(|| "?".into()),
            field(item, "end_line").unwrap_or_else(|| "?".into()),
        );
        print_map_items(
            item["children"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_default(),
            depth + 1,
        );
    }
}

/// Symbol rows with their match confidence, then the caveats the server
/// attaches: an incomplete index and what static parsing cannot see.
fn print_symbols(repo: &RepoContext, value: &Value) {
    output::line!("# Symbols in {repo} @ {}", short_sha(value, "commit_sha"));
    if value["partial"].as_bool() == Some(true) {
        output::line!("Partial index: a missing symbol is not proof it does not exist.");
    }
    let items = value["items"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    if items.is_empty() {
        output::line!("No matching definitions.");
    }
    for item in items {
        let name = field(item, "name").unwrap_or_else(|| "?".into());
        let kind = field(item, "kind").unwrap_or_else(|| "?".into());
        let path = field(item, "path").unwrap_or_else(|| "?".into());
        let lines = format!(
            "L{}-{}",
            field(item, "start_line").unwrap_or_else(|| "?".into()),
            field(item, "end_line").unwrap_or_else(|| "?".into())
        );
        // An exact hit is the answer; a name_match is only a candidate.
        let mark = match item["confidence"].as_str() {
            Some("exact") => String::new(),
            Some(other) => format!(" [{}]", output::sanitize(other)),
            None => " [?]".to_string(),
        };
        output::line!("- {kind} {name}  {path}:{lines}{mark}");
        if let Some(signature) = field(item, "signature") {
            output::line!(
                "    {}",
                signature.split_whitespace().collect::<Vec<_>>().join(" ")
            );
        }
    }
    // The caveats repeat on every answer, so a row list gets one pointer;
    // an empty answer is where they matter, so it gets the full text.
    let limitations = value["limitations"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    if items.is_empty() && !limitations.is_empty() {
        output::line!();
        output::line!("Limitations:");
        for text in limitations.iter().filter_map(Value::as_str) {
            output::line!("- {}", output::sanitize(text));
        }
    } else if !limitations.is_empty() {
        output::line!();
        output::line!("Static parsing misses some symbols; see \"limitations\" in --json.");
    }
}

/// The index report: the commit both passes were built from and the counts.
fn print_index(repo: &RepoContext, value: &Value) {
    output::line!("Indexed {repo} at {}.", short_sha(value, "commit_sha"));
    print_field("files in code search", value, "indexed_files");
    print_field("definitions written", value, "definitions_written");
    print_field("old commits pruned", value, "pruned_commits");
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

/// A summary field on one line: sanitized, with line breaks and tabs turned
/// into spaces so each item stays on one line. A missing field shows `?`.
fn inline(value: &Value, key: &str) -> String {
    field(value, key)
        .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|| "?".to_string())
}

/// One line per issue, with the fields `IssueSummary` declares.
fn issue_line(item: &Value) -> String {
    format!(
        "- #{} {} ({}, author {}, created {})",
        inline(item, "number"),
        inline(item, "title"),
        inline(item, "state"),
        inline(item, "author_id"),
        inline(item, "created_at"),
    )
}

/// One line per pull request, with the fields `PrSummary` declares.
fn pr_line(item: &Value) -> String {
    format!(
        "- #{} {} ({}, {} -> {}, author {}, created {})",
        inline(item, "number"),
        inline(item, "title"),
        inline(item, "state"),
        inline(item, "head_branch"),
        inline(item, "base_branch"),
        inline(item, "author_id"),
        inline(item, "created_at"),
    )
}

/// The server's summary array as a Markdown list under a `# repo kind` heading.
fn print_summaries(
    repo: &RepoContext,
    kind: &str,
    value: &Value,
    line: fn(&Value) -> String,
) -> Result<(), AppError> {
    let items = value
        .as_array()
        .ok_or_else(|| AppError::Other(format!("invalid {kind} response: expected an array")))?;
    output::line!("# {repo} {kind}");
    if items.is_empty() {
        output::line!("No {kind}.");
    }
    for item in items {
        output::line!("{}", line(item));
    }
    Ok(())
}

fn print_issues(repo: &RepoContext, value: &Value) -> Result<(), AppError> {
    print_summaries(repo, "issues", value, issue_line)
}

fn print_prs(repo: &RepoContext, value: &Value) -> Result<(), AppError> {
    print_summaries(repo, "pull requests", value, pr_line)
}
