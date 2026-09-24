//! Chrome integration test for redirects that must be blocked before egress.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tinybrowser::{Action, Browser, NavigateRequest, SessionOptions, SnapshotRequest, Target};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn disallowed_redirect_never_reaches_server() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let blocked_hits = Arc::new(AtomicUsize::new(0));
    let hits = Arc::clone(&blocked_hits);
    let server = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let hits = Arc::clone(&hits);
            tokio::spawn(async move {
                let mut request = [0_u8; 2048];
                let count = stream.read(&mut request).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&request[..count]);
                let response = if request.starts_with("GET /redirect ") {
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/blocked\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                } else if request.starts_with("GET /link ") {
                    let body =
                        format!("<a href=\"http://127.0.0.1:{port}/blocked\">Leave site</a>");
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                } else if request.starts_with("GET /blocked ") {
                    hits.fetch_add(1, Ordering::SeqCst);
                    "HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nblocked"
                        .to_owned()
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_owned()
                };
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });

    let browser = Browser::new();
    let session = browser
        .open_session(SessionOptions {
            allowed_origins: vec!["localhost".to_owned()],
            ..SessionOptions::default()
        })
        .await?;
    let _ = browser
        .navigate(
            &session.id,
            &NavigateRequest::new(format!("http://localhost:{port}/redirect")),
        )
        .await;
    assert_eq!(blocked_hits.load(Ordering::SeqCst), 0);

    browser
        .navigate(
            &session.id,
            &NavigateRequest::new(format!("http://localhost:{port}/link")),
        )
        .await?;
    let snapshot = browser
        .snapshot(&session.id, &SnapshotRequest::interactive())
        .await?;
    let link = snapshot
        .refs
        .iter()
        .find(|reference| reference.role == "link")
        .ok_or("link missing from snapshot")?;
    let _ = browser
        .perform(
            &session.id,
            &Action::Click {
                target: Target::reference(&link.id),
                new_tab: false,
            },
        )
        .await;
    assert_eq!(blocked_hits.load(Ordering::SeqCst), 0);
    browser.close_session(&session.id).await?;
    server.abort();
    Ok(())
}
