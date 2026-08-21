//! End-to-end tests against a real browser.
//!
//! # Why these are behind an environment variable
//!
//! Everything else in this repository is deterministic and needs nothing but a
//! Rust toolchain. These need a Chrome, and a CI runner without one would fail
//! them for a reason that has nothing to do with the change under test. So they
//! are opt-in and named `live_*`, per this repository's testing rules:
//!
//! ```sh
//! TINYBROWSER_LIVE_TESTS=1 cargo test -p tinybrowser --test live_chrome
//! ```
//!
//! An environment variable rather than a Cargo feature because the contract
//! command is `cargo test --all-features`, and a feature would be switched on by
//! exactly the command that must keep passing on a machine with no browser.
//!
//! They are still hermetic in the way that matters: every page they drive is
//! served by a one-shot HTTP server this file starts on loopback, so nothing
//! here touches the network and no assertion depends on a website that can
//! change underneath it.
//!
//! Loopback rather than a `data:` URL because the module refuses to navigate to
//! anything but `http`, `https`, and `about:blank` — a policy worth keeping, and
//! one a test suite should be bound by rather than exempt from.
//!
//! # Opted in, they fail rather than skip
//!
//! Setting the variable is the opt-in. Having opted in, a run that cannot find a
//! browser fails: a suite that quietly skips reports green for a build in which
//! nothing was checked, which is worse than no suite at all.
//!
//! On a host where Chrome needs extra flags — no usable sandbox, most often —
//! pass them through:
//!
//! ```sh
//! TINYBROWSER_CHROME=/path/to/chrome \
//! TINYBROWSER_TEST_ARGS=--no-sandbox \
//!   cargo test -p tinybrowser --features live-chrome
//! ```
//!
//! # What they are for
//!
//! The unit suites cover every decision this crate makes *around* the protocol.
//! What they cannot cover is whether the protocol conversation is right — that a
//! ref resolves to the node the snapshot named, that a dispatched click reaches
//! a handler, that a covered element is refused. That is the whole of what these
//! check, and it is the part that would otherwise only be discovered in
//! production.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tinybrowser::{
    Action, Browser, Error, LocateBy, Locator, NavigateRequest, ReadFormat, ReadRequest,
    ScreenshotRequest, SessionInfo, SessionOptions, SnapshotRequest, Target, WaitUntil,
};

/// Serves `body` as a complete HTML document on loopback, and returns its URL.
///
/// One connection per request and no routing: every test wants exactly one page,
/// and a server with fewer moving parts is one that cannot fail in a way that
/// looks like the module failing.
async fn serve(body: &str) -> String {
    let document = format!(
        "<!doctype html><html><head><title>tinybrowser</title></head><body>{body}</body></html>"
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binds a loopback port");
    let address = listener.local_addr().expect("has an address");

    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let document = document.clone();
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

                // Read and discard the request: a server that replies without
                // draining can have the write fail on it.
                let mut scratch = [0_u8; 2048];
                let _ = stream.read(&mut scratch).await;

                let response = format!(
                    "HTTP/1.1 200 OK\r\n\
                     Content-Type: text/html; charset=utf-8\r\n\
                     Content-Length: {}\r\n\
                     Connection: close\r\n\r\n{document}",
                    document.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });

    format!("http://{address}/")
}

/// Whether this run opted in.
///
/// Returning `false` here is the *only* skip in this file, and it is the one a
/// reader can see at the top of every test.
fn enabled() -> bool {
    std::env::var("TINYBROWSER_LIVE_TESTS").is_ok_and(|value| !value.is_empty() && value != "0")
}

/// The session options these tests open with.
///
/// `TINYBROWSER_TEST_ARGS` is how a host whose Chrome needs extra flags supplies
/// them without those flags becoming this module's defaults.
fn options() -> SessionOptions {
    SessionOptions {
        args: std::env::var("TINYBROWSER_TEST_ARGS")
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_string)
            .collect(),
        ..SessionOptions::default()
    }
}

/// An engine with one session on `body`.
async fn on(body: &str) -> (Browser, SessionInfo) {
    let browser = Browser::new();
    let session = browser
        .open_session(options())
        .await
        .expect("a browser is available; see this file's docs");

    browser
        .navigate(&session.id, &NavigateRequest::new(serve(body).await))
        .await
        .expect("navigates");

    (browser, session)
}

#[tokio::test]
async fn live_navigating_reports_where_the_page_landed() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<h1>Hello</h1>").await;

    let state = browser
        .navigate(
            &session.id,
            &NavigateRequest {
                url: serve("<h1>Second</h1>").await,
                wait_until: WaitUntil::DomContentLoaded,
                timeout_ms: Some(10_000),
            },
        )
        .await
        .expect("navigates");

    assert_eq!(state.title, "tinybrowser");
    assert!(state.url.starts_with("http://127.0.0.1:"), "{}", state.url);
    // The status comes from the response the navigation actually received, which
    // is the one thing a caller cannot recover from the URL alone.
    assert_eq!(state.status, Some(200));
    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_a_snapshot_ref_resolves_to_the_element_it_named() {
    if !enabled() {
        return;
    }
    // The single most important property in the crate: an agent acts on the ref
    // it read, and the node behind it is the one the snapshot described.
    let (browser, session) = on("<button id='a'>Alpha</button><button id='b'>Beta</button>\
         <script>document.body.onclick = (e) => { document.title = e.target.id; };</script>")
    .await;

    let snapshot = browser
        .snapshot(&session.id, &SnapshotRequest::interactive())
        .await
        .expect("snapshots");

    let beta = snapshot
        .refs
        .iter()
        .find(|element| element.name == "Beta")
        .expect("the second button is in the snapshot");

    let outcome = browser
        .perform(
            &session.id,
            &Action::Click {
                target: Target::reference(&beta.id),
                new_tab: false,
            },
        )
        .await
        .expect("clicks");

    assert_eq!(outcome.page.title, "b");
    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_a_ref_from_a_previous_snapshot_is_refused() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<button>Alpha</button>").await;

    let first = browser
        .snapshot(&session.id, &SnapshotRequest::interactive())
        .await
        .expect("snapshots");
    let reference = first.refs.first().expect("a ref").id.clone();

    // Navigating replaces the document, so every outstanding ref names a node
    // that is gone. Acting on one must say so rather than resolve to whatever
    // now occupies that position.
    browser
        .navigate(
            &session.id,
            &NavigateRequest::new(serve("<button>Beta</button>").await),
        )
        .await
        .expect("navigates");

    let error = browser
        .perform(
            &session.id,
            &Action::Click {
                target: Target::reference(&reference),
                new_tab: false,
            },
        )
        .await
        .expect_err("refused");

    assert!(matches!(error, Error::StaleRef { .. }), "{error}");
    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_a_covered_element_is_refused_and_the_cover_is_named() {
    if !enabled() {
        return;
    }
    // A click dispatched at a point a banner covers is delivered to the banner,
    // and without this check the caller is told it succeeded.
    let (browser, session) = on(
        "<button id='target' style='position:fixed;top:50px;left:50px'>Buy</button>\
         <div id='banner' style='position:fixed;inset:0;background:#000'>Consent</div>",
    )
    .await;

    let error = browser
        .perform(
            &session.id,
            &Action::Click {
                target: Target::selector("#target"),
                new_tab: false,
            },
        )
        .await
        .expect_err("refused");

    assert!(matches!(error, Error::NotActionable { .. }), "{error}");
    assert!(error.to_string().contains("banner"), "{error}");
    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_filling_replaces_the_value_and_fires_the_page_handlers() {
    if !enabled() {
        return;
    }
    // `Fill` clears and types through real key events, so a field that reacts to
    // input sees them. Assigning `value` directly would not.
    let (browser, session) = on("<input id='q' value='old'>\
         <script>document.getElementById('q').addEventListener('input', () => { \
            document.title = document.getElementById('q').value; });</script>")
    .await;

    browser
        .perform(
            &session.id,
            &Action::Fill {
                target: Target::selector("#q"),
                value: "new value".to_string(),
            },
        )
        .await
        .expect("fills");

    let outcome = browser
        .perform(
            &session.id,
            &Action::GetText {
                target: Target::selector("#q"),
            },
        )
        .await
        .expect("reads");

    assert_eq!(outcome.value, serde_json::json!("new value"));
    assert_eq!(outcome.page.title, "new value");
    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_pressing_a_key_reaches_the_page() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<input id='q'>\
         <script>document.getElementById('q').addEventListener('keydown', (e) => { \
            document.title = e.key + ':' + e.keyCode; });</script>")
    .await;

    browser
        .perform(
            &session.id,
            &Action::Focus {
                target: Target::selector("#q"),
            },
        )
        .await
        .expect("focuses");
    let outcome = browser
        .perform(
            &session.id,
            &Action::Press {
                key: "Enter".to_string(),
            },
        )
        .await
        .expect("presses");

    // Both halves: a page reading `key` and a page reading the legacy `keyCode`
    // must each see the right thing.
    assert_eq!(outcome.page.title, "Enter:13");
    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_a_locator_finds_an_element_by_what_it_says() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<button>Cancel</button><button>Submit order</button>\
         <script>document.body.onclick = (e) => { document.title = e.target.innerText; };</script>")
    .await;

    let outcome = browser
        .perform(
            &session.id,
            &Action::Click {
                target: Target::locator(Locator::new(LocateBy::Role, "button").with_name("Submit")),
                new_tab: false,
            },
        )
        .await
        .expect("clicks");

    assert_eq!(outcome.page.title, "Submit order");
    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_a_missing_element_is_reported_rather_than_guessed_at() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<p>nothing here</p>").await;

    let error = browser
        .perform(
            &session.id,
            &Action::Click {
                target: Target::selector("#absent"),
                new_tab: false,
            },
        )
        .await
        .expect_err("refused");

    assert!(matches!(error, Error::NoSuchElement { .. }), "{error}");
    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_is_visible_answers_false_rather_than_failing() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<p id='shown'>here</p>").await;

    for (selector, expected) in [("#shown", true), ("#absent", false)] {
        let outcome = browser
            .perform(
                &session.id,
                &Action::IsVisible {
                    target: Target::selector(selector),
                },
            )
            .await
            .expect("answers");
        assert_eq!(outcome.value, serde_json::json!(expected), "{selector}");
    }

    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_reading_a_page_produces_markdown_a_model_can_use() {
    if !enabled() {
        return;
    }
    let (browser, session) = on(
        "<h1>Title</h1><p>Body text.</p><a href='https://example.com/x'>Link</a>\
         <script>console.log('not content')</script><style>p{color:red}</style>",
    )
    .await;

    let text = browser
        .read_page(&session.id, &ReadRequest::default())
        .await
        .expect("reads");

    assert_eq!(text.format, ReadFormat::Markdown);
    assert!(text.content.contains("# Title"), "{}", text.content);
    assert!(text.content.contains("Body text."), "{}", text.content);
    assert!(
        text.content.contains("[Link](https://example.com/x)"),
        "{}",
        text.content
    );
    // Scripts and styles are chrome, not content.
    assert!(!text.content.contains("not content"), "{}", text.content);
    assert!(!text.content.contains("color:red"), "{}", text.content);

    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_evaluating_returns_a_value_and_reports_a_throw() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<p>page</p>").await;

    let value = browser
        .evaluate(&session.id, &tinybrowser::EvaluateRequest::new("1 + 1"))
        .await
        .expect("evaluates");
    assert_eq!(value, serde_json::json!(2));

    // A throw arrives from CDP as a *successful* reply. Reading it wrong turns
    // every page error into a silent `null`.
    let error = browser
        .evaluate(
            &session.id,
            &tinybrowser::EvaluateRequest::new("nope.missing"),
        )
        .await
        .expect_err("refused");
    assert!(matches!(error, Error::PageError { .. }), "{error}");

    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_a_screenshot_round_trips_through_the_output_handle() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<h1 style='font-size:64px'>Shot</h1>").await;

    let handle = browser
        .screenshot(&session.id, &ScreenshotRequest::default())
        .await
        .expect("captures");

    assert_eq!(handle.media_type, "image/png");
    assert!(handle.total_bytes > 0);

    let mut collected = Vec::new();
    let mut offset = 0;
    loop {
        let chunk = browser
            .read_output(&handle.id, offset, 256 * 1024)
            .await
            .expect("reads");
        let bytes = base64_decode(&chunk.data);
        offset += bytes.len() as u64;
        collected.extend_from_slice(&bytes);
        if chunk.eof {
            break;
        }
    }

    assert_eq!(collected.len() as u64, handle.total_bytes);
    // The PNG signature: what came back is an image, not a truncated frame.
    assert_eq!(&collected[..8], b"\x89PNG\r\n\x1a\n");

    browser.release_output(&handle.id).await.expect("releases");
    assert!(browser.read_output(&handle.id, 0, 1024).await.is_err());

    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_an_origin_allowlist_refuses_what_it_does_not_admit() {
    if !enabled() {
        return;
    }
    let browser = Browser::new();
    let session = browser
        .open_session(SessionOptions {
            allowed_origins: vec!["https://example.com".to_string()],
            ..options()
        })
        .await
        .expect("a browser is available; see this file's docs");

    let error = browser
        .navigate(
            &session.id,
            &NavigateRequest::new("https://elsewhere.test/"),
        )
        .await
        .expect_err("refused");

    assert!(matches!(error, Error::BlockedByPolicy { .. }), "{error}");
    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_sessions_are_listed_and_close_cleanly() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<h1>One</h1>").await;

    let listed = browser.list_sessions().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, session.id);
    assert!(
        listed[0].launched,
        "a launched browser reports itself as one"
    );

    browser.close_session(&session.id).await.expect("closes");
    assert!(browser.list_sessions().await.is_empty());
}

/// Standard base64, for reassembling a held output.
fn base64_decode(encoded: &str) -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .expect("the module encodes standard base64")
}
