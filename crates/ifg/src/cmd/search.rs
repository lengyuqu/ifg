//! `ifg search <query>` — one table across repositories, issues and wiki
//! pages (the server returns a uniform result shape).

use crate::cli::Cli;
use crate::cmd::bootstrap;
use crate::error::AppError;
use crate::output;

const DEFAULT_LIMIT: u64 = 30;
/// The server caps `per_page`; asking for more does not return more.
const SERVER_PER_PAGE: u64 = 100;

pub async fn run(
    cli: &Cli,
    query: &str,
    search_type: &str,
    limit: Option<u64>,
) -> Result<(), AppError> {
    let ctx = bootstrap(cli)?;
    let limit = limit.unwrap_or(DEFAULT_LIMIT).max(1);
    // Fetch whole pages and stop at `limit`: clamping a single page to 100
    // used to make `--limit 200` silently return 100 hits.
    let per_page = limit.min(SERVER_PER_PAGE);

    let mut hits: Vec<ifg_client::SearchHit> = Vec::new();
    // Assigned on the first iteration, which always runs before the loop can
    // break, so the later read is definite.
    let mut total: u64;
    let mut page_no = 1u64;
    loop {
        let page = ctx
            .client
            .search()
            .query(query, search_type, page_no, per_page)
            .await?;
        total = page.total;
        let received = page.results.len();
        hits.extend(page.results);
        let short_page = received < per_page as usize;
        if received == 0 || short_page || hits.len() as u64 >= limit {
            break;
        }
        page_no += 1;
    }
    hits.truncate(limit as usize);

    if cli.json || cli.template.is_some() {
        let results: Vec<serde_json::Value> = hits
            .iter()
            .map(|hit| serde_json::to_value(hit).unwrap_or(serde_json::Value::Null))
            .collect();
        match &cli.template {
            Some(template) => output::print_template(&results, template),
            None => output::print_json(&serde_json::json!({
                "results": results,
                "total": total,
                "page": 1,
                "per_page": per_page,
            })),
        }
        return Ok(());
    }

    if hits.is_empty() {
        println!("No matches for '{query}'.");
        return Ok(());
    }
    let rows: Vec<Vec<String>> = hits
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
    eprintln!(
        "({} of {} match(es))",
        hits.len(),
        total.max(hits.len() as u64)
    );
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
