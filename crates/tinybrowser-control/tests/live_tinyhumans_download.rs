//! Opt-in live `OpenRouter` download task against tinyhumans.ai.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::path::PathBuf;

use tinybrowser::{
    Browser, DownloadState, DownloadWaitRequest, NavigateRequest, SessionOptions, WaitUntil,
};
use tinybrowser_control::{ControlLimits, JevController, TaskRequest};
use tinyjevclient::{Client, ClientConfig};

#[tokio::test]
async fn live_tinyhumans_downloads_openhuman_for_apple_silicon() {
    if std::env::var_os("TINYBROWSER_DOWNLOAD_LIVE_TESTS").is_none() {
        return;
    }
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .expect("OPENROUTER_API_KEY must be set for the opted-in download test");
    let download_dir = PathBuf::from(
        std::env::var("TINYBROWSER_DOWNLOAD_DIR")
            .expect("TINYBROWSER_DOWNLOAD_DIR must name an explicit destination"),
    );
    assert!(download_dir.is_dir(), "download destination must exist");
    let browser = Browser::new();
    let session = browser
        .open_session(SessionOptions {
            headless: false,
            download_dir: Some(download_dir.to_string_lossy().into_owned()),
            ..SessionOptions::default()
        })
        .await
        .expect("launch headed browser");
    browser
        .navigate(
            &session.id,
            &NavigateRequest {
                url: "https://tinyhumans.ai/openhuman".to_owned(),
                wait_until: WaitUntil::DomContentLoaded,
                timeout_ms: Some(60_000),
            },
        )
        .await
        .expect("open TinyHumans");
    let task = TaskRequest::new(
        "Download OpenHuman for Apple Silicon macOS. Start the official download from TinyHumans, then stop. Do not install, mount, or open the downloaded application.",
    );
    let controller = JevController::new(
        Client::new(ClientConfig::openrouter(api_key)).expect("OpenRouter client"),
    )
    .with_limits(ControlLimits {
        max_steps: 12,
        max_unchanged_steps: 2,
        wait_ms: 750,
        ..ControlLimits::default()
    });
    let result = controller
        .run(&browser, &session.id, &task)
        .await
        .expect("controller run");
    println!(
        "controller status={:?} steps={}",
        result.status,
        result.steps.len()
    );
    let download = browser
        .wait_download(
            &session.id,
            &DownloadWaitRequest {
                timeout_ms: Some(180_000),
            },
        )
        .await
        .expect("download handle");

    assert_eq!(download.state, DownloadState::Completed);
    let downloaded = PathBuf::from(download.path.expect("configured download path"));
    let size = tokio::fs::metadata(&downloaded)
        .await
        .expect("download metadata")
        .len();
    assert!(
        size > 1_000_000,
        "downloaded installer is unexpectedly small"
    );
    println!("downloaded={} bytes={size}", downloaded.display());
    browser
        .close_session(&session.id)
        .await
        .expect("close browser");
}
