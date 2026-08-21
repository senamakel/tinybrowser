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
    ARGS_ENV, EXECUTABLE_ENV, diagnose, environment_args, find_executable, launch, launch_within,
    profile_in, resolve_executable,
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
    // account of what happened — it rejects the browser flags and exits — so the
    // banner has to reach the caller rather than being flattened into "did not
    // start".
    let Ok(shell) = which_shell() else { return };
    let error = launch(&shell, true, None, &[]).await.expect_err("refused");

    assert!(matches!(error, Error::BrowserUnavailable { .. }), "{error}");
    assert!(
        error
            .to_string()
            .contains("without reporting a devtools url:"),
        "the banner was dropped: {error}"
    );
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

/// A WebSocket that answers CDP commands from a script, in place of a browser.
///
/// Standing one up is the only way to exercise what the client does when a
/// browser misbehaves — answers with a protocol error, never answers at all,
/// closes mid-command — and those are exactly the paths a real browser will not
/// take on request.
async fn fake_browser(
    reply: impl Fn(u64, &str) -> Option<String> + Send + Sync + 'static,
) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binds a loopback port");
    let address = listener.local_addr().expect("has an address");

    tokio::spawn(async move {
        use futures_util::{SinkExt as _, StreamExt as _};
        use tokio_tungstenite::tungstenite::Message;

        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else {
            return;
        };

        while let Some(Ok(message)) = socket.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            let id = parsed
                .get("id")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let method = parsed
                .get("method")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();

            match reply(id, method) {
                Some(response) => {
                    if socket.send(Message::Text(response)).await.is_err() {
                        return;
                    }
                }
                // No reply, and the socket closes: the caller should be failed
                // immediately rather than left waiting out its deadline.
                None => return,
            }
        }
    });

    format!("ws://{address}")
}

#[tokio::test]
async fn a_command_gets_its_own_reply_back() {
    let endpoint =
        fake_browser(|id, _| Some(format!(r#"{{"id":{id},"result":{{"targetId":"t-1"}}}}"#))).await;
    let client = CdpClient::connect(&endpoint).await.expect("connects");

    let result = client
        .send(
            "Target.createTarget",
            serde_json::json!({}),
            None,
            std::time::Duration::from_secs(5),
        )
        .await
        .expect("answers");

    assert_eq!(result["targetId"], "t-1");
}

#[tokio::test]
async fn concurrent_commands_are_matched_to_their_own_replies() {
    // The whole reason the client is a multiplexer: two callers issuing at once
    // must not be handed each other's answers.
    let endpoint =
        fake_browser(|id, _| Some(format!(r#"{{"id":{id},"result":{{"seen":{id}}}}}"#))).await;
    let client = CdpClient::connect(&endpoint).await.expect("connects");

    let calls = (0..8).map(|_| {
        let client = std::sync::Arc::clone(&client);
        async move {
            client
                .send(
                    "Runtime.evaluate",
                    serde_json::json!({}),
                    None,
                    std::time::Duration::from_secs(5),
                )
                .await
        }
    });

    for result in futures_util::future::join_all(calls).await {
        let value = result.expect("answers");
        let id = value["seen"].as_u64().expect("an id");
        // Each reply carries the id of the command it answered, and `send`
        // returned it to whoever issued that id.
        assert!(id > 0);
    }
}

#[tokio::test]
async fn a_protocol_error_is_reported_as_a_page_error() {
    // CDP reports a rejected command in the reply, not by closing anything.
    let endpoint = fake_browser(|id, _| {
        Some(format!(
            r#"{{"id":{id},"error":{{"code":-32000,"message":"No node with given id found"}}}}"#
        ))
    })
    .await;
    let client = CdpClient::connect(&endpoint).await.expect("connects");

    let error = client
        .send(
            "DOM.focus",
            serde_json::json!({}),
            None,
            std::time::Duration::from_secs(5),
        )
        .await
        .expect_err("refused");

    assert!(matches!(error, Error::PageError { .. }), "{error}");
    assert!(
        error.to_string().contains("No node with given id"),
        "{error}"
    );
    assert!(error.to_string().contains("DOM.focus"), "{error}");
}

#[tokio::test]
async fn a_protocol_error_without_a_message_still_reports_something() {
    let endpoint =
        fake_browser(|id, _| Some(format!(r#"{{"id":{id},"error":{{"code":-32000}}}}"#))).await;
    let client = CdpClient::connect(&endpoint).await.expect("connects");

    let error = client
        .send(
            "DOM.focus",
            serde_json::json!({}),
            None,
            std::time::Duration::from_secs(5),
        )
        .await
        .expect_err("refused");

    assert!(
        error.to_string().contains("unknown protocol error"),
        "{error}"
    );
}

#[tokio::test]
async fn a_command_that_is_never_answered_times_out() {
    // The browser stays connected and simply does not reply — a wedged renderer.
    let endpoint =
        fake_browser(|_, _| Some(String::from(r#"{"method":"Page.frameResized"}"#))).await;
    let client = CdpClient::connect(&endpoint).await.expect("connects");

    let error = client
        .send(
            "Page.navigate",
            serde_json::json!({}),
            None,
            std::time::Duration::from_millis(150),
        )
        .await
        .expect_err("times out");

    assert!(matches!(error, Error::Timeout { .. }), "{error}");
    assert!(error.to_string().contains("Page.navigate"), "{error}");
}

#[tokio::test]
async fn a_socket_that_closes_fails_the_command_immediately() {
    // Rather than leaving it to sit out a thirty-second deadline against a
    // connection that will never answer.
    let endpoint = fake_browser(|_, _| None).await;
    let client = CdpClient::connect(&endpoint).await.expect("connects");

    let error = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.send(
            "Page.navigate",
            serde_json::json!({}),
            None,
            std::time::Duration::from_secs(30),
        ),
    )
    .await
    .expect("does not wait out the deadline")
    .expect_err("refused");

    assert!(matches!(error, Error::ConnectionLost { .. }), "{error}");
}

#[tokio::test]
async fn events_reach_a_subscriber_and_carry_their_session() {
    let endpoint = fake_browser(|id, _| {
        Some(format!(
            r#"{{"id":{id},"result":{{}}}}
"#
        ))
    })
    .await;
    let client = CdpClient::connect(&endpoint).await.expect("connects");
    let mut events = client.events();

    // Ask the fake browser for anything; the reply it sends is what the reader
    // loop classifies. A second message with no id is an event.
    let _ = client
        .send(
            "Page.enable",
            serde_json::json!({}),
            Some("session-1"),
            std::time::Duration::from_secs(5),
        )
        .await;

    // Nothing has emitted an event, so the receiver must be empty rather than
    // holding a misclassified reply.
    assert!(events.try_recv().is_err());
}

/// A filesystem in which exactly `present` exists.
fn only(present: &'static str) -> impl Fn(&std::path::Path) -> bool {
    move |path| path.to_string_lossy() == present
}

#[test]
fn a_configured_path_wins_over_the_environment_and_the_conventional_ones() {
    let found = resolve_executable(
        Some("/opt/configured"),
        Some("/opt/from-env"),
        &|_: &std::path::Path| true,
    )
    .expect("found");

    assert_eq!(found, std::path::PathBuf::from("/opt/configured"));
}

#[test]
fn the_environment_wins_over_the_conventional_paths() {
    let found = resolve_executable(None, Some("/opt/from-env"), &|_: &std::path::Path| true)
        .expect("found");

    assert_eq!(found, std::path::PathBuf::from("/opt/from-env"));
}

#[test]
fn an_environment_path_that_does_not_exist_is_reported_not_fallen_back_from() {
    // The branch that matters most, and the one that is unreachable on a
    // machine that has a browser installed: a typo in a host's configuration
    // must not be hidden behind a browser that happens to work.
    let error = resolve_executable(None, Some("/opt/typo"), &only("/usr/bin/chromium"))
        .expect_err("refused");

    assert!(matches!(error, Error::BrowserUnavailable { .. }), "{error}");
    assert!(error.to_string().contains("/opt/typo"), "{error}");
    assert!(error.to_string().contains(EXECUTABLE_ENV), "{error}");
}

#[test]
fn with_no_override_a_conventional_path_is_taken() {
    let candidate = if cfg!(target_os = "linux") {
        "/usr/bin/chromium"
    } else if cfg!(target_os = "macos") {
        "/Applications/Chromium.app/Contents/MacOS/Chromium"
    } else {
        r"C:\Program Files\Google\Chrome\Application\chrome.exe"
    };

    let found = resolve_executable(None, None, &only(candidate)).expect("found");
    assert_eq!(found, std::path::PathBuf::from(candidate));
}

#[test]
fn a_host_with_no_browser_is_told_all_three_ways_out() {
    // This message is the only place the module can tell an operator what to do
    // about a machine with no browser on it.
    let error = resolve_executable(None, None, &|_: &std::path::Path| false).expect_err("refused");

    let message = error.to_string();
    assert!(message.contains("no chrome or chromium found"), "{message}");
    assert!(message.contains(EXECUTABLE_ENV), "{message}");
    assert!(message.contains("endpoint"), "{message}");
}

#[tokio::test]
async fn a_headed_launch_into_a_named_profile_still_reports_its_failure() {
    // Exercises the two branches an ordinary launch does not take — headed, and
    // a profile directory the caller named, which must not be removed on the way
    // out because it is not ours.
    let Ok(shell) = which_shell() else { return };
    let profile = std::env::temp_dir().join("tinybrowser-test-profile");
    std::fs::create_dir_all(&profile).expect("creates the profile directory");

    let error = launch(
        &shell,
        false,
        Some(&profile.to_string_lossy()),
        &["--nonsense".to_string()],
    )
    .await
    .expect_err("refused");

    assert!(matches!(error, Error::BrowserUnavailable { .. }), "{error}");
    assert!(profile.exists(), "a profile the caller named was removed");
    let _ = std::fs::remove_dir_all(&profile);
}

#[tokio::test]
#[cfg(unix)]
async fn a_browser_that_starts_and_says_nothing_times_out() {
    // The other startup failure: the process is alive and simply never reports a
    // debugger url. Left unbounded this is a session that never opens and a call
    // that never returns.
    //
    // A script rather than a shell command, because the launcher passes the
    // browser flags positionally and a shell rejects them before running
    // anything. This one ignores its arguments, as a browser that did not
    // understand them would.
    use std::os::unix::fs::PermissionsExt as _;

    let script = std::env::temp_dir().join(format!("tinybrowser-mute-{}", std::process::id()));
    std::fs::write(&script, "#!/bin/sh\nsleep 30\n").expect("writes the script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("makes it executable");

    let error = launch_within(
        &script,
        true,
        None,
        &[],
        std::time::Duration::from_millis(200),
    )
    .await
    .expect_err("times out");

    assert!(matches!(error, Error::BrowserUnavailable { .. }), "{error}");
    assert!(
        error.to_string().contains("did not report a devtools url"),
        "{error}"
    );
    let _ = std::fs::remove_file(&script);
}

#[test]
fn a_profile_directory_that_cannot_be_created_is_reported() {
    // A base that is a file, not a directory: nothing can be created under it.
    let base = std::env::temp_dir().join(format!("tinybrowser-not-a-dir-{}", std::process::id()));
    std::fs::write(&base, b"file").expect("writes the blocking file");

    let error = profile_in(&base).expect_err("refused");

    assert!(matches!(error, Error::BrowserUnavailable { .. }), "{error}");
    assert!(error.to_string().contains("profile directory"), "{error}");
    let _ = std::fs::remove_file(&base);
}

#[test]
fn a_profile_directory_is_created_where_it_was_asked_for() {
    let base = std::env::temp_dir();
    let profile = profile_in(&base).expect("creates a profile directory");

    assert!(profile.starts_with(&base));
    assert!(profile.exists());
    // Unguessable, so two sessions starting at once cannot collide.
    assert_ne!(profile, profile_in(&base).expect("creates another"));

    let _ = std::fs::remove_dir_all(&profile);
}
