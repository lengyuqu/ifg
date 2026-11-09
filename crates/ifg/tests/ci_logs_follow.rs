//! `ifg ci logs --follow` end to end: a server that speaks both the REST API
//! and the job-log WebSocket on one port.
//!
//! Covers the two tail problems: a last frame that arrives after the quiet
//! window (reconciled from `job.log`), and the `existing + "\n" + chunk`
//! separator the cursor must account for.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use assert_cmd::Command;
use serde_json::json;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone)]
struct Script {
    /// `(chunk, delay before sending)`
    frames: Vec<(String, u64)>,
    close_after: bool,
}

fn log_frame(chunk: &str) -> tokio_tungstenite::tungstenite::Message {
    tokio_tungstenite::tungstenite::Message::Text(
        json!({"event_type": "job_log", "data": {"log": chunk}})
            .to_string()
            .into(),
    )
}

fn http_response(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn pipeline_body(status: &str, log: &str) -> String {
    json!({
        "pipeline": {
            "id": 1, "repo_id": 1, "ref_name": "main",
            "status": status, "trigger_type": "manual"
        },
        "stages": [{
            "stage": {"id": 1, "pipeline_id": 1, "name": "test", "stage_order": 0, "status": status},
            "jobs": [{"id": 7, "stage_id": 1, "name": "test", "status": status, "log": log}]
        }]
    })
    .to_string()
}

fn job_body(status: &str, log: &str) -> String {
    json!({"id": 7, "stage_id": 1, "name": "test", "status": status, "log": log}).to_string()
}

/// Serve REST + WebSocket on one port. `initial` is what the pipeline detail
/// reports, `final_log` is what every later job read returns.
async fn spawn_server(
    initial: &'static str,
    final_log: &'static str,
    scripts: Vec<Script>,
) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let scripts = Arc::new(scripts);
    let connections = Arc::new(AtomicUsize::new(0));

    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let scripts = scripts.clone();
            let connections = connections.clone();
            tokio::spawn(async move {
                // Peek, never consume: `accept_async` has to read the
                // handshake itself. Reading it here would leave both sides
                // waiting forever.
                let mut probe = vec![0u8; 8192];
                let mut head = String::new();
                for _ in 0..20 {
                    match stream.peek(&mut probe).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            head = String::from_utf8_lossy(&probe[..n]).to_string();
                            if head.contains("\r\n\r\n") {
                                break;
                            }
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                let request_line = head.lines().next().unwrap_or_default().to_string();

                if head.to_ascii_lowercase().contains("upgrade: websocket") {
                    let index = connections.fetch_add(1, Ordering::SeqCst);
                    let script = scripts.get(index).or_else(|| scripts.last()).cloned();
                    let mut ws = match tokio_tungstenite::accept_async(stream).await {
                        Ok(ws) => ws,
                        Err(_) => return,
                    };
                    if let Some(script) = script {
                        for (chunk, delay) in script.frames {
                            if delay > 0 {
                                tokio::time::sleep(Duration::from_millis(delay)).await;
                            }
                            if futures_util::SinkExt::send(&mut ws, log_frame(&chunk))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        if script.close_after {
                            let _ = ws.close(None).await;
                            return;
                        }
                    } else {
                        // Match IronForge's initial catch-up: `since` counts
                        // lines already printed, and zero replays everything.
                        let target = request_line.split_whitespace().nth(1).unwrap();
                        let parsed = url::Url::parse(&format!("http://localhost{target}")).unwrap();
                        let since = parsed
                            .query_pairs()
                            .find(|(key, _)| key == "since")
                            .unwrap()
                            .1
                            .parse::<usize>()
                            .unwrap();
                        let catchup = final_log
                            .split('\n')
                            .skip(since)
                            .collect::<Vec<_>>()
                            .join("\n");
                        if !catchup.is_empty() {
                            let _ = futures_util::SinkExt::send(&mut ws, log_frame(&catchup)).await;
                        }
                    }
                    // Keep the socket open and silent so the drain has to time
                    // out on its own.
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    return;
                }

                // Consume the request now that it is known to be plain HTTP.
                let mut buf = vec![0u8; 8192];
                if matches!(stream.read(&mut buf).await, Ok(0) | Err(_)) {
                    return;
                }
                let body = if request_line.contains("/jobs/7") {
                    job_body("success", final_log)
                } else if request_line.contains("/pipelines/1") {
                    pipeline_body("running", initial)
                } else {
                    "{}".to_string()
                };
                let _ = stream.write_all(http_response(&body).as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });

    format!("http://{addr}")
}

/// Run the CLI on a blocking thread: `#[tokio::test]` uses a current-thread
/// runtime, and blocking it would starve the in-process server.
async fn run_logs(url: String, home: TempDir) -> std::process::Output {
    tokio::task::spawn_blocking(move || {
        command(&url, &home).assert().success().get_output().clone()
    })
    .await
    .expect("blocking task")
}

fn command(url: &str, home: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("ifg").unwrap();
    cmd.env("IRONFORGE_URL", url)
        .env("IRONFORGE_PAT", "ifp_test")
        .env("XDG_CONFIG_HOME", home.path())
        .args(["-R", "a/r", "ci", "logs", "1", "--follow"]);
    cmd
}

/// The terminal status is polled before the last frame arrives (1.2 s later).
/// The quiet window cannot prove completeness, so the final `job.log` is
/// consulted and `omega` is still printed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn last_frame_after_the_quiet_window_is_recovered() {
    let url = spawn_server(
        "alpha",
        "alpha\nbeta\nomega",
        vec![Script {
            frames: vec![("beta".to_string(), 0), ("omega".to_string(), 3200)],
            close_after: false,
        }],
    )
    .await;
    let home = TempDir::new().unwrap();

    let output = run_logs(url, home).await;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(
        stdout.trim_end(),
        "alpha\nbeta\nomega",
        "tail missing or duplicated: {stdout:?}"
    );
}

/// After a dropped socket the follow reconnects and resumes; the newline the
/// server inserts before every chunk must be counted, otherwise the next chunk
/// repeats a character.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replayed_chunks_keep_the_separator_and_cursor() {
    let url = spawn_server(
        "alpha",
        "alpha\nbeta\nomega",
        vec![
            Script {
                frames: vec![("beta".to_string(), 0)],
                close_after: true,
            },
            Script {
                frames: vec![("omega".to_string(), 0)],
                close_after: false,
            },
        ],
    )
    .await;
    let home = TempDir::new().unwrap();

    let output = run_logs(url, home).await;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(
        stdout.trim_end(),
        "alpha\nbeta\nomega",
        "reconnect lost or repeated text: {stdout:?}"
    );
}

/// Multi-line and CJK chunks keep their layout and are not repeated when the
/// poller takes over.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multiline_cjk_chunk_is_not_repeated() {
    let final_log = "第一行\n第二行\n第三行\n尾部";
    let url = spawn_server(
        "第一行",
        final_log,
        vec![Script {
            frames: vec![("第二行\n第三行".to_string(), 0)],
            close_after: false,
        }],
    )
    .await;
    let home = TempDir::new().unwrap();

    let output = run_logs(url, home).await;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(stdout.trim_end(), final_log, "stdout: {stdout:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_initial_log_replays_from_zero() {
    let final_log = "第一行\n第二行\n";
    let url = spawn_server("", final_log, vec![]).await;
    let output = run_logs(url, TempDir::new().unwrap()).await;
    assert_eq!(String::from_utf8(output.stdout).unwrap(), final_log);
}

/// Equal lengths do not prove that the stream is the final log's prefix.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inconsistent_final_log_is_an_error() {
    let url = spawn_server(
        "alpha",
        "alpha\nzeta",
        vec![Script {
            frames: vec![("beta".to_string(), 0)],
            close_after: false,
        }],
    )
    .await;
    let home = TempDir::new().unwrap();
    let output = tokio::task::spawn_blocking(move || {
        command(&url, &home).assert().failure().get_output().clone()
    })
    .await
    .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("prefix"), "stderr: {stderr}");
}
