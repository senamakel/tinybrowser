//! Driving the module the way a host does: over a real `TinyBus`, from a
//! loaded `cdylib`.
//!
//! This is the reference for a host integration. Everything a host has to get
//! right is here in the order it has to happen: load the artifact, wait for it
//! to claim its name, check the contract version, open a session, drive it, and
//! collect a screenshot through the held-output protocol.
//!
//! Run it against a module you have built:
//!
//! ```sh
//! cargo build -p tinybrowser --release
//! cargo run -p tinybrowser --example over_the_bus -- \
//!   target/release/libtinybrowser.so https://example.com
//! ```
//!
//! Unlike `verify_module`, this one drives a browser, so it needs one on the
//! host. `verify_module` is the check CI runs; this one is the thing to read.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use tinybrowser::{
    Action, NavigateRequest, OutputChunk, OutputRef, PageState, ScreenshotRequest, SessionId,
    SessionInfo, SessionOptions, Snapshot, SnapshotRequest, Target, is_compatible, names,
};
use tinybus::broker::Broker;
use tinybus::module::ModuleHost;
use tinybus::transport::memory::MemoryBus;
use tinybus::{Connection, Proxy};

/// How much of a held screenshot to pull per `ReadOutput`.
///
/// Well below the bus frame limit with room for the base64 expansion and the
/// JSON envelope around it.
const READ_CHUNK: u64 = 1024 * 1024;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (module, url) = arguments()?;

    // A private broker with the module attached to it as an ordinary peer. A
    // host does this once at startup; everything after it is an ordinary call.
    let bus = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(bus.clone());
    ModuleHost::new(broker).load_file(&module)?;

    let client = Connection::connect(bus.connect().await?).await?;
    await_name(&client).await?;
    let browser = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;

    // Before anything else. A host that discovers an incompatible contract by
    // making a call cannot tell "this member does not exist" from "this member
    // failed", and will report the wrong thing either way.
    let version: (u32, u32) = browser.call(names::methods::CONTRACT_VERSION, ()).await?;
    if !is_compatible(version) {
        return Err(io::Error::other(format!(
            "module serves contract {version:?}, which this host cannot bind to"
        ))
        .into());
    }

    let session: SessionInfo = browser
        .call(names::methods::OPEN_SESSION, (SessionOptions::default(),))
        .await?;
    println!("session {} on {}", session.id, session.endpoint);

    // Everything from here is wrapped so the session is closed even when a step
    // fails. A leaked session is a browser process nobody will ever end.
    let outcome = drive(&browser, &session.id, &url).await;

    browser
        .call::<()>(names::methods::CLOSE_SESSION, (&session.id,))
        .await?;
    broker_task.abort();
    outcome
}

/// Navigate, snapshot, click the first link, and screenshot the result.
async fn drive(
    browser: &Proxy,
    session: &SessionId,
    url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let page: PageState = browser
        .call(
            names::methods::NAVIGATE,
            (session, NavigateRequest::new(url)),
        )
        .await?;
    println!("{} — {} ({:?})", page.url, page.title, page.status);

    let snapshot: Snapshot = browser
        .call(
            names::methods::SNAPSHOT,
            (session, SnapshotRequest::interactive()),
        )
        .await?;
    println!("\nsnapshot {}:\n{}\n", snapshot.sequence, snapshot.tree);

    if let Some(link) = snapshot.refs.iter().find(|element| element.role == "link") {
        let action = Action::Click {
            target: Target::reference(&link.id),
            new_tab: false,
        };
        let outcome: tinybrowser::ActionOutcome = browser
            .call(names::methods::PERFORM, (session, action))
            .await?;
        println!("clicked {:?} — now at {}", link.name, outcome.page.url);
    }

    let image = collect_screenshot(browser, session).await?;
    println!("screenshot: {} bytes of png", image.len());
    Ok(())
}

/// Captures a screenshot and pulls it back through the held-output protocol.
///
/// The release is in a `defer`-shaped position on purpose: the module bounds
/// what it holds and expires it, but leaving an image to time out costs its
/// budget in the meantime, and the next caller sees a full store rather than a
/// slot.
async fn collect_screenshot(
    browser: &Proxy,
    session: &SessionId,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let handle: OutputRef = browser
        .call(
            names::methods::SCREENSHOT,
            (session, ScreenshotRequest::default()),
        )
        .await?;

    let collected = read_all(browser, &handle).await;

    browser
        .call::<()>(names::methods::RELEASE_OUTPUT, (&handle.id,))
        .await?;

    collected
}

/// Reads a held output to completion.
async fn read_all(
    browser: &Proxy,
    handle: &OutputRef,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use base64::Engine as _;

    // Not `with_capacity(handle.total_bytes)`: the length is a number the module
    // sent us, and trusting a wrong or enormous one for an allocation turns it
    // into an abort rather than an error.
    let mut collected: Vec<u8> = Vec::new();
    let mut offset = 0;

    loop {
        let chunk: OutputChunk = browser
            .call(
                names::methods::READ_OUTPUT,
                (&handle.id, offset, READ_CHUNK),
            )
            .await?;

        let bytes = base64::engine::general_purpose::STANDARD.decode(&chunk.data)?;
        offset += bytes.len() as u64;
        collected.extend_from_slice(&bytes);

        if chunk.eof {
            break;
        }
        if bytes.is_empty() {
            // A chunk that is neither the end nor any progress would loop
            // forever. This cannot happen against this module, and a host
            // should still not take that on trust from a peer.
            return Err(io::Error::other("output stalled before its end").into());
        }
    }

    if collected.len() as u64 != handle.total_bytes {
        return Err(io::Error::other(format!(
            "collected {} bytes for an output declared as {}",
            collected.len(),
            handle.total_bytes
        ))
        .into());
    }

    Ok(collected)
}

/// Waits for the module to claim its well-known name.
///
/// Loading returns as soon as the library's entry point has run; the name is
/// claimed a moment later, on the module's own runtime. A host that calls
/// immediately gets `NameHasNoOwner` for a module that is perfectly healthy.
async fn await_name(client: &Connection) -> tinybus::Result<()> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if client
                .list_names()
                .await?
                .iter()
                .any(|name| name.as_str() == names::INTERFACE)
            {
                return tinybus::Result::Ok(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .map_err(|_| tinybus::Error::failed("module did not claim its name within five seconds"))?
}

/// The module path and the URL to visit.
fn arguments() -> Result<(PathBuf, String), io::Error> {
    let mut args = std::env::args_os().skip(1);
    let module = args.next().map(PathBuf::from).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: cargo run --example over_the_bus -- <module-path> [url]",
        )
    })?;

    let url = args.next().map_or_else(
        || "https://example.com".to_string(),
        |raw| raw.to_string_lossy().into_owned(),
    );

    Ok((module, url))
}
