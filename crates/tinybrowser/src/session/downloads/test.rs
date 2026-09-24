//! Tests for download event retention and one-time terminal delivery.

#![allow(clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::time::Duration;

use serde_json::json;
use tinybrowser_bus::DownloadState;

use super::{DownloadStore, DownloadTracker, expected_path};
use crate::cdp::CdpEvent;

fn event(method: &str, params: serde_json::Value) -> CdpEvent {
    CdpEvent {
        method: method.to_owned(),
        params,
        session_id: None,
    }
}

#[test]
fn begin_and_progress_build_a_terminal_handle() {
    let mut store = DownloadStore::default();
    assert!(store.apply(
        &event(
            "Browser.downloadWillBegin",
            json!({
                "guid": "d-1",
                "url": "https://example.com/file.dmg",
                "suggestedFilename": "file.dmg",
            }),
        ),
        Some(Path::new("/downloads")),
    ));
    assert!(store.apply(
        &event(
            "Browser.downloadProgress",
            json!({
                "guid": "d-1",
                "receivedBytes": 42.0,
                "totalBytes": 42.0,
                "state": "completed",
            }),
        ),
        None,
    ));

    let info = store.take_terminal().expect("terminal handle");
    assert_eq!(info.sequence, 1);
    assert_eq!(info.state, DownloadState::Completed);
    assert_eq!(info.received_bytes, 42);
    assert_eq!(info.total_bytes, Some(42));
    assert_eq!(info.path.as_deref(), Some("/downloads/file.dmg"));
    assert!(store.take_terminal().is_none());
}

#[test]
fn cancellation_is_terminal_and_listing_does_not_consume_it() {
    let mut store = DownloadStore::default();
    store.apply(
        &event(
            "Browser.downloadProgress",
            json!({"guid": "d-2", "state": "canceled"}),
        ),
        None,
    );
    assert_eq!(store.list()[0].state, DownloadState::Cancelled);
    assert_eq!(store.list()[0].state, DownloadState::Cancelled);
    assert!(store.take_terminal().is_some());
}

#[test]
fn duplicate_terminal_progress_is_delivered_once() {
    let mut store = DownloadStore::default();
    let completed = event(
        "Browser.downloadProgress",
        json!({"guid": "d-3", "state": "completed"}),
    );
    store.apply(&completed, None);
    store.apply(&completed, None);
    assert!(store.take_terminal().is_some());
    assert!(store.take_terminal().is_none());
}

#[test]
fn unsafe_suggested_paths_are_reduced_to_the_filename() {
    assert_eq!(
        expected_path(Path::new("/downloads"), "../../OpenHuman.dmg")
            .expect("safe filename")
            .to_string_lossy(),
        "/downloads/OpenHuman.dmg"
    );
    assert!(expected_path(Path::new("/downloads"), "").is_none());
}

#[tokio::test]
async fn a_terminal_event_retained_before_wait_is_returned_immediately() {
    let tracker = DownloadTracker::new(None);
    tracker.store.lock().await.apply(
        &event(
            "Browser.downloadProgress",
            json!({"guid": "fast", "state": "completed"}),
        ),
        None,
    );
    let info = tracker
        .wait(Duration::from_millis(10))
        .await
        .expect("retained event");
    assert_eq!(info.id.as_str(), "fast");
}

#[tokio::test]
async fn waiting_without_a_terminal_event_times_out() {
    let error = DownloadTracker::new(None)
        .wait(Duration::from_millis(1))
        .await
        .expect_err("times out");
    assert!(error.to_string().contains("download timed out"));
}
