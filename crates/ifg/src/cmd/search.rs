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

    let mut hits: Vec<serde_json::Value> = Vec::new();
    let mut envelope = None;
    // Assigned on the first iteration, which always runs before the loop can
    // break, so the later read is definite.
    let mut total: u64;
    let mut page_no = 1u64;
    loop {
        let page = ctx
            .client
            .search()
            .query_raw(query, search_type, page_no, per_page)
            .await?;
        total = page["total"].as_u64().unwrap_or(0);
        let results = page["results"].as_array().ok_or_else(|| {
            AppError::Other("invalid search response: results must be an array".into())
        })?;
        let received = results.len();
        hits.extend(results.iter().cloned());
        if envelope.is_none() {
            envelope = Some(page.clone());
        }
        let short_page = received < per_page as usize;
        if received == 0 || short_page || hits.len() as u64 >= limit {
            break;
        }
        page_no += 1;
    }
    hits.truncate(limit as usize);

    if cli.json || cli.template.is_some() {
        match &cli.template {
            Some(template) => output::print_template(&hits, template),
            None => {
                let mut value = envelope.expect("the first search page was fetched");
                value["results"] = serde_json::Value::Array(hits);
                if value.get("total").is_some() {
                    value["total"] = serde_json::json!(total);
                }
                output::print_json(&value);
            }
        }
        return Ok(());
    }

    if hits.is_empty() {
        output::line!("No matches for '{query}'.");
        return Ok(());
    }
    let hits: Vec<ifg_client::SearchHit> = hits
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<_, _>>()
        .map_err(|error| AppError::Other(format!("could not decode search result: {error}")))?;
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
