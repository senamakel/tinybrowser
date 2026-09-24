//! Tests for download event retention and one-time terminal delivery.

#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use serde_json::json;
use tinybrowser_bus::DownloadState;

use super::{DownloadStore, DownloadTracker};
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
                "frameId": "page-1",
            }),
        ),
        Some(std::path::Path::new("/downloads")),
        "page-1",
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
        "page-1",
    ));

    let info = store.take_terminal().expect("terminal handle");
    assert_eq!(info.sequence, 1);
    assert_eq!(info.state, DownloadState::Completed);
    assert_eq!(info.received_bytes, 42);
    assert_eq!(info.total_bytes, Some(42));
    assert_eq!(info.path.as_deref(), Some("/downloads/d-1"));
    assert!(store.take_terminal().is_none());
}

#[test]
fn cancellation_is_terminal_and_listing_does_not_consume_it() {
    let mut store = DownloadStore::default();
    store.apply(
        &event(
            "Browser.downloadWillBegin",
            json!({"guid": "d-2", "frameId": "page-1"}),
        ),
        None,
        "page-1",
    );
    store.apply(
        &event(
            "Browser.downloadProgress",
            json!({"guid": "d-2", "state": "canceled"}),
        ),
        None,
        "page-1",
    );
    assert_eq!(store.list()[0].state, DownloadState::Cancelled);
    assert_eq!(store.list()[0].state, DownloadState::Cancelled);
    assert!(store.take_terminal().is_some());
}

#[test]
fn duplicate_terminal_progress_is_delivered_once() {
    let mut store = DownloadStore::default();
    store.apply(
        &event(
            "Browser.downloadWillBegin",
            json!({"guid": "d-3", "frameId": "page-1"}),
        ),
        None,
        "page-1",
    );
    let completed = event(
        "Browser.downloadProgress",
        json!({"guid": "d-3", "state": "completed"}),
    );
    store.apply(&completed, None, "page-1");
    store.apply(&completed, None, "page-1");
    assert!(store.take_terminal().is_some());
    assert!(store.take_terminal().is_none());
}

#[test]
fn events_from_another_page_and_unknown_guids_are_ignored() {
    let mut store = DownloadStore::default();
    assert!(!store.apply(
        &event(
            "Browser.downloadWillBegin",
            json!({"guid": "other", "frameId": "other-page"}),
        ),
        None,
        "page-1",
    ));
    assert!(!store.apply(
        &event(
            "Browser.downloadProgress",
            json!({"guid": "other", "state": "completed"})
        ),
        None,
        "page-1",
    ));
    assert!(store.list().is_empty());
}

#[tokio::test]
async fn a_terminal_event_retained_before_wait_is_returned_immediately() {
    let tracker = DownloadTracker::new(None, "page-1");
    tracker.store.lock().await.apply(
        &event(
            "Browser.downloadWillBegin",
            json!({"guid": "fast", "frameId": "page-1"}),
        ),
        None,
        "page-1",
    );
    tracker.store.lock().await.apply(
        &event(
            "Browser.downloadProgress",
            json!({"guid": "fast", "state": "completed"}),
        ),
        None,
        "page-1",
    );
    let info = tracker
        .wait(Duration::from_millis(10))
        .await
        .expect("retained event");
    assert_eq!(info.id.as_str(), "fast");
}

#[tokio::test]
async fn waiting_without_a_terminal_event_times_out() {
    let error = DownloadTracker::new(None, "page-1")
        .wait(Duration::from_millis(1))
        .await
        .expect_err("times out");
    assert!(error.to_string().contains("download timed out"));
}

#[tokio::test]
async fn closing_the_tracker_wakes_waiters_with_connection_lost() {
    let tracker = DownloadTracker::new(None, "page-1");
    tracker.close().await;
    let error = tracker
        .wait(Duration::from_secs(1))
        .await
        .expect_err("closed tracker fails");
    assert!(error.to_string().contains("download monitor stopped"));
}
