//! Tests for the `TinyBus` module adapter and its declared surface.
//!
//! Every one of these runs against a real in-memory broker with a real client on
//! the other side, because the parts worth testing here only exist once a frame
//! has been serialized: whether the dispatch table matches the contract, whether
//! a payload survives the round trip, and whether a failure arrives carrying the
//! error name a host will match on.
//!
//! None of them needs a browser. The members exercised are the ones that reach a
//! decision before a session is looked up, which is exactly the layer this
//! adapter is responsible for.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tinybrowser_bus::{SessionId, errors, names};
use tinybus::broker::Broker;
use tinybus::transport::memory::MemoryBus;
use tinybus::{Connection, Interface};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::{BrowserService, setup};

/// A broker, the module, and a client proxy pointed at it.
///
/// The module's own connection comes back with the proxy and has to be held:
/// dropping it releases the well-known name, and every call then fails with
/// `NameHasNoOwner` rather than reaching the service that was just set up.
async fn connected() -> tinybus::Result<(tinybus::Proxy, Connection)> {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());

    let service = Connection::connect(bus.connect().await?).await?;
    setup(service.clone()).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    Ok((proxy, service))
}

#[test]
fn declared_methods_match_the_dispatch_table() {
    // The contract lists the members; the macro derives them from the `impl`.
    // This is what stops a member added to one and forgotten in the other from
    // surfacing as an unknown method in a host at runtime.
    let methods = BrowserService
        .members()
        .into_iter()
        .map(|member| member.to_string())
        .collect::<Vec<_>>();

    assert_eq!(methods, names::METHODS.to_vec());
}

#[test]
fn the_served_interface_name_matches_the_contract() {
    assert_eq!(BrowserService.name().to_string(), names::INTERFACE);
}

#[tokio::test]
async fn the_module_reports_the_contract_version_it_was_built_against() -> tinybus::Result<()> {
    let (proxy, _module) = connected().await?;
    let version: (u32, u32) = proxy.call(names::methods::CONTRACT_VERSION, ()).await?;

    assert_eq!(version, tinybrowser_bus::CONTRACT_VERSION);
    assert!(tinybrowser_bus::is_compatible(version));
    Ok(())
}

#[tokio::test]
async fn a_fresh_module_lists_no_sessions() -> tinybus::Result<()> {
    let (proxy, _module) = connected().await?;
    let sessions: Vec<tinybrowser_bus::SessionInfo> =
        proxy.call(names::methods::LIST_SESSIONS, ()).await?;

    assert!(sessions.is_empty());
    Ok(())
}

#[tokio::test]
async fn closing_an_unknown_session_succeeds_over_the_bus() -> tinybus::Result<()> {
    // Idempotent teardown is a property of the wire surface, not only of the
    // engine: a host retrying a close after a timeout must not get an error.
    let (proxy, _module) = connected().await?;
    proxy
        .call::<()>(
            names::methods::CLOSE_SESSION,
            (SessionId::new("never-opened"),),
        )
        .await?;

    Ok(())
}

#[tokio::test]
async fn a_failure_arrives_carrying_its_wire_error_name() -> tinybus::Result<()> {
    // The name is the whole point of the mapping: a host decides what to show a
    // model by matching on it, and a failure that arrives as generic prose sends
    // it down the wrong recovery path.
    let (proxy, _module) = connected().await?;
    let result = proxy
        .call::<tinybrowser_bus::PageState>(
            names::methods::NAVIGATE,
            (
                SessionId::new("never-opened"),
                tinybrowser_bus::NavigateRequest::new("https://example.com"),
            ),
        )
        .await;

    let Err(error) = result else {
        panic!("navigating in a session that does not exist unexpectedly succeeded");
    };
    assert!(
        error.to_string().contains(errors::NO_SUCH_SESSION)
            || error.to_string().contains("no such session"),
        "{error}"
    );
    Ok(())
}

#[tokio::test]
async fn an_empty_expression_is_refused_over_the_bus() -> tinybus::Result<()> {
    let (proxy, _module) = connected().await?;
    let result = proxy
        .call::<serde_json::Value>(
            names::methods::EVALUATE,
            (
                SessionId::new("never-opened"),
                tinybrowser_bus::EvaluateRequest::new("   "),
            ),
        )
        .await;

    let Err(error) = result else {
        panic!("an empty expression unexpectedly succeeded");
    };
    assert!(error.to_string().contains("expression is empty"), "{error}");
    Ok(())
}

#[tokio::test]
async fn releasing_an_unknown_output_succeeds_over_the_bus() -> tinybus::Result<()> {
    let (proxy, _module) = connected().await?;
    proxy
        .call::<()>(
            names::methods::RELEASE_OUTPUT,
            (tinybrowser_bus::OutputId::new("never-captured"),),
        )
        .await?;

    Ok(())
}

#[tokio::test]
async fn reading_an_unknown_output_reports_it() -> tinybus::Result<()> {
    let (proxy, _module) = connected().await?;
    let result = proxy
        .call::<tinybrowser_bus::OutputChunk>(
            names::methods::READ_OUTPUT,
            (
                tinybrowser_bus::OutputId::new("never-captured"),
                0u64,
                1024u64,
            ),
        )
        .await;

    let Err(error) = result else {
        panic!("reading an output that was never captured unexpectedly succeeded");
    };
    assert!(error.to_string().contains("no such output"), "{error}");
    Ok(())
}

/// Whether this run opted into driving a real browser.
///
/// The same switch `tests/live_chrome.rs` reads, for the same reason: the
/// members below cannot be exercised without a browser, and a machine without
/// one must still pass `cargo test --all-features`.
fn live() -> bool {
    std::env::var("TINYBROWSER_LIVE_TESTS").is_ok_and(|value| !value.is_empty() && value != "0")
}

/// The session options the live tests here open with.
fn options() -> tinybrowser_bus::SessionOptions {
    tinybrowser_bus::SessionOptions {
        args: std::env::var("TINYBROWSER_TEST_ARGS")
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_string)
            .collect(),
        ..tinybrowser_bus::SessionOptions::default()
    }
}

/// Serves one page on loopback and returns its URL.
async fn serve(body: &'static str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binds a loopback port");
    let address = listener.local_addr().expect("has an address");

    tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let mut scratch = [0_u8; 2048];
        let _ = stream.read(&mut scratch).await;
        let document = format!(
            "<!doctype html><html><head><title>bus</title></head><body>{body}</body></html>"
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{document}",
            document.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;
    });

    format!("http://{address}/")
}

#[tokio::test]
async fn live_every_member_answers_over_a_real_bus() -> tinybus::Result<()> {
    if !live() {
        return Ok(());
    }

    // One test rather than a dozen because the expensive part is the browser,
    // and because what is being checked is the *surface*: every member
    // serializes its arguments, reaches the engine, and returns a payload the
    // client can decode. The behaviour behind each one is covered by
    // `tests/live_chrome.rs`.
    let (proxy, _module) = connected().await?;

    let session: tinybrowser_bus::SessionInfo = proxy
        .call(names::methods::OPEN_SESSION, (options(),))
        .await?;
    assert!(session.launched);

    let listed: Vec<tinybrowser_bus::SessionInfo> =
        proxy.call(names::methods::LIST_SESSIONS, ()).await?;
    assert_eq!(listed.len(), 1);

    let page: tinybrowser_bus::PageState = proxy
        .call(
            names::methods::NAVIGATE,
            (
                &session.id,
                tinybrowser_bus::NavigateRequest::new(
                    serve("<h1>Bus</h1><button id='b'>Press</button>").await,
                ),
            ),
        )
        .await?;
    assert_eq!(page.title, "bus");
    assert_eq!(page.status, Some(200));

    let snapshot: tinybrowser_bus::Snapshot = proxy
        .call(
            names::methods::SNAPSHOT,
            (&session.id, tinybrowser_bus::SnapshotRequest::interactive()),
        )
        .await?;
    let reference = snapshot.refs.first().expect("a button in the snapshot");
    assert_eq!(reference.role, "button");

    let outcome: tinybrowser_bus::ActionOutcome = proxy
        .call(
            names::methods::PERFORM,
            (
                &session.id,
                tinybrowser_bus::Action::GetText {
                    target: tinybrowser_bus::Target::reference(&reference.id),
                },
            ),
        )
        .await?;
    assert_eq!(outcome.value, serde_json::json!("Press"));

    let text: tinybrowser_bus::PageText = proxy
        .call(
            names::methods::READ_PAGE,
            (&session.id, tinybrowser_bus::ReadRequest::default()),
        )
        .await?;
    assert!(text.content.contains("Bus"), "{}", text.content);

    let value: serde_json::Value = proxy
        .call(
            names::methods::EVALUATE,
            (
                &session.id,
                tinybrowser_bus::EvaluateRequest::new("document.title"),
            ),
        )
        .await?;
    assert_eq!(value, serde_json::json!("bus"));

    let handle: tinybrowser_bus::OutputRef = proxy
        .call(
            names::methods::SCREENSHOT,
            (&session.id, tinybrowser_bus::ScreenshotRequest::default()),
        )
        .await?;
    assert_eq!(handle.media_type, "image/png");

    let chunk: tinybrowser_bus::OutputChunk = proxy
        .call(
            names::methods::READ_OUTPUT,
            (&handle.id, 0_u64, 64_u64 * 1024),
        )
        .await?;
    assert!(!chunk.data.is_empty());

    proxy
        .call::<()>(names::methods::RELEASE_OUTPUT, (&handle.id,))
        .await?;
    proxy
        .call::<()>(names::methods::CLOSE_SESSION, (&session.id,))
        .await?;

    let after: Vec<tinybrowser_bus::SessionInfo> =
        proxy.call(names::methods::LIST_SESSIONS, ()).await?;
    assert!(after.is_empty());

    Ok(())
}

#[tokio::test]
async fn live_an_engine_failure_keeps_its_error_name_across_the_wire() -> tinybus::Result<()> {
    if !live() {
        return Ok(());
    }

    // The mapping matters most for a failure that came from deep inside the
    // engine rather than from the adapter's own argument checks.
    let (proxy, _module) = connected().await?;
    let session: tinybrowser_bus::SessionInfo = proxy
        .call(names::methods::OPEN_SESSION, (options(),))
        .await?;

    let result = proxy
        .call::<tinybrowser_bus::PageState>(
            names::methods::NAVIGATE,
            (
                &session.id,
                tinybrowser_bus::NavigateRequest::new("file:///etc/passwd"),
            ),
        )
        .await;

    let Err(error) = result else {
        panic!("navigating to a file url unexpectedly succeeded");
    };
    assert!(error.to_string().contains(errors::INVALID_INPUT), "{error}");

    proxy
        .call::<()>(names::methods::CLOSE_SESSION, (&session.id,))
        .await?;
    Ok(())
}
