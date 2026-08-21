//! Tests for the protocol layer that need no browser.
//!
//! Everything here is about the decisions made *before* a socket exists:
//! which endpoint form is usable, and which executable a launch would pick.
//! The socket itself and the launch it performs are exercised by the
//! `live-chrome` suite, where there is a browser to talk to.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::client::CdpClient;
use super::endpoint::resolve;
use super::launch::{
    ARGS_ENV, EXECUTABLE_ENV, diagnose, environment_args, find_executable, launch,
};
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

/// Serves one HTTP response on loopback, and returns the base URL.
///
/// The `DevTools` endpoint this stands in for answers a single GET from memory,
/// so a listener that accepts once and writes a fixed body is not a simplified
/// version of it — it is the same shape.
async fn http_once(body: &'static str, status: &'static str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binds a loopback port");
    let address = listener.local_addr().expect("has an address");

    tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let mut scratch = [0_u8; 1024];
        let _ = stream.read(&mut scratch).await;
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;
    });

    format!("http://{address}")
}

#[tokio::test]
async fn an_http_endpoint_is_asked_for_its_debugger_url() {
    let endpoint = http_once(
        r#"{"Browser":"Chrome/1","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/browser/x"}"#,
        "200 OK",
    )
    .await;

    assert_eq!(
        resolve(&endpoint).await.expect("resolves"),
        "ws://127.0.0.1:9222/devtools/browser/x"
    );
}

#[tokio::test]
async fn an_endpoint_that_answers_without_a_debugger_url_is_reported_as_such() {
    // Reachable but not a devtools endpoint — an ordinary web server on the port
    // somebody meant to point at Chrome. Saying so is more useful than "failed".
    let endpoint = http_once(r#"{"Browser":"Chrome/1"}"#, "200 OK").await;
    let error = resolve(&endpoint).await.expect_err("refused");

    assert!(matches!(error, Error::BrowserUnavailable { .. }), "{error}");
    assert!(error.to_string().contains("webSocketDebuggerUrl"));
}

#[tokio::test]
async fn an_endpoint_that_answers_with_something_else_is_reported() {
    let endpoint = http_once("not json at all", "200 OK").await;
    let error = resolve(&endpoint).await.expect_err("refused");

    assert!(matches!(error, Error::BrowserUnavailable { .. }), "{error}");
}

#[tokio::test]
async fn an_endpoint_nothing_is_listening_on_is_reported() {
    // Port 1 on loopback: reserved, and nothing legitimate binds it.
    let error = resolve("http://127.0.0.1:1").await.expect_err("refused");

    assert!(matches!(error, Error::BrowserUnavailable { .. }), "{error}");
}

#[tokio::test]
async fn connecting_to_a_socket_that_is_not_there_is_reported() {
    let error = CdpClient::connect("ws://127.0.0.1:1/devtools/browser/x")
        .await
        .expect_err("refused");

    assert!(matches!(error, Error::BrowserUnavailable { .. }), "{error}");
    assert!(error.to_string().contains("cdp connect"));
}

#[tokio::test]
async fn launching_something_that_is_not_a_browser_reports_what_it_printed() {
    // A path that exists and runs but is not Chrome. The banner is the only
    // account of what happened, so it has to reach the caller.
    let Ok(shell) = which_shell() else { return };
    let error = launch(
        &shell,
        true,
        None,
        &["-c".to_string(), "echo nope >&2".to_string()],
    )
    .await
    .expect_err("refused");

    assert!(matches!(error, Error::BrowserUnavailable { .. }), "{error}");
    assert!(error.to_string().contains("nope"), "{error}");
}

#[tokio::test]
async fn launching_a_path_that_cannot_be_executed_is_reported() {
    let error = launch(std::path::Path::new("/nonexistent/chrome"), true, None, &[])
        .await
        .expect_err("refused");

    assert!(matches!(error, Error::BrowserUnavailable { .. }), "{error}");
}

#[test]
fn a_missing_sandbox_is_diagnosed_by_name() {
    // Chrome buries this behind a stack trace, and it is the failure an operator
    // is most likely to hit — a container without the right capabilities, or any
    // Ubuntu since 23.10.
    let banner = vec![
        "[1:1:0101/000000:FATAL:zygote_host_impl_linux.cc(128)] No usable sandbox! If you are"
            .to_string(),
        "Received signal 6".to_string(),
    ];

    let diagnosis = diagnose(&banner);
    assert!(diagnosis.contains("--no-sandbox"), "{diagnosis}");
    assert!(diagnosis.contains("user namespaces"), "{diagnosis}");
}

#[test]
fn another_failure_reports_what_the_browser_printed() {
    let banner = vec![
        String::new(),
        "error while loading shared libraries: libnss3.so".to_string(),
    ];

    let diagnosis = diagnose(&banner);
    assert!(diagnosis.contains("libnss3.so"), "{diagnosis}");
}

#[test]
fn a_silent_exit_still_says_something() {
    let diagnosis = diagnose(&[]);

    assert!(diagnosis.contains("without reporting a devtools url"));
}

#[test]
fn the_environment_supplies_no_extra_flags_by_default() {
    // Read rather than set: a test that mutates the environment races every
    // other test in the process, and the default is the case worth pinning.
    if std::env::var(ARGS_ENV).is_err() {
        assert!(environment_args().is_empty());
    }
}

#[test]
fn the_environment_variables_are_the_documented_names() {
    // An operator sets these on a host; renaming one silently stops it working.
    assert_eq!(EXECUTABLE_ENV, "TINYBROWSER_CHROME");
    assert_eq!(ARGS_ENV, "TINYBROWSER_CHROME_ARGS");
}

/// A shell to stand in for a browser that starts and then fails.
fn which_shell() -> Result<std::path::PathBuf, ()> {
    ["/bin/sh", "/usr/bin/sh"]
        .into_iter()
        .map(std::path::PathBuf::from)
        .find(|path| path.exists())
        .ok_or(())
}
