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
    Action, Browser, Error, ImageFormat, LocateBy, Locator, NavigateRequest, ReadFormat,
    ReadRequest, ScreenshotRequest, ScrollDirection, SessionInfo, SessionOptions, SnapshotRequest,
    Target, WaitState, WaitUntil,
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

#[tokio::test]
async fn live_double_click_and_hover_reach_their_handlers() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<div id='t' style='padding:40px'>Target</div>\
         <script>const t = document.getElementById('t');\
         t.ondblclick = () => { document.title = 'double'; };\
         t.onmouseover = () => { document.title = 'hovered'; };</script>")
    .await;

    let hovered = browser
        .perform(
            &session.id,
            &Action::Hover {
                target: Target::selector("#t"),
            },
        )
        .await
        .expect("hovers");
    assert_eq!(hovered.page.title, "hovered");

    let clicked = browser
        .perform(
            &session.id,
            &Action::DoubleClick {
                target: Target::selector("#t"),
            },
        )
        .await
        .expect("double-clicks");
    assert_eq!(clicked.page.title, "double");

    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_typing_key_by_key_reaches_a_field_that_reacts_to_each_one() {
    if !enabled() {
        return;
    }
    // The distinction `Type` exists for: an autocomplete that fires per
    // keystroke never sees the input a bulk value assignment skips.
    let (browser, session) = on("<input id='q'><script>let count = 0;\
         document.getElementById('q').addEventListener('input', () => { \
            count += 1; document.title = String(count); });</script>")
    .await;

    browser
        .perform(
            &session.id,
            &Action::Type {
                target: Some(Target::selector("#q")),
                text: "abc".to_string(),
                delay_ms: Some(1),
            },
        )
        .await
        .expect("types");

    let outcome = browser
        .perform(
            &session.id,
            &Action::GetAttribute {
                target: Target::selector("#q"),
                attribute: "id".to_string(),
            },
        )
        .await
        .expect("reads an attribute");

    assert_eq!(outcome.value, serde_json::json!("q"));
    assert_eq!(outcome.page.title, "3", "one input event per keystroke");

    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_typing_without_a_target_goes_to_whatever_has_focus() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<input id='q' autofocus><script>\
         document.getElementById('q').addEventListener('input', (e) => { \
            document.title = e.target.value; });</script>")
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
            &Action::Type {
                target: None,
                text: "typed".to_string(),
                delay_ms: None,
            },
        )
        .await
        .expect("types");

    assert_eq!(outcome.page.title, "typed");
    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_an_absent_attribute_reads_as_null() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<p id='p'>text</p>").await;

    let outcome = browser
        .perform(
            &session.id,
            &Action::GetAttribute {
                target: Target::selector("#p"),
                attribute: "data-missing".to_string(),
            },
        )
        .await
        .expect("reads");

    assert!(outcome.value.is_null());
    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_selecting_an_option_fires_the_change_a_form_listens_for() {
    if !enabled() {
        return;
    }
    let (browser, session) = on(
        "<select id='s'><option value='a'>Alpha</option><option value='b'>Beta</option></select>\
         <script>document.getElementById('s').addEventListener('change', (e) => { \
            document.title = e.target.value; });</script>",
    )
    .await;

    let outcome = browser
        .perform(
            &session.id,
            &Action::Select {
                target: Target::selector("#s"),
                values: vec!["b".to_string()],
            },
        )
        .await
        .expect("selects");
    assert_eq!(outcome.page.title, "b");

    // An option nothing matches is an error, not a silent no-op that leaves the
    // form on its previous value while reporting success.
    let error = browser
        .perform(
            &session.id,
            &Action::Select {
                target: Target::selector("#s"),
                values: vec!["nonexistent".to_string()],
            },
        )
        .await
        .expect_err("refused");
    assert!(matches!(error, Error::PageError { .. }), "{error}");

    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_check_sets_a_state_rather_than_toggling_it() {
    if !enabled() {
        return;
    }
    // Called twice with the same value. A toggle would leave the box in the
    // opposite state to the one that was asked for.
    let (browser, session) = on("<input type='checkbox' id='c'>").await;

    for _ in 0..2 {
        browser
            .perform(
                &session.id,
                &Action::Check {
                    target: Target::selector("#c"),
                    checked: true,
                },
            )
            .await
            .expect("checks");
    }

    let checked = browser
        .evaluate(
            &session.id,
            &tinybrowser::EvaluateRequest::new("document.getElementById('c').checked"),
        )
        .await
        .expect("evaluates");
    assert_eq!(checked, serde_json::json!(true));

    browser
        .perform(
            &session.id,
            &Action::Check {
                target: Target::selector("#c"),
                checked: false,
            },
        )
        .await
        .expect("unchecks");
    let unchecked = browser
        .evaluate(
            &session.id,
            &tinybrowser::EvaluateRequest::new("document.getElementById('c').checked"),
        )
        .await
        .expect("evaluates");
    assert_eq!(unchecked, serde_json::json!(false));

    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_scrolling_moves_the_page_and_an_element_that_scrolls_within_it() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<div id='box' style='height:60px;overflow:auto'>\
           <div style='height:2000px'>tall</div></div>\
         <div style='height:4000px'>page</div>")
    .await;

    for direction in [ScrollDirection::Down, ScrollDirection::Bottom] {
        browser
            .perform(
                &session.id,
                &Action::Scroll {
                    direction,
                    pixels: None,
                    target: None,
                },
            )
            .await
            .expect("scrolls the page");
    }
    let page_offset = browser
        .evaluate(&session.id, &tinybrowser::EvaluateRequest::new("scrollY"))
        .await
        .expect("evaluates");
    assert!(page_offset.as_f64().unwrap_or(0.0) > 0.0, "{page_offset}");

    browser
        .perform(
            &session.id,
            &Action::Scroll {
                direction: ScrollDirection::Down,
                pixels: Some(200),
                target: Some(Target::selector("#box")),
            },
        )
        .await
        .expect("scrolls the element");
    let box_offset = browser
        .evaluate(
            &session.id,
            &tinybrowser::EvaluateRequest::new("document.getElementById('box').scrollTop"),
        )
        .await
        .expect("evaluates");
    assert_eq!(box_offset, serde_json::json!(200));

    browser
        .perform(
            &session.id,
            &Action::Scroll {
                direction: ScrollDirection::Top,
                pixels: None,
                target: None,
            },
        )
        .await
        .expect("scrolls back");
    let back = browser
        .evaluate(&session.id, &tinybrowser::EvaluateRequest::new("scrollY"))
        .await
        .expect("evaluates");
    assert_eq!(back, serde_json::json!(0));

    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_waiting_returns_when_the_condition_holds() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<div id='late' style='display:none'>Ready</div>\
         <script>setTimeout(() => { \
            document.getElementById('late').style.display = 'block'; }, 150);</script>")
    .await;

    browser
        .perform(
            &session.id,
            &Action::WaitFor {
                target: Some(Target::selector("#late")),
                text: None,
                state: WaitState::Visible,
                ms: None,
                timeout_ms: Some(5_000),
            },
        )
        .await
        .expect("waits for visibility");

    browser
        .perform(
            &session.id,
            &Action::WaitFor {
                target: None,
                text: Some("Ready".to_string()),
                state: WaitState::Visible,
                ms: None,
                timeout_ms: Some(5_000),
            },
        )
        .await
        .expect("waits for text");

    browser
        .perform(
            &session.id,
            &Action::WaitFor {
                target: Some(Target::selector("#absent")),
                text: None,
                state: WaitState::Detached,
                ms: None,
                timeout_ms: Some(5_000),
            },
        )
        .await
        .expect("an absent element already satisfies detached");

    // A flat delay is the only thing a caller with neither a target nor text can
    // have meant.
    browser
        .perform(
            &session.id,
            &Action::WaitFor {
                target: None,
                text: None,
                state: WaitState::Visible,
                ms: Some(10),
                timeout_ms: None,
            },
        )
        .await
        .expect("waits a flat delay");

    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_waiting_for_something_that_never_happens_times_out() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<p>static</p>").await;

    let error = browser
        .perform(
            &session.id,
            &Action::WaitFor {
                target: None,
                text: Some("never appears".to_string()),
                state: WaitState::Visible,
                ms: None,
                timeout_ms: Some(300),
            },
        )
        .await
        .expect_err("times out");

    assert!(matches!(error, Error::Timeout { .. }), "{error}");
    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_history_moves_back_and_forward_and_refuses_the_ends() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<h1>First</h1>").await;
    let first = browser.list_sessions().await[0].url.clone();

    browser
        .navigate(
            &session.id,
            &NavigateRequest::new(serve("<h1>Second</h1>").await),
        )
        .await
        .expect("navigates");

    let back = browser
        .perform(&session.id, &Action::Back)
        .await
        .expect("goes back");
    assert_eq!(back.page.url, first);

    let forward = browser
        .perform(&session.id, &Action::Forward)
        .await
        .expect("goes forward");
    assert_ne!(forward.page.url, first);

    // At the end of the history there is nothing to go to, and saying so is
    // better than a no-op that reports success.
    let error = browser
        .perform(&session.id, &Action::Forward)
        .await
        .expect_err("refused");
    assert!(matches!(error, Error::InvalidInput { .. }), "{error}");

    browser
        .perform(&session.id, &Action::Reload)
        .await
        .expect("reloads");

    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_reading_a_page_as_text_drops_the_markdown_syntax() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<h1>Title</h1><a href='https://example.com/x'>Link</a>").await;

    let text = browser
        .read_page(
            &session.id,
            &ReadRequest {
                format: ReadFormat::Text,
                ..ReadRequest::default()
            },
        )
        .await
        .expect("reads");

    assert!(text.content.contains("Title"), "{}", text.content);
    assert!(!text.content.contains("# Title"), "{}", text.content);
    assert!(!text.content.contains("]("), "{}", text.content);

    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_reading_a_selector_reads_only_that_subtree() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<div id='keep'>kept</div><div id='drop'>dropped</div>").await;

    let text = browser
        .read_page(
            &session.id,
            &ReadRequest {
                selector: Some("#keep".to_string()),
                ..ReadRequest::default()
            },
        )
        .await
        .expect("reads");

    assert!(text.content.contains("kept"), "{}", text.content);
    assert!(!text.content.contains("dropped"), "{}", text.content);

    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_reading_a_selector_that_matches_nothing_is_reported() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<p>only this</p>").await;

    let error = browser
        .read_page(
            &session.id,
            &ReadRequest {
                selector: Some("#absent".to_string()),
                ..ReadRequest::default()
            },
        )
        .await
        .expect_err("refused");

    assert!(matches!(error, Error::NoSuchElement { .. }), "{error}");
    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_reading_as_html_returns_the_live_dom() {
    if !enabled() {
        return;
    }
    // Not the response body: what the page became after its scripts ran.
    let (browser, session) = on(
        "<div id='root'></div><script>document.getElementById('root').innerHTML = \
            '<span>injected</span>';</script>",
    )
    .await;

    let html = browser
        .read_page(
            &session.id,
            &ReadRequest {
                format: ReadFormat::Html,
                ..ReadRequest::default()
            },
        )
        .await
        .expect("reads");
    assert!(html.content.contains("injected"), "{}", html.content);

    let fragment = browser
        .read_page(
            &session.id,
            &ReadRequest {
                format: ReadFormat::Html,
                selector: Some("#root".to_string()),
                ..ReadRequest::default()
            },
        )
        .await
        .expect("reads");
    assert!(
        fragment.content.starts_with("<div id=\"root\""),
        "{}",
        fragment.content
    );

    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_reading_reports_truncation_rather_than_hiding_it() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<p>a long enough paragraph to cut</p>").await;

    let text = browser
        .read_page(
            &session.id,
            &ReadRequest {
                max_chars: 5,
                ..ReadRequest::default()
            },
        )
        .await
        .expect("reads");

    assert!(text.truncated);
    assert_eq!(text.content.chars().count(), 5);
    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_a_snapshot_can_be_scoped_and_bounded() {
    if !enabled() {
        return;
    }
    let (browser, session) =
        on("<div id='panel'><button>Inside</button></div><button>Outside</button>").await;

    let scoped = browser
        .snapshot(
            &session.id,
            &SnapshotRequest {
                selector: Some("#panel".to_string()),
                ..SnapshotRequest::interactive()
            },
        )
        .await
        .expect("snapshots");
    assert!(scoped.tree.contains("Inside"), "{}", scoped.tree);
    assert!(!scoped.tree.contains("Outside"), "{}", scoped.tree);

    let bounded = browser
        .snapshot(
            &session.id,
            &SnapshotRequest {
                max_chars: 10,
                ..SnapshotRequest::default()
            },
        )
        .await
        .expect("snapshots");
    assert!(bounded.truncated);

    // The generation moves for every snapshot, so refs from the first are dead.
    assert!(bounded.sequence > scoped.sequence);

    let error = browser
        .snapshot(
            &session.id,
            &SnapshotRequest {
                selector: Some("#absent".to_string()),
                ..SnapshotRequest::default()
            },
        )
        .await
        .expect_err("refused");
    assert!(matches!(error, Error::NoSuchElement { .. }), "{error}");

    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_screenshots_cover_the_page_the_element_and_the_lossy_formats() {
    if !enabled() {
        return;
    }
    let (browser, session) = on(
        "<div id='box' style='width:100px;height:80px;background:#333'></div>\
            <div style='height:3000px'></div>",
    )
    .await;

    let full = browser
        .screenshot(
            &session.id,
            &ScreenshotRequest {
                full_page: true,
                ..ScreenshotRequest::default()
            },
        )
        .await
        .expect("captures the page");
    assert!(
        full.height > 1000,
        "a full-page capture is taller than the viewport"
    );

    let element = browser
        .screenshot(
            &session.id,
            &ScreenshotRequest {
                target: Some(Target::selector("#box")),
                format: ImageFormat::Jpeg,
                quality: Some(70),
                ..ScreenshotRequest::default()
            },
        )
        .await
        .expect("captures the element");
    assert_eq!(element.media_type, "image/jpeg");
    assert_eq!((element.width, element.height), (100, 80));

    // Quality outside the range is the caller's mistake, and saying so is more
    // useful than a protocol error from the browser.
    let error = browser
        .screenshot(
            &session.id,
            &ScreenshotRequest {
                format: ImageFormat::Webp,
                quality: Some(0),
                ..ScreenshotRequest::default()
            },
        )
        .await
        .expect_err("refused");
    assert!(matches!(error, Error::InvalidInput { .. }), "{error}");

    for handle in [full, element] {
        browser.release_output(&handle.id).await.expect("releases");
    }
    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_the_session_limit_refuses_the_one_past_it() {
    if !enabled() {
        return;
    }
    let browser = Browser::with_session_limit(1);
    let first = browser
        .open_session(options())
        .await
        .expect("a browser is available; see this file's docs");

    let error = browser.open_session(options()).await.expect_err("refused");
    assert!(matches!(error, Error::LimitExceeded { .. }), "{error}");

    // And the limit is a bound on what is open, not a lifetime cap.
    browser.close_session(&first.id).await.expect("closes");
    let second = browser.open_session(options()).await.expect("opens");

    browser.shutdown().await;
    assert!(browser.list_sessions().await.is_empty());
    let _ = second;
}

#[tokio::test]
async fn live_a_session_can_attach_to_a_browser_it_did_not_launch() {
    if !enabled() {
        return;
    }
    // The arrangement a container deployment uses: one browser, many sessions,
    // and closing a session leaves the browser somebody else owns running.
    let host = Browser::new();
    let launched = host
        .open_session(options())
        .await
        .expect("a browser is available; see this file's docs");

    let attached_engine = Browser::new();
    let attached = attached_engine
        .open_session(SessionOptions {
            endpoint: Some(launched.endpoint.clone()),
            ..options()
        })
        .await
        .expect("attaches");

    assert!(!attached.launched, "an attached session did not launch it");
    attached_engine
        .close_session(&attached.id)
        .await
        .expect("closes");

    // The browser is still there, which is the whole point.
    host.navigate(
        &launched.id,
        &NavigateRequest::new(serve("<p>still here</p>").await),
    )
    .await
    .expect("the launched browser survived");

    host.shutdown().await;
}

#[tokio::test]
async fn live_an_invalid_selector_is_the_callers_mistake_not_the_pages() {
    if !enabled() {
        return;
    }
    // The page throws a `SyntaxError`, which as a page error would read as the
    // site's fault. It is not: the caller wrote a selector that is not CSS.
    let (browser, session) = on("<p>page</p>").await;

    let error = browser
        .perform(
            &session.id,
            &Action::Click {
                target: Target::selector("<<not a selector>>"),
                new_tab: false,
            },
        )
        .await
        .expect_err("refused");

    assert!(matches!(error, Error::InvalidInput { .. }), "{error}");
    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_a_locator_that_matches_nothing_names_what_it_looked_for() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<button>Cancel</button>").await;

    let error = browser
        .perform(
            &session.id,
            &Action::Click {
                target: Target::locator(Locator::new(LocateBy::Text, "Definitely Not Here")),
                new_tab: false,
            },
        )
        .await
        .expect_err("refused");

    assert!(matches!(error, Error::NoSuchElement { .. }), "{error}");
    assert!(error.to_string().contains("Definitely Not Here"), "{error}");

    // An empty locator is refused before the page is asked anything.
    let empty = browser
        .perform(
            &session.id,
            &Action::Click {
                target: Target::locator(Locator::new(LocateBy::Text, "   ")),
                new_tab: false,
            },
        )
        .await
        .expect_err("refused");
    assert!(matches!(empty, Error::InvalidInput { .. }), "{empty}");

    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_every_locator_dimension_finds_its_element() {
    if !enabled() {
        return;
    }
    // Each dimension crosses into JavaScript as a string. A rename on one side
    // alone produces "unknown locator dimension" at runtime and nothing at
    // compile time, so every one is exercised here at least once.
    let (browser, session) = on(
        "<input id='labelled' aria-label='Email address'>\
         <input id='placeheld' placeholder='Search products'>\
         <button data-testid='cart'>Cart</button>\
         <img src='data:image/gif;base64,R0lGODlhAQABAAAAACw=' alt='Company logo' width='20' height='20'>\
         <span title='Tooltip text'>hover me</span>",
    )
    .await;

    for (by, value) in [
        (LocateBy::Label, "Email address"),
        (LocateBy::Placeholder, "Search products"),
        (LocateBy::TestId, "cart"),
        (LocateBy::AltText, "Company logo"),
        (LocateBy::Title, "Tooltip text"),
    ] {
        let outcome = browser
            .perform(
                &session.id,
                &Action::IsVisible {
                    target: Target::locator(Locator::new(by, value)),
                },
            )
            .await
            .unwrap_or_else(|error| panic!("{by:?} {value:?}: {error}"));

        assert_eq!(outcome.value, serde_json::json!(true), "{by:?} {value:?}");
    }

    // And an exact match narrows where a substring would not.
    let exact = browser
        .perform(
            &session.id,
            &Action::IsVisible {
                target: Target::locator(Locator {
                    exact: true,
                    ..Locator::new(LocateBy::Placeholder, "Search")
                }),
            },
        )
        .await
        .expect("answers");
    assert_eq!(exact.value, serde_json::json!(false));

    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_snapshotting_a_node_the_tree_does_not_contain_is_reported() {
    if !enabled() {
        return;
    }
    // `<style>` resolves as an element and is not in the accessibility tree,
    // which is exactly the case where a scoped snapshot has nothing to render.
    let (browser, session) = on("<style>p{color:red}</style><p>text</p>").await;

    let error = browser
        .snapshot(
            &session.id,
            &SnapshotRequest {
                selector: Some("style".to_string()),
                ..SnapshotRequest::default()
            },
        )
        .await
        .expect_err("refused");

    assert!(matches!(error, Error::NoSuchElement { .. }), "{error}");
    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_a_click_in_a_new_tab_leaves_the_session_where_it_was() {
    if !enabled() {
        return;
    }
    // A documented limitation rather than a feature: the modifier reaches the
    // page and the browser opens a tab, but the session keeps driving the one
    // it has. Adopting the new tab is on the roadmap; silently appearing to
    // follow it would be worse than this.
    let (browser, session) = on("<a href='https://example.com/'>Away</a>").await;
    let before = browser.list_sessions().await[0].url.clone();

    let outcome = browser
        .perform(
            &session.id,
            &Action::Click {
                target: Target::selector("a"),
                new_tab: true,
            },
        )
        .await
        .expect("clicks");

    assert_eq!(outcome.page.url, before);
    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_navigating_somewhere_unreachable_is_a_page_error() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<p>page</p>").await;

    // A hostname that cannot resolve, by construction: `.invalid` is reserved
    // for exactly this and never resolves anywhere.
    let error = browser
        .navigate(
            &session.id,
            &NavigateRequest {
                url: "https://nothing.invalid/".to_string(),
                wait_until: WaitUntil::Load,
                timeout_ms: Some(10_000),
            },
        )
        .await
        .expect_err("refused");

    assert!(
        matches!(error, Error::PageError { .. } | Error::Timeout { .. }),
        "{error}"
    );
    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_a_session_can_be_told_what_size_to_render_at() {
    if !enabled() {
        return;
    }
    // Layout decides what a snapshot contains: a headless default would serve
    // the mobile tree, and an agent then cannot find the navigation an operator
    // sees.
    let browser = Browser::new();
    let session = browser
        .open_session(SessionOptions {
            // Narrow but not `mobile`: mobile emulation gives a page with no
            // viewport meta tag the legacy 980px layout viewport, which is
            // correct browser behaviour and would make this assertion about
            // Chrome's compatibility rules rather than about the module.
            viewport: tinybrowser::Viewport {
                device_scale_factor: 2.0,
                ..tinybrowser::Viewport::desktop(390, 844)
            },
            user_agent: Some("tinybrowser-test/1.0".to_string()),
            ..options()
        })
        .await
        .expect("a browser is available; see this file's docs");

    browser
        .navigate(
            &session.id,
            &NavigateRequest::new(serve("<p>sized</p>").await),
        )
        .await
        .expect("navigates");

    let size = browser
        .evaluate(
            &session.id,
            &tinybrowser::EvaluateRequest::new(
                "[innerWidth, devicePixelRatio, navigator.userAgent]",
            ),
        )
        .await
        .expect("evaluates");

    assert_eq!(size[0], serde_json::json!(390));
    assert_eq!(size[1], serde_json::json!(2.0));
    assert_eq!(size[2], serde_json::json!("tinybrowser-test/1.0"));

    browser.close_session(&session.id).await.expect("closes");
}

#[tokio::test]
async fn live_navigation_settles_at_every_wait_mode() {
    if !enabled() {
        return;
    }
    let (browser, session) = on("<p>first</p>").await;

    for wait_until in [
        WaitUntil::Commit,
        WaitUntil::DomContentLoaded,
        WaitUntil::Load,
        WaitUntil::NetworkIdle,
    ] {
        let url = serve("<p>settled</p>").await;
        browser
            .navigate(
                &session.id,
                &NavigateRequest {
                    url,
                    wait_until,
                    timeout_ms: Some(10_000),
                },
            )
            .await
            .unwrap_or_else(|error| panic!("{wait_until:?}: {error}"));
    }

    browser.close_session(&session.id).await.expect("closes");
}
