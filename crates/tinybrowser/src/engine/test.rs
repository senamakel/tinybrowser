//! Tests for the engine facade.
//!
//! Every operation on a session needs a browser, so what is covered here is the
//! bookkeeping around them: the session limit, the way an unknown session is
//! reported, and the output operations, which need no browser at all because the
//! store is fed by the capture path rather than by the socket.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tinybrowser_bus::{
    EvaluateRequest, NavigateRequest, OutputId, ReadRequest, ScreenshotRequest, SessionId,
    SnapshotRequest,
};

use super::Browser;
use crate::error::Error;

#[test]
fn a_new_engine_holds_the_default_session_limit() {
    assert_eq!(Browser::new().session_limit(), 8);
    assert_eq!(Browser::default().session_limit(), 8);
}

#[test]
fn a_session_limit_of_zero_is_raised_to_one() {
    // Zero would make the engine unable to open anything, which is never what a
    // caller configuring a limit meant.
    assert_eq!(Browser::with_session_limit(0).session_limit(), 1);
}

#[tokio::test]
async fn a_fresh_engine_lists_no_sessions() {
    assert!(Browser::new().list_sessions().await.is_empty());
}

#[tokio::test]
async fn every_session_operation_reports_an_unknown_session() {
    let browser = Browser::new();
    let id = SessionId::new("never-opened");

    let failures: Vec<Error> = vec![
        browser
            .navigate(&id, &NavigateRequest::new("https://example.com"))
            .await
            .expect_err("refused"),
        browser
            .snapshot(&id, &SnapshotRequest::default())
            .await
            .expect_err("refused"),
        browser
            .read_page(&id, &ReadRequest::default())
            .await
            .expect_err("refused"),
        browser
            .screenshot(&id, &ScreenshotRequest::default())
            .await
            .expect_err("refused"),
        browser
            .perform(&id, &tinybrowser_bus::Action::Reload)
            .await
            .expect_err("refused"),
        browser
            .evaluate(&id, &EvaluateRequest::new("1 + 1"))
            .await
            .expect_err("refused"),
    ];

    for error in failures {
        assert!(matches!(error, Error::NoSuchSession { .. }), "{error}");
    }
}

#[tokio::test]
async fn closing_an_unknown_session_succeeds() {
    let browser = Browser::new();

    assert!(
        browser
            .close_session(&SessionId::new("never-opened"))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn an_empty_expression_is_refused_before_the_session_is_looked_up() {
    // The order matters: an empty expression is the caller's mistake whether or
    // not the session exists, and reporting the session first would send them
    // looking in the wrong place.
    let browser = Browser::new();
    let error = browser
        .evaluate(&SessionId::new("never-opened"), &EvaluateRequest::new("  "))
        .await
        .expect_err("refused");

    assert!(matches!(error, Error::InvalidInput { .. }), "{error}");
}

#[tokio::test]
async fn reading_an_unknown_output_is_reported_as_such() {
    let browser = Browser::new();
    let error = browser
        .read_output(&OutputId::new("never-captured"), 0, 1024)
        .await
        .expect_err("refused");

    assert!(matches!(error, Error::NoSuchOutput { .. }), "{error}");
}

#[tokio::test]
async fn releasing_an_unknown_output_succeeds() {
    let browser = Browser::new();

    assert!(
        browser
            .release_output(&OutputId::new("never-captured"))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn shutting_down_an_empty_engine_is_a_no_op() {
    let browser = Browser::new();
    browser.shutdown().await;

    assert!(browser.list_sessions().await.is_empty());
}
