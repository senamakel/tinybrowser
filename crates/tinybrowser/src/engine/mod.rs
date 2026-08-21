//! The engine: the one type everything else in this crate is reached through.
//!
//! # Why a facade
//!
//! A [`Browser`] holds the open sessions and the outputs waiting to be
//! collected, and every member of the bus interface is one method on it. That
//! makes the `TinyBus` adapter a translation layer with no decisions in it —
//! which is the point, because a decision made in the adapter is a decision that
//! cannot be tested without a bus, and one that a Rust caller using this crate
//! directly would not get.
//!
//! # Concurrency
//!
//! Sessions live behind an `RwLock` holding `Arc<Session>`s, and every operation
//! takes a read lock, clones the `Arc`, and drops the lock before doing anything
//! slow. Two agents driving two sessions never wait on each other; two agents
//! driving the *same* session serialise inside that session's own protocol
//! calls, which is what a single page requires anyway.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tinybrowser_bus::{
    ActionOutcome, EvaluateRequest, NavigateRequest, OutputChunk, OutputId, OutputRef, PageState,
    PageText, ReadRequest, ScreenshotRequest, SessionId, SessionInfo, SessionOptions, Snapshot,
    SnapshotRequest,
};
use tokio::sync::{Mutex, RwLock};

use crate::capture::store::OutputStore;
use crate::error::{Error, Result};
use crate::session::Session;
use crate::{capture, extract, interact, snapshot};

/// How many sessions one module may hold open.
///
/// Each is a browser process with its own renderer, so this is a bound on the
/// host's memory rather than on this module's bookkeeping. Eight is more
/// concurrent browsers than an agent host has any reason to want and few enough
/// that a runaway loop is stopped before it takes the machine with it.
const MAX_SESSIONS: usize = 8;

/// An engine holding browser sessions.
///
/// # Examples
///
/// The whole surface, without a browser in sight — every method below is what
/// one bus member calls:
///
/// ```
/// # use tinybrowser::Browser;
/// let browser = Browser::new();
/// assert_eq!(browser.session_limit(), 8);
/// ```
#[derive(Debug)]
pub struct Browser {
    sessions: RwLock<HashMap<SessionId, Arc<Session>>>,
    outputs: Mutex<OutputStore>,
    limit: usize,
}

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}

impl Browser {
    /// An engine holding no sessions.
    #[must_use]
    pub fn new() -> Self {
        Self::with_session_limit(MAX_SESSIONS)
    }

    /// An engine that will hold at most `limit` sessions.
    #[must_use]
    pub fn with_session_limit(limit: usize) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            outputs: Mutex::new(OutputStore::default()),
            limit: limit.max(1),
        }
    }

    /// How many sessions this engine will hold open.
    #[must_use]
    pub fn session_limit(&self) -> usize {
        self.limit
    }

    /// Opens a session, launching or attaching to a browser.
    ///
    /// # Errors
    ///
    /// [`Error::LimitExceeded`] when the engine is already at its session
    /// limit, and [`Error::BrowserUnavailable`] when no browser can be launched
    /// or reached.
    pub async fn open_session(&self, options: SessionOptions) -> Result<SessionInfo> {
        // Checked before the browser is launched, not after: the point of the
        // limit is to not start the ninth browser.
        if self.sessions.read().await.len() >= self.limit {
            return Err(Error::LimitExceeded {
                message: format!(
                    "{} sessions are already open; close one before opening another",
                    self.limit
                ),
            });
        }

        let id = SessionId::new(uuid::Uuid::new_v4().to_string());
        let session = Arc::new(Session::open(id.clone(), options).await?);
        let info = session.info().await?;

        self.sessions.write().await.insert(id, session);
        Ok(info)
    }

    /// Closes a session and everything it owns.
    ///
    /// Closing one that is already gone succeeds: a host retrying a close must
    /// not have to tell "never existed" apart from "already cleaned up".
    ///
    /// # Errors
    ///
    /// Never. The signature is fallible so the bus member's shape does not
    /// change if teardown ever acquires a failure mode.
    pub async fn close_session(&self, id: &SessionId) -> Result<()> {
        let session = self.sessions.write().await.remove(id);
        if let Some(session) = session {
            session.close().await;
        }
        Ok(())
    }

    /// Every session this engine is holding open.
    ///
    /// A session whose browser has died is reported with whatever state could
    /// still be read rather than omitted, because "it is gone" is the answer a
    /// caller listing sessions most needs.
    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        let sessions: Vec<Arc<Session>> =
            self.sessions.read().await.values().cloned().collect();

        let mut infos = Vec::with_capacity(sessions.len());
        for session in sessions {
            match session.info().await {
                Ok(info) => infos.push(info),
                Err(_) => infos.push(SessionInfo {
                    id: session.id().clone(),
                    endpoint: String::new(),
                    launched: false,
                    headless: session.options().headless,
                    viewport: session.options().viewport,
                    url: String::new(),
                    title: String::new(),
                }),
            }
        }

        infos.sort_by(|left, right| left.id.cmp(&right.id));
        infos
    }

    /// Navigates a session's active page.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchSession`], plus anything
    /// [`Session::navigate`](crate::session::Session::navigate) reports.
    pub async fn navigate(
        &self,
        id: &SessionId,
        request: &NavigateRequest,
    ) -> Result<PageState> {
        self.session(id).await?.navigate(request).await
    }

    /// Snapshots a session's active page.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchSession`], plus anything the capture reports.
    pub async fn snapshot(&self, id: &SessionId, request: &SnapshotRequest) -> Result<Snapshot> {
        let session = self.session(id).await?;
        snapshot::capture(&session, request).await
    }

    /// Performs one interaction.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchSession`], plus anything the interaction reports.
    pub async fn perform(
        &self,
        id: &SessionId,
        action: &tinybrowser_bus::Action,
    ) -> Result<ActionOutcome> {
        let session = self.session(id).await?;
        interact::perform(&session, action).await
    }

    /// Reads a session's active page as text.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchSession`], plus anything the extraction reports.
    pub async fn read_page(&self, id: &SessionId, request: &ReadRequest) -> Result<PageText> {
        let session = self.session(id).await?;
        extract::read(&session, request).await
    }

    /// Evaluates JavaScript in a session's active page.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInput`] for an empty expression, [`Error::NoSuchSession`],
    /// and [`Error::PageError`] when the expression throws.
    pub async fn evaluate(&self, id: &SessionId, request: &EvaluateRequest) -> Result<Value> {
        if request.expression.trim().is_empty() {
            return Err(Error::invalid_input("expression is empty"));
        }

        let session = self.session(id).await?;
        let deadline = session.deadline(request.timeout_ms);
        session
            .evaluate(&request.expression, request.await_promise, deadline)
            .await
    }

    /// Captures a screenshot and holds it for collection.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchSession`], plus anything the capture reports.
    pub async fn screenshot(
        &self,
        id: &SessionId,
        request: &ScreenshotRequest,
    ) -> Result<OutputRef> {
        let session = self.session(id).await?;
        capture::screenshot(&session, request, &self.outputs).await
    }

    /// Reads one chunk of a held output.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchOutput`] when it is unknown or expired, and
    /// [`Error::InvalidInput`] when `offset` is past its end.
    pub async fn read_output(
        &self,
        id: &OutputId,
        offset: u64,
        len: u64,
    ) -> Result<OutputChunk> {
        self.outputs.lock().await.read(id, offset, len)
    }

    /// Releases a held output.
    ///
    /// # Errors
    ///
    /// Never, for the same reason [`Browser::close_session`] does not.
    pub async fn release_output(&self, id: &OutputId) -> Result<()> {
        self.outputs.lock().await.release(id);
        Ok(())
    }

    /// Closes every session.
    ///
    /// A module is unloaded by the process ending, so this is what stops a
    /// launched browser outliving the host that asked for it.
    pub async fn shutdown(&self) {
        let sessions: Vec<Arc<Session>> =
            self.sessions.write().await.drain().map(|(_, s)| s).collect();

        for session in sessions {
            session.close().await;
        }
    }

    /// The session named by `id`.
    async fn session(&self, id: &SessionId) -> Result<Arc<Session>> {
        self.sessions
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| Error::NoSuchSession { id: id.to_string() })
    }
}

#[cfg(test)]
mod test;
