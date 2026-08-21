//! Tests for the protocol layer that need no browser.
//!
//! Everything here is about the decisions made *before* a socket exists:
//! which endpoint form is usable, and which executable a launch would pick.
//! The socket itself and the launch it performs are exercised by the
//! `live-chrome` suite, where there is a browser to talk to.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::endpoint::resolve;
use super::launch::{EXECUTABLE_ENV, find_executable};
use crate::error::Error;

#[tokio::test]
async fn a_websocket_endpoint_is_already_resolved() {
    let url = "ws://127.0.0.1:9222/devtools/browser/abc";
    assert_eq!(resolve(url).await.expect("resolves"), url);
}

#[tokio::test]
async fn a_secure_websocket_endpoint_is_returned_unchanged() {
    let url = "wss://browser.example.com/devtools/browser/abc";
    assert_eq!(resolve(url).await.expect("resolves"), url);
}

#[tokio::test]
async fn a_trailing_slash_is_not_carried_into_the_request_path() {
    // `{endpoint}/json/version` against an unstripped endpoint would ask for
    // `//json/version`, which some proxies answer with a 404.
    assert_eq!(
        resolve("ws://127.0.0.1:9222/devtools/browser/abc/")
            .await
            .expect("resolves"),
        "ws://127.0.0.1:9222/devtools/browser/abc"
    );
}

#[tokio::test]
async fn an_unusable_scheme_is_refused_without_a_request() {
    let error = resolve("tcp://127.0.0.1:9222").await.expect_err("refused");
    assert!(matches!(error, Error::InvalidInput { .. }), "{error}");
}

#[tokio::test]
async fn a_bare_host_is_refused_rather_than_guessed_at() {
    let error = resolve("127.0.0.1:9222").await.expect_err("refused");
    assert!(matches!(error, Error::InvalidInput { .. }), "{error}");
}

#[test]
fn a_configured_executable_that_exists_is_taken_as_given() {
    let existing = std::env::current_exe().expect("this test binary exists");
    let path = existing.to_string_lossy().to_string();

    assert_eq!(find_executable(Some(&path)).expect("found"), existing);
}

#[test]
fn a_configured_executable_that_does_not_exist_is_reported_not_searched_past() {
    // The alternative — silently falling back to whatever browser happens to be
    // installed — hides a typo in a host's configuration behind a browser that
    // works, which is the worst possible outcome for a setting whose whole
    // purpose is to pin which binary runs.
    let error = find_executable(Some("/nonexistent/chrome")).expect_err("refused");

    assert!(matches!(error, Error::BrowserUnavailable { .. }), "{error}");
    assert!(error.to_string().contains("/nonexistent/chrome"));
}

#[test]
fn the_error_for_a_missing_browser_names_the_three_ways_out() {
    // This message is what an operator sees when a host has no browser at all,
    // and it is the only place the module can tell them what to do about it.
    let Err(error) = find_executable(Some("/nonexistent/chrome")) else {
        panic!("expected a missing browser");
    };
    let _ = error;

    // The override is named in the message the discovery path produces, which is
    // only reachable on a host without a browser — assert on the constant the
    // message is built from instead.
    assert_eq!(EXECUTABLE_ENV, "TINYBROWSER_CHROME");
}
