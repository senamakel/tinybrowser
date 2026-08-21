//! `TinyBus` module entrypoint and bus-facing interface.
//!
//! # What this layer is, and what it deliberately is not
//!
//! It is a translation: each member deserializes its arguments, calls one method
//! on a [`Browser`], and turns an [`Error`] into a wire error name. There are no
//! decisions in it. That is the point — anything decided here could only be
//! tested through a bus, and a Rust caller using [`Browser`] directly would not
//! get it.
//!
//! The names and payload types come from [`tinybrowser_bus`], so a host spells
//! them from the contract crate instead of repeating string literals.
//!
//! # One engine for the life of the process
//!
//! The engine is a process-wide singleton, because the sessions it holds are
//! browser processes: a per-call engine would launch and discard a browser for
//! every navigation, and a per-connection one would strand sessions the moment a
//! host reconnected. `TinyBus` never unloads a module, so "the life of the
//! process" and "the life of the module" are the same thing.

use std::sync::{Arc, OnceLock};

use tinybrowser_bus::{
    Action, ActionOutcome, EvaluateRequest, NavigateRequest, OutputChunk, OutputId, OutputRef,
    PageState, PageText, ReadRequest, ScreenshotRequest, SessionId, SessionInfo, SessionOptions,
    Snapshot, SnapshotRequest, names,
};
use tinybus::{Connection, Result as BusResult};

use crate::{Browser, Error};

/// The engine every call goes to.
///
/// A `OnceLock` rather than a lazily-created-per-call value: two calls arriving
/// together must reach the same session table, or the second would not find the
/// session the first opened.
static ENGINE: OnceLock<Arc<Browser>> = OnceLock::new();

/// The process-wide engine, created on first use.
fn engine() -> Arc<Browser> {
    Arc::clone(ENGINE.get_or_init(|| Arc::new(Browser::new())))
}

/// The bus-facing object.
struct BrowserService;

#[tinybus::interface(name = "ai.tinyhumans.tinybrowser.Browser")]
impl BrowserService {
    /// Launches or attaches a browser and returns the session that owns it.
    async fn open_session(&self, options: SessionOptions) -> BusResult<SessionInfo> {
        engine()
            .open_session(options)
            .await
            .map_err(|error| to_bus(&error))
    }

    /// Closes a session and everything it owns.
    async fn close_session(&self, id: SessionId) -> BusResult<()> {
        engine()
            .close_session(&id)
            .await
            .map_err(|error| to_bus(&error))
    }

    /// Lists the sessions this module is holding open.
    async fn list_sessions(&self) -> BusResult<Vec<SessionInfo>> {
        Ok(engine().list_sessions().await)
    }

    /// Navigates a session's active page.
    async fn navigate(&self, id: SessionId, request: NavigateRequest) -> BusResult<PageState> {
        engine()
            .navigate(&id, &request)
            .await
            .map_err(|error| to_bus(&error))
    }

    /// Captures the accessibility tree of a session's active page.
    async fn snapshot(&self, id: SessionId, request: SnapshotRequest) -> BusResult<Snapshot> {
        engine()
            .snapshot(&id, &request)
            .await
            .map_err(|error| to_bus(&error))
    }

    /// Performs one interaction against a session's active page.
    async fn perform(&self, id: SessionId, action: Action) -> BusResult<ActionOutcome> {
        engine()
            .perform(&id, &action)
            .await
            .map_err(|error| to_bus(&error))
    }

    /// Extracts a session's active page as agent-readable text.
    async fn read_page(&self, id: SessionId, request: ReadRequest) -> BusResult<PageText> {
        engine()
            .read_page(&id, &request)
            .await
            .map_err(|error| to_bus(&error))
    }

    /// Evaluates JavaScript in a session's active page.
    async fn evaluate(
        &self,
        id: SessionId,
        request: EvaluateRequest,
    ) -> BusResult<serde_json::Value> {
        engine()
            .evaluate(&id, &request)
            .await
            .map_err(|error| to_bus(&error))
    }

    /// Captures a screenshot and holds it for collection.
    async fn screenshot(&self, id: SessionId, request: ScreenshotRequest) -> BusResult<OutputRef> {
        engine()
            .screenshot(&id, &request)
            .await
            .map_err(|error| to_bus(&error))
    }

    /// Reads one chunk of a held output.
    async fn read_output(&self, id: OutputId, offset: u64, len: u64) -> BusResult<OutputChunk> {
        engine()
            .read_output(&id, offset, len)
            .await
            .map_err(|error| to_bus(&error))
    }

    /// Releases a held output before it expires.
    async fn release_output(&self, id: OutputId) -> BusResult<()> {
        engine()
            .release_output(&id)
            .await
            .map_err(|error| to_bus(&error))
    }

    /// Reports the contract version this module serves.
    ///
    /// A host calls this once, before its first real call, and compares it with
    /// [`tinybrowser_bus::is_compatible`]. Doing it the other way round — making
    /// a real call and reading the failure — cannot distinguish "this member
    /// does not exist" from "this member failed".
    #[allow(
        clippy::unused_async,
        reason = "the interface macro requires every member to be an async fn"
    )]
    async fn contract_version(&self) -> BusResult<(u32, u32)> {
        Ok(tinybrowser_bus::CONTRACT_VERSION)
    }
}

/// Turns an engine error into the wire error a host matches on.
///
/// The name is what carries the meaning; the message is for a person reading a
/// log. See [`tinybrowser_bus::errors`] for what a host does with each name.
fn to_bus(error: &Error) -> tinybus::Error {
    tinybus::Error::MethodFailed {
        name: error.wire_name().to_string(),
        message: error.to_string(),
    }
}

/// Serves the interface and claims the well-known name.
async fn setup(connection: Connection) -> BusResult<()> {
    connection
        .serve_at(names::OBJECT_PATH.try_into()?, BrowserService)
        .await?;
    connection.request_name(names::INTERFACE).await?;
    Ok(())
}

// Isolate the generated public C symbols so the lint exception cannot hide
// undocumented Rust API. Their contract is TinyBus ABI v1, and none is a
// Rust-callable export from this module.
#[allow(
    missing_docs,
    unreachable_pub,
    reason = "generated C ABI symbols are documented by the TinyBus module SDK"
)]
mod exports {
    tinybus_module::module_export! {
        setup = super::setup,
        // Two threads: one to serve calls, one so a long navigation does not
        // block the call that would close the session it is stuck in.
        worker_threads = 2,
        provides = ["ai.tinyhumans.tinybrowser.Browser"],
        methods = [
            "OpenSession",
            "CloseSession",
            "ListSessions",
            "Navigate",
            "Snapshot",
            "Perform",
            "ReadPage",
            "Evaluate",
            "Screenshot",
            "ReadOutput",
            "ReleaseOutput",
            "ContractVersion",
        ],
        signals = [],
        requires = [],
        optional = [],
        // Eager rather than lazy: the module is cheap until a session is opened,
        // and a host that discovers the interface at startup can report the
        // capability without paying a load to find out it exists.
        lazy = false,
    }
}

#[cfg(test)]
mod test;
