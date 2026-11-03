//! Timeout semantics (design doc §4.5): API calls carry a total timeout,
//! asset downloads only an idle timeout. A tiny raw-TCP server dribbles the
//! response body so the total transfer outlasts the timeout while no single
//! gap does — wiremock can only delay a response as a whole.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

use ifg_client::{Client, Method};

const TIMEOUT: Duration = Duration::from_secs(1);

/// Serve one HTTP response whose body is `chunks` written `gap` apart.
/// Returns the base URL.
fn dribbling_server(chunks: usize, gap: Duration) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buf = [0u8; 1024];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).unwrap();
            if n == 0 {
                return;
            }
            request.extend_from_slice(&buf[..n]);
        }
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
             Content-Length: {chunks}\r\nConnection: close\r\n\r\n"
        );
        // Write errors just mean the client gave up, which some tests expect.
        let _ = stream.write_all(head.as_bytes());
        for _ in 0..chunks {
            std::thread::sleep(gap);
            if stream
                .write_all(b"x")
                .and_then(|()| stream.flush())
                .is_err()
            {
                return;
            }
        }
    });
    format!("http://{addr}")
}

fn client(base: &str) -> Client {
    Client::builder(base)
        .timeout(TIMEOUT)
        .max_retries(0)
        .build()
        .unwrap()
}

#[tokio::test]
async fn slow_download_outlasting_the_timeout_succeeds() {
    // 4 × 400ms = 1.6s total, each gap well under the 1s timeout.
    let base = dribbling_server(4, Duration::from_millis(400));
    let response = client(&base)
        .releases("alice", "demo")
        .download(1)
        .await
        .unwrap();
    let body = response.bytes().await.unwrap();
    assert_eq!(body.len(), 4);
}

#[tokio::test]
async fn stalled_download_times_out() {
    let base = dribbling_server(2, Duration::from_millis(2500));
    let response = client(&base)
        .releases("alice", "demo")
        .download(1)
        .await
        .unwrap();
    let err = response.bytes().await.unwrap_err();
    assert!(err.is_timeout(), "expected a timeout, got {err:?}");
}

#[tokio::test]
async fn api_calls_keep_a_total_timeout() {
    let base = dribbling_server(4, Duration::from_millis(400));
    let err = client(&base)
        .raw(Method::GET, "users/me", None)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ifg_client::Error::Transport(e) if e.is_timeout()),
        "expected a transport timeout, got {err:?}"
    );
}
