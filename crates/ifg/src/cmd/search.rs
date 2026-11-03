//! `ifg search <query>` — one table across repositories, issues and wiki
//! pages (the server returns a uniform result shape).

use crate::cli::Cli;
use crate::cmd::bootstrap;
use crate::error::AppError;
use crate::output;

const DEFAULT_LIMIT: u64 = 30;

pub async fn run(
    cli: &Cli,
    query: &str,
    search_type: &str,
    limit: Option<u64>,
) -> Result<(), AppError> {
    let ctx = bootstrap(cli)?;
    let per_page = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 100);

    if cli.json || cli.template.is_some() {
        let value = ctx
            .client
            .search()
            .query_raw(query, search_type, 1, per_page)
            .await?;
        let hits = value["results"].clone();
        match &cli.template {
            Some(template) => output::print_template(
                hits.as_array().cloned().unwrap_or_default().as_slice(),
                template,
            ),
            None => output::print_json(&value),
        }
        return Ok(());
    }

    let page = ctx
        .client
        .search()
        .query(query, search_type, 1, per_page)
        .await?;
    if page.results.is_empty() {
        println!("No matches for '{query}'.");
        return Ok(());
    }
    let rows: Vec<Vec<String>> = page
        .results
        .iter()
        .map(|hit| {
            vec![
                hit.result_type.clone(),
                hit.repo_label(),
                match (&hit.number, &hit.state) {
                    (Some(number), Some(state)) => format!("#{number} [{state}]"),
                    (Some(number), None) => format!("#{number}"),
                    _ => "—".to_string(),
                },
                truncate(&hit.title, 60),
            ]
        })
        .collect();
    output::print_table(&["TYPE", "REPO", "NUMBER", "TITLE"], &rows);
    eprintln!("({} of {} match(es))", page.results.len(), page.total);
    Ok(())
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}
