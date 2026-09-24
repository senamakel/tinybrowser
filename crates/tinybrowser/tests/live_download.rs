//! End-to-end download events against a real browser and loopback server.

#![allow(clippy::expect_used, clippy::panic)]

use tinybrowser::{
    Action, Browser, DownloadState, DownloadWaitRequest, NavigateRequest, SessionOptions,
    SnapshotRequest, Target,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

#[tokio::test]
async fn live_a_completed_download_is_returned_as_a_retained_handle() {
    if std::env::var_os("TINYBROWSER_LIVE_TESTS").is_none() {
        return;
    }
    let directory = tempfile::tempdir().expect("download directory");
    let url = serve().await;
    let browser = Browser::new();
    let session = browser
        .open_session(SessionOptions {
            download_dir: Some(directory.path().to_string_lossy().into_owned()),
            ..SessionOptions::default()
        })
        .await
        .expect("open browser");
    browser
        .navigate(&session.id, &NavigateRequest::new(url))
        .await
        .expect("open fixture");
    let snapshot = browser
        .snapshot(&session.id, &SnapshotRequest::interactive())
        .await
        .expect("snapshot fixture");
    let link = snapshot
        .refs
        .iter()
        .find(|element| element.name == "Download fixture")
        .expect("download link");
    browser
        .perform(
            &session.id,
            &Action::Click {
                target: Target::reference(&link.id),
                new_tab: false,
            },
        )
        .await
        .expect("click download");

    let download = browser
        .wait_download(
            &session.id,
            &DownloadWaitRequest {
                timeout_ms: Some(10_000),
            },
        )
        .await
        .expect("completed download");
    assert_eq!(download.state, DownloadState::Completed);
    assert_eq!(download.suggested_filename, "fixture.bin");
    assert_eq!(download.received_bytes, 16);
    let path = download.path.expect("configured local path");
    assert_eq!(
        tokio::fs::read(path).await.expect("downloaded bytes"),
        b"tinybrowser-data"
    );
    assert_eq!(
        browser
            .list_downloads(&session.id)
            .await
            .expect("list")
            .len(),
        1
    );
    browser.close_session(&session.id).await.expect("close");
}

async fn serve() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture");
    let address = listener.local_addr().expect("fixture address");
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut request = [0_u8; 2_048];
                let count = stream.read(&mut request).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&request[..count]);
                let (content_type, disposition, body): (&str, &str, &[u8]) = if request
                    .starts_with("GET /file ")
                {
                    (
                        "application/octet-stream",
                        "Content-Disposition: attachment; filename=\"fixture.bin\"\r\n",
                        b"tinybrowser-data",
                    )
                } else {
                    (
                            "text/html; charset=utf-8",
                            "",
                            b"<!doctype html><title>download</title><a href='/file' download>Download fixture</a>",
                        )
                };
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n{disposition}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes()).await;
                let _ = stream.write_all(body).await;
            });
        }
    });
    format!("http://{address}/")
}
