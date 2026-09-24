//! Integration tests for the public crate surface.
//!
//! These tests link against the crate as a downstream consumer would: they can
//! only use what `src/lib.rs` re-exports. Treat them as the regression suite for
//! the crate's public contract — if a change breaks a test here, it is a
//! breaking change for users.
//!
//! Nothing here launches a browser. What a consumer most needs to be able to do
//! without one is build every payload, name every member, and classify every
//! failure, and that is what these cover.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tinybrowser::{
    Action, Browser, DownloadWaitRequest, Error, EvaluateRequest, LocateBy, Locator,
    NavigateRequest, OutputId, ReadFormat, ReadRequest, ScreenshotRequest, SessionId,
    SessionOptions, SnapshotRequest, Target, WaitUntil, errors, is_compatible, names,
};

#[test]
fn the_engine_is_available_to_consumers() {
    let browser = Browser::new();
    assert_eq!(browser.session_limit(), 8);
}

#[test]
fn the_contract_is_re_exported_rather_than_restated() {
    // The same type, not a structural twin: a host that depends on
    // `tinybrowser-bus` alone and a module author who depends on this crate must
    // be able to pass payloads between each other without a conversion.
    let via_module: tinybrowser::Action = Action::Reload;
    let via_contract: tinybrowser_bus::Action = via_module;

    assert_eq!(via_contract, tinybrowser_bus::Action::Reload);
}

#[test]
fn the_bus_identity_is_available_to_consumers() {
    assert_eq!(names::INTERFACE, "ai.tinyhumans.tinybrowser.Browser");
    assert_eq!(names::OBJECT_PATH, "/ai/tinyhumans/tinybrowser/Browser");
    assert_eq!(names::METHODS.len(), 14);
    assert!(is_compatible(tinybrowser::CONTRACT_VERSION));
}

#[test]
fn every_payload_a_caller_needs_can_be_built_from_the_public_surface() {
    let _ = SessionOptions::default();
    let _ = NavigateRequest::new("https://example.com");
    let _ = SnapshotRequest::interactive();
    let _ = ReadRequest::default();
    let _ = EvaluateRequest::new("1 + 1");
    let _ = ScreenshotRequest::default();
    let _ = DownloadWaitRequest::default();
    let _ = Action::Fill {
        target: Target::parse("#email"),
        value: "someone@example.com".to_string(),
    };
    let _ = Target::locator(Locator::new(LocateBy::Role, "button").with_name("Submit"));
}

#[test]
fn defaults_are_the_ones_the_contract_documents() {
    assert_eq!(NavigateRequest::new("x").wait_until, WaitUntil::Load);
    assert_eq!(ReadRequest::default().format, ReadFormat::Markdown);
    assert!(SessionOptions::default().headless);
}

#[test]
fn errors_carry_the_wire_names_a_host_matches_on() {
    assert_eq!(
        Error::invalid_input("bad").wire_name(),
        errors::INVALID_INPUT
    );
    assert!(errors::is_agent_recoverable(errors::NO_SUCH_ELEMENT));
    assert!(!errors::is_agent_recoverable(errors::BROWSER_UNAVAILABLE));
}

#[tokio::test]
async fn an_unknown_session_is_reported_without_a_browser() {
    let browser = Browser::new();
    let error = browser
        .navigate(&SessionId::new("never-opened"), &NavigateRequest::new("x"))
        .await
        .expect_err("refused");

    assert!(matches!(error, Error::NoSuchSession { .. }), "{error}");
    assert_eq!(error.wire_name(), errors::NO_SUCH_SESSION);
}

#[tokio::test]
async fn teardown_is_idempotent_for_consumers() {
    // A host retrying a close or a release after a timeout must not have to tell
    // "never existed" apart from "already cleaned up".
    let browser = Browser::new();

    assert!(browser.close_session(&SessionId::new("gone")).await.is_ok());
    assert!(browser.release_output(&OutputId::new("gone")).await.is_ok());
}
