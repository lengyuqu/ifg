//! `ifg ci …` (design doc §4.5). Log streaming is polling (§7.1 option A):
//! the WebSocket job endpoint only accepts session JWTs, not PATs — the
//! CLI re-reads the job every 2s and prints the increment.

use serde_json::Value;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use super::bootstrap;
use crate::cli::{CiCommand, Cli};
use crate::context::{resolve_repo, RepoContext};
use crate::error::AppError;
use crate::output;

const DEFAULT_LIMIT: u64 = 30;
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

pub async fn run(command: &CiCommand, cli: &Cli) -> Result<(), AppError> {
    let ctx = bootstrap(cli)?;
    let repo = resolve_repo(cli.repo.as_deref(), &ctx.host)?;
    match command {
        CiCommand::Runs {
            branch,
            status,
            limit,
        } => runs(&ctx, cli, &repo, branch, status, *limit).await,
        CiCommand::View { id } => view(&ctx, cli, &repo, *id).await,
        CiCommand::Logs { id, job, follow } => logs(&ctx, &repo, *id, job, *follow).await,
        CiCommand::Cancel { id } => act(&ctx, &repo, *id, "cancel").await,
        CiCommand::Retry { id } => act(&ctx, &repo, *id, "retry").await,
        CiCommand::Approve { id, job } => approve(&ctx, &repo, *id, job).await,
    }
}

async fn runs(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    branch: &Option<String>,
    status: &Option<String>,
    limit: Option<u64>,
) -> Result<(), AppError> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    if cli.json || cli.template.is_some() {
        let items: Vec<Value> = ctx
            .client
            .ci(&repo.owner, &repo.name)
            .list_raw()
            .collect()
            .await?;
        let filtered: Vec<Value> = items
            .into_iter()
            .filter(|item| matches_filter(item, branch, status))
            .take(limit as usize)
            .collect();
        output::print_json(&Value::Array(filtered));
        return Ok(());
    }

    let pipelines = ctx
        .client
        .ci(&repo.owner, &repo.name)
        .list()
        .collect()
        .await?;
    let rows: Vec<Vec<String>> = pipelines
        .iter()
        .filter(|pipeline| {
            branch.as_deref().is_none_or(|b| pipeline.ref_name == b)
                && status.as_deref().is_none_or(|s| pipeline.status == s)
        })
        .take(limit as usize)
        .map(|pipeline| {
            vec![
                pipeline.id.to_string(),
                pipeline.status.clone(),
                pipeline.ref_name.clone(),
                short_sha(pipeline.commit_sha.as_deref()),
                pipeline.trigger_type.clone(),
                short_date(pipeline.created_at.as_deref()),
            ]
        })
        .collect();
    if rows.is_empty() {
        println!("No pipelines in {repo}.");
        return Ok(());
    }
    output::print_table(
        &["ID", "STATUS", "REF", "COMMIT", "TRIGGER", "CREATED"],
        &rows,
    );
    Ok(())
}

fn matches_filter(item: &Value, branch: &Option<String>, status: &Option<String>) -> bool {
    branch
        .as_deref()
        .is_none_or(|b| item["ref_name"].as_str() == Some(b))
        && status
            .as_deref()
            .is_none_or(|s| item["status"].as_str() == Some(s))
}

async fn view(
    ctx: &super::CommandContext,
    cli: &Cli,
    repo: &RepoContext,
    id: i64,
) -> Result<(), AppError> {
    let ci = ctx.client.ci(&repo.owner, &repo.name);
    if cli.json || cli.template.is_some() {
        let value = ci.get_raw(id).await?;
        match &cli.template {
            Some(template) => output::print_template(std::slice::from_ref(&value), template),
            None => output::print_json(&value),
        }
        return Ok(());
    }
    let detail = ci.get(id).await?;
    let pipeline = &detail.pipeline;
    let summary = vec![
        vec!["id".into(), pipeline.id.to_string()],
        vec!["status".into(), pipeline.status.clone()],
        vec!["ref".into(), pipeline.ref_name.clone()],
        vec![
            "commit".into(),
            pipeline.commit_sha.clone().unwrap_or_else(|| "—".into()),
        ],
        vec![
            "trigger".into(),
            format!(
                "{} ({})",
                pipeline.trigger_type,
                pipeline.triggered_by.clone().unwrap_or_else(|| "—".into())
            ),
        ],
        vec!["created".into(), short_date(pipeline.created_at.as_deref())],
    ];
    output::print_table(&["FIELD", "VALUE"], &summary);

    let rows: Vec<Vec<String>> = detail
        .stages
        .iter()
        .flat_map(|stage| {
            stage.jobs.iter().map(move |job| {
                vec![
                    stage.stage.name.clone(),
                    job.name.clone(),
                    job.status.clone(),
                    job.exit_code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "—".into()),
                    job.environment_name.clone().unwrap_or_else(|| "—".into()),
                ]
            })
        })
        .collect();
    if !rows.is_empty() {
        println!();
        output::print_table(&["STAGE", "JOB", "STATUS", "EXIT", "ENV"], &rows);
    }
    Ok(())
}

async fn logs(
    ctx: &super::CommandContext,
    repo: &RepoContext,
    id: i64,
    job: &Option<String>,
    follow: bool,
) -> Result<(), AppError> {
    let ci = ctx.client.ci(&repo.owner, &repo.name);
    let detail = ci.get(id).await?;
    let selected: ifg_client::Job = match job {
        Some(selector) => detail.find_job(selector).cloned().ok_or_else(|| {
            let names: Vec<String> = detail.jobs().iter().map(|j| j.name.clone()).collect();
            AppError::Usage(format!(
                "no job '{selector}' in pipeline {id}; jobs: {}",
                names.join(", ")
            ))
        })?,
        None => {
            let jobs = detail.jobs();
            if jobs.len() == 1 {
                jobs[0].clone()
            } else {
                let names: Vec<String> = jobs.iter().map(|j| j.name.clone()).collect();
                return Err(AppError::Usage(format!(
                    "pipeline {id} has several jobs; pass --job: {}",
                    names.join(", ")
                )));
            }
        }
    };
    eprintln!(
        "==> job {} ({}): {}",
        selected.id, selected.name, selected.status
    );

    // Initial print, then incremental polling (§7.1-A).
    let mut printed = print_log(&selected.log);
    if !follow || selected.is_terminal() {
        return Ok(());
    }
    if selected.is_blocked() {
        eprintln!("{}", blocked_hint(repo, id, &selected));
        return Ok(());
    }

    // Prefer the WebSocket stream (design doc §7.1-B): chunks arrive as they
    // are produced instead of re-fetching the whole log every 2s.
    let printed_lines = selected
        .log
        .as_deref()
        .map(|log| log.split('\n').count())
        .unwrap_or(0);
    if follow_websocket(ctx, repo, id, selected.id, printed_lines).await {
        eprintln!("==> stream closed (job finished)");
        return Ok(());
    }
    eprintln!("ifg: note: WebSocket streaming unavailable; falling back to polling");

    loop {
        tokio::time::sleep(POLL_INTERVAL).await;
        let job = ci.job(id, selected.id).await?;
        printed += print_increment(&job.log, printed);
        if job.is_terminal() {
            eprintln!("==> job finished: {}", job.status);
            return Ok(());
        }
        if job.is_blocked() {
            eprintln!("{}", blocked_hint(repo, id, &job));
            return Ok(());
        }
    }
}

/// One WS frame → the log chunk it carries, if any. The server sends
/// `{"event_type":"job_log","data":{"job_id":N,"log":"…"}}` and a `connected`
/// welcome frame that carries no log.
fn job_log_chunk(frame: &str) -> Option<String> {
    let value: Value = serde_json::from_str(frame).ok()?;
    if value["event_type"] != "job_log" {
        return None;
    }
    let chunk = value["data"]["log"].as_str()?;
    (!chunk.is_empty()).then(|| chunk.to_string())
}

/// `ws(s)://…/api/v1/ws/job/{id}?since=<lines>` — the server replays only the
/// log suffix the caller has not printed yet.
fn job_log_ws_url(host_url: &str, job_id: i64, since_lines: usize) -> String {
    let base = host_url
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    format!("{base}/api/v1/ws/job/{job_id}?since={since_lines}")
}

/// Stream a job log over `/api/v1/ws/job/{id}` until the job is terminal.
///
/// Returns `false` when streaming is not available (older server, refused
/// handshake, or a connection that dies before the job finishes) so the
/// caller can fall back to polling.
async fn follow_websocket(
    ctx: &super::CommandContext,
    repo: &RepoContext,
    pipeline_id: i64,
    job_id: i64,
    printed_lines: usize,
) -> bool {
    use futures_util::StreamExt;
    use std::io::Write as _;

    let url = job_log_ws_url(&ctx.host.url, job_id, printed_lines);
    let Ok(mut request) = url.as_str().into_client_request() else {
        return false;
    };
    // Ask for the suffix we have not printed yet (line-based `since`).
    if let Some(token) = &ctx.host.token {
        let header = format!("Bearer {}", token.expose());
        if let Ok(value) = tokio_tungstenite::tungstenite::http::HeaderValue::from_str(&header) {
            request.headers_mut().insert("authorization", value);
        }
    }
    let Ok((mut socket, _)) = tokio_tungstenite::connect_async(request).await else {
        return false;
    };

    let ci = ctx.client.ci(&repo.owner, &repo.name);
    let mut ticker = tokio::time::interval(POLL_INTERVAL);
    let mut finished = false;
    loop {
        tokio::select! {
            message = socket.next() => match message {
                Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                    if let Some(chunk) = job_log_chunk(&text) {
                        print!("{chunk}");
                        let _ = std::io::stdout().flush();
                    }
                }
                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            },
            _ = ticker.tick() => {
                match ci.job(pipeline_id, job_id).await {
                    Ok(job) if job.is_terminal() => {
                        // Give in-flight chunks a moment to arrive.
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                        finished = true;
                        break;
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        }
    }
    let _ = socket.close(None).await;
    finished
}

/// Why `--follow` stops on a job that cannot progress by itself, and how to
/// unblock it.
fn blocked_hint(repo: &RepoContext, pipeline: i64, job: &ifg_client::Job) -> String {
    let action = if job.status == "waiting_approval" {
        format!("approve it with `ifg ci approve {pipeline} {}`", job.id)
    } else {
        format!(
            "start it with `ifg api -X POST repos/{}/{}/pipelines/{pipeline}/jobs/{}/play`",
            repo.owner, repo.name, job.id
        )
    };
    format!(
        "==> job is {}; it will not progress on its own: {action}",
        job.status
    )
}

/// Print the whole log; returns how many chars were printed.
fn print_log(log: &Option<String>) -> usize {
    match log {
        Some(log) if !log.is_empty() => {
            print!("{log}");
            use std::io::Write;
            let _ = std::io::stdout().flush();
            log.chars().count()
        }
        _ => 0,
    }
}

/// Print the part of `log` beyond `printed` chars; returns that length.
fn print_increment(log: &Option<String>, printed: usize) -> usize {
    let Some(log) = log else { return 0 };
    let rest: String = log.chars().skip(printed).collect();
    if rest.is_empty() {
        return 0;
    }
    print!("{rest}");
    use std::io::Write;
    let _ = std::io::stdout().flush();
    rest.chars().count()
}

async fn act(
    ctx: &super::CommandContext,
    repo: &RepoContext,
    id: i64,
    action: &str,
) -> Result<(), AppError> {
    let ci = ctx.client.ci(&repo.owner, &repo.name);
    let value = match action {
        "cancel" => ci.cancel(id).await?,
        "retry" => ci.retry(id).await?,
        _ => unreachable!("action routing is exhaustive"),
    };
    println!("{action} requested for pipeline {id}.");
    output::print_json(&value);
    Ok(())
}

async fn approve(
    ctx: &super::CommandContext,
    repo: &RepoContext,
    id: i64,
    job: &str,
) -> Result<(), AppError> {
    let ci = ctx.client.ci(&repo.owner, &repo.name);
    let detail = ci.get(id).await?;
    let selected = detail
        .find_job(job)
        .ok_or_else(|| AppError::Usage(format!("no job '{job}' in pipeline {id}")))?;
    let value = ci.approve_job(id, selected.id).await?;
    println!("Approved job {} in pipeline {id}.", selected.name);
    output::print_json(&value);
    Ok(())
}

fn short_sha(sha: Option<&str>) -> String {
    match sha {
        Some(sha) => sha.chars().take(12).collect(),
        None => "—".to_string(),
    }
}

fn short_date(iso: Option<&str>) -> String {
    crate::output::human_time(iso)
}

#[cfg(test)]
mod tests {
    use super::{job_log_chunk, job_log_ws_url};

    #[test]
    fn job_log_frames_are_parsed_and_filtered() {
        assert_eq!(
            job_log_chunk(r#"{"event_type":"job_log","data":{"job_id":3,"log":"step 1\n"}}"#)
                .as_deref(),
            Some("step 1\n")
        );
        // The welcome frame carries no log.
        assert_eq!(job_log_chunk(r#"{"type":"connected","job_id":3}"#), None);
        // Empty chunks are dropped, malformed frames are ignored.
        assert_eq!(
            job_log_chunk(r#"{"event_type":"job_log","data":{"log":""}}"#),
            None
        );
        assert_eq!(job_log_chunk("not json"), None);
    }

    #[test]
    fn job_log_ws_url_maps_schemes() {
        assert_eq!(
            job_log_ws_url("http://127.0.0.1:8080", 7, 3),
            "ws://127.0.0.1:8080/api/v1/ws/job/7?since=3"
        );
        assert_eq!(
            job_log_ws_url("https://forge.example.com", 1, 0),
            "wss://forge.example.com/api/v1/ws/job/1?since=0"
        );
    }
}
