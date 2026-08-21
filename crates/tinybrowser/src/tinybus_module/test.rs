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
            (tinybrowser_bus::OutputId::new("never-captured"), 0u64, 1024u64),
        )
        .await;

    let Err(error) = result else {
        panic!("reading an output that was never captured unexpectedly succeeded");
    };
    assert!(error.to_string().contains("no such output"), "{error}");
    Ok(())
}
