//! One browser, one page, and everything driving them shares this.
//!
//! # What a session is
//!
//! A [`Session`] owns a CDP socket, a page target attached to it, and the refs
//! the last snapshot of that page minted. Every other part of the engine —
//! snapshots, interactions, extraction, screenshots — is written against this
//! type rather than against the protocol, so the awkward parts of CDP are
//! written once: attaching flatly, keeping a deadline on every command,
//! resolving a backend node into something a function can be called on, and
//! knowing what the page's URL and title are without a round trip per caller.
//!
//! # Layout
//!
//! - [`policy`] — which destinations this session admits.
//! - [`refs`] — the refs a snapshot minted, and when they go stale.

pub(crate) mod policy;
pub(crate) mod refs;

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tinybrowser_bus::{
    NavigateRequest, PageState, SessionId, SessionInfo, SessionOptions, WaitUntil,
};
use tokio::sync::Mutex;

use crate::cdp::launch::LaunchedBrowser;
use crate::cdp::{CdpClient, endpoint, launch};
use crate::error::{Error, Result};

use refs::RefMap;

/// How long a command that is not a navigation gets.
///
/// Protocol commands answer in microseconds when the renderer is responsive and
/// never when it is wedged, so this is a liveness bound rather than a budget:
/// long enough that a busy page is not cut off, short enough that a stuck one
/// does not hold a bus call open until the host's own deadline fires.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// How long to wait for the network to fall quiet under
/// [`WaitUntil::NetworkIdle`] before settling anyway.
///
/// A page that polls, holds a websocket open, or streams telemetry never goes
/// idle. Treating that as a navigation failure would fail on exactly the modern
/// applications the mode exists for, so the quiet period is best-effort: the
/// `load` event underneath it is what actually settles the navigation.
const NETWORK_IDLE_GRACE: Duration = Duration::from_secs(2);

/// One open browser session.
#[derive(Debug)]
pub(crate) struct Session {
    id: SessionId,
    options: SessionOptions,
    client: Arc<CdpClient>,
    /// The flat CDP session id addressing this session's page.
    page: String,
    /// The page target, kept so it can be closed without closing the browser a
    /// host lent us.
    target: String,
    endpoint: String,
    /// The browser this module started, if it started one. A session that
    /// attached to somebody else's browser leaves it running.
    launched: Mutex<Option<LaunchedBrowser>>,
    refs: Mutex<RefMap>,
}

impl Session {
    /// Opens a session: obtain a browser, attach to a page, and configure it.
    ///
    /// # Errors
    ///
    /// [`Error::BrowserUnavailable`] when no browser can be launched or
    /// reached, and [`Error::PageError`] when the browser rejects the setup
    /// commands.
    pub(crate) async fn open(id: SessionId, options: SessionOptions) -> Result<Self> {
        let (endpoint, launched) = if let Some(configured) = options.endpoint.as_deref() {
            (endpoint::resolve(configured).await?, None)
        } else {
            let executable = launch::find_executable(options.executable.as_deref())?;
            let browser = launch::launch(
                &executable,
                options.headless,
                options.user_data_dir.as_deref(),
                &options.args,
            )
            .await?;
            (browser.websocket_url.clone(), Some(browser))
        };

        let client = CdpClient::connect(&endpoint).await?;

        // A launched browser opens with a tab already. Creating our own anyway
        // means the session owns exactly one page and knows which it is —
        // adopting whatever tab happened to exist would make an attached
        // session start driving a page somebody else is using.
        let created = client
            .send(
                "Target.createTarget",
                json!({ "url": "about:blank" }),
                None,
                COMMAND_TIMEOUT,
            )
            .await?;
        let target = string_field(&created, "targetId")?;

        let attached = client
            .send(
                "Target.attachToTarget",
                json!({ "targetId": target, "flatten": true }),
                None,
                COMMAND_TIMEOUT,
            )
            .await?;
        let page = string_field(&attached, "sessionId")?;

        let session = Self {
            id,
            options,
            client,
            page,
            target,
            endpoint,
            launched: Mutex::new(launched),
            refs: Mutex::new(RefMap::default()),
        };

        session.configure().await?;
        Ok(session)
    }

    /// Enables the domains this engine needs and applies the session's
    /// emulation settings.
    async fn configure(&self) -> Result<()> {
        for domain in ["Page", "Runtime", "DOM", "Network"] {
            self.send(&format!("{domain}.enable"), json!({})).await?;
        }

        // Lifecycle events are what a navigation waits on. Without this, `load`
        // and `networkIdle` never arrive and every navigation settles at its
        // deadline instead.
        self.send("Page.setLifecycleEventsEnabled", json!({ "enabled": true }))
            .await?;

        let viewport = self.options.viewport;
        self.send(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": viewport.width,
                "height": viewport.height,
                "deviceScaleFactor": viewport.device_scale_factor,
                "mobile": viewport.mobile,
            }),
        )
        .await?;

        if let Some(user_agent) = &self.options.user_agent {
            self.send(
                "Network.setUserAgentOverride",
                json!({ "userAgent": user_agent }),
            )
            .await?;
        }

        Ok(())
    }

    /// This session's identity.
    pub(crate) fn id(&self) -> &SessionId {
        &self.id
    }

    /// The options it was opened with.
    pub(crate) fn options(&self) -> &SessionOptions {
        &self.options
    }

    /// Sends a CDP command addressed to this session's page.
    ///
    /// # Errors
    ///
    /// Whatever [`CdpClient::send`] reports.
    pub(crate) async fn send(&self, method: &str, params: Value) -> Result<Value> {
        self.client
            .send(method, params, Some(&self.page), COMMAND_TIMEOUT)
            .await
    }

    /// Sends a CDP command with an explicit deadline.
    ///
    /// # Errors
    ///
    /// Whatever [`CdpClient::send`] reports.
    pub(crate) async fn send_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        self.client
            .send(method, params, Some(&self.page), timeout)
            .await
    }

    /// The deadline for an operation that asked for `timeout_ms`, falling back
    /// to the session's default.
    pub(crate) fn deadline(&self, timeout_ms: Option<u64>) -> Duration {
        Duration::from_millis(timeout_ms.unwrap_or(self.options.default_timeout_ms))
    }

    /// The refs the last snapshot minted.
    pub(crate) fn refs(&self) -> &Mutex<RefMap> {
        &self.refs
    }

    /// Evaluates `expression` in the page and returns its value.
    ///
    /// # Errors
    ///
    /// [`Error::PageError`] when the expression throws, and [`Error::Timeout`]
    /// when it does not finish in time.
    pub(crate) async fn evaluate(
        &self,
        expression: &str,
        await_promise: bool,
        timeout: Duration,
    ) -> Result<Value> {
        let result = self
            .send_with_timeout(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": await_promise,
                    // Without this, an expression that reads a value from a
                    // click handler's closure — or any page that has been
                    // interacted with — can be refused as a user-gesture
                    // violation for no reason the caller can see.
                    "userGesture": true,
                }),
                timeout,
            )
            .await?;

        unwrap_evaluation(&result)
    }

    /// Calls `function` — a JavaScript function expression — on the node behind
    /// `backend_node_id`, with `args` as its remaining parameters.
    ///
    /// This is the shape almost every interaction takes: resolve the node once,
    /// then run a small function with the element as `this` rather than building
    /// a selector string and hoping it still matches.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchElement`] when the node is gone from the DOM, and
    /// [`Error::PageError`] when the function throws.
    pub(crate) async fn call_on_node(
        &self,
        backend_node_id: i64,
        function: &str,
        args: Vec<Value>,
        timeout: Duration,
    ) -> Result<Value> {
        let object_id = self.resolve_node(backend_node_id).await?;

        let result = self
            .send_with_timeout(
                "Runtime.callFunctionOn",
                json!({
                    "objectId": object_id,
                    "functionDeclaration": function,
                    "arguments": args.into_iter().map(|value| json!({ "value": value })).collect::<Vec<_>>(),
                    "returnByValue": true,
                    "awaitPromise": true,
                    "userGesture": true,
                }),
                timeout,
            )
            .await?;

        // The handle is released whether or not the call succeeded above: a
        // retained object keeps its whole DOM subtree alive in the renderer, and
        // a session that snapshots repeatedly would otherwise grow without
        // bound. Failing to release is not worth failing the call over.
        let _ = self
            .send("Runtime.releaseObject", json!({ "objectId": object_id }))
            .await;

        unwrap_evaluation(&result)
    }

    /// Resolves a backend node id into a `Runtime` object id.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchElement`] when the node is no longer in the document.
    pub(crate) async fn resolve_node(&self, backend_node_id: i64) -> Result<String> {
        let resolved = self
            .send(
                "DOM.resolveNode",
                json!({ "backendNodeId": backend_node_id }),
            )
            .await
            .map_err(|_| Error::NoSuchElement {
                target: format!("node {backend_node_id}"),
            })?;

        resolved
            .get("object")
            .and_then(|object| object.get("objectId"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or(Error::NoSuchElement {
                target: format!("node {backend_node_id}"),
            })
    }

    /// Where the page is now.
    ///
    /// # Errors
    ///
    /// [`Error::PageError`] when the page cannot be evaluated in.
    pub(crate) async fn page_state(&self) -> Result<PageState> {
        let state = self
            .evaluate(
                "({ url: location.href, title: document.title })",
                false,
                COMMAND_TIMEOUT,
            )
            .await?;

        Ok(PageState {
            url: state
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            title: state
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            status: None,
        })
    }

    /// What this session is, for [`crate::Browser::list_sessions`].
    ///
    /// # Errors
    ///
    /// [`Error::PageError`] when the page cannot be read.
    pub(crate) async fn info(&self) -> Result<SessionInfo> {
        let page = self.page_state().await?;

        Ok(SessionInfo {
            id: self.id.clone(),
            endpoint: self.endpoint.clone(),
            launched: self.launched.lock().await.is_some(),
            headless: self.options.headless,
            viewport: self.options.viewport,
            url: page.url,
            title: page.title,
        })
    }

    /// Navigates the page and waits for it to settle.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidInput`] for an unusable URL, [`Error::BlockedByPolicy`]
    /// when the session's allowlist refuses it, [`Error::PageError`] when the
    /// browser cannot reach it, and [`Error::Timeout`] when it does not settle
    /// in time.
    pub(crate) async fn navigate(&self, request: &NavigateRequest) -> Result<PageState> {
        let url = policy::normalize_url(&request.url)?;
        policy::check_allowed(&url, &self.options.allowed_origins)?;

        let deadline = self.deadline(request.timeout_ms);

        // Subscribed *before* the navigation is issued. A fast page can fire its
        // load event before a subscription taken afterwards exists, and the wait
        // would then sit out the whole deadline for an event that already
        // happened.
        let events = self.client.events();

        let started = self
            .send_with_timeout("Page.navigate", json!({ "url": url.as_str() }), deadline)
            .await?;

        if let Some(error) = started.get("errorText").and_then(Value::as_str) {
            return Err(Error::page(format!("navigating to {url}: {error}")));
        }
        let frame = string_field(&started, "frameId")?;

        // A ref names a node in the document that has just been replaced. The
        // snapshot counter moves on so every outstanding ref reports as stale
        // rather than resolving against the new page.
        self.refs
            .lock()
            .await
            .replace(std::collections::HashMap::new());

        let status = self
            .settle(events, &frame, request.wait_until, deadline)
            .await?;

        let mut page = self.page_state().await?;
        page.status = status;
        Ok(page)
    }

    /// Waits for the navigation of `frame` to reach `wait_until`, collecting the
    /// main document's HTTP status on the way past.
    async fn settle(
        &self,
        mut events: tokio::sync::broadcast::Receiver<crate::cdp::CdpEvent>,
        frame: &str,
        wait_until: WaitUntil,
        deadline: Duration,
    ) -> Result<Option<u16>> {
        let Some(target) = lifecycle_name(wait_until) else {
            return Ok(None);
        };

        let mut status = None;
        let wait = async {
            loop {
                let event = match events.recv().await {
                    Ok(event) => event,
                    // Lagged: the page emitted more events than the buffer
                    // holds. The one being waited for may have been among them,
                    // so the deadline below decides rather than this loop
                    // blocking forever on a subscription that has already
                    // missed its answer.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return Err(Error::connection_lost(
                            "browser closed during navigation".to_string(),
                        ));
                    }
                };

                if event.session_id.as_deref() != Some(self.page.as_str()) {
                    continue;
                }

                if event.method == "Network.responseReceived"
                    && event.params.get("type").and_then(Value::as_str) == Some("Document")
                    && event.params.get("frameId").and_then(Value::as_str) == Some(frame)
                {
                    status = event
                        .params
                        .get("response")
                        .and_then(|response| response.get("status"))
                        .and_then(Value::as_u64)
                        .and_then(|code| u16::try_from(code).ok());
                }

                if event.method == "Page.lifecycleEvent"
                    && event.params.get("frameId").and_then(Value::as_str) == Some(frame)
                    && event.params.get("name").and_then(Value::as_str) == Some(target)
                {
                    return Ok(());
                }
            }
        };

        match tokio::time::timeout(deadline, wait).await {
            Ok(result) => result?,
            Err(_) if wait_until == WaitUntil::NetworkIdle => {
                // Documented best-effort: a page that never goes quiet still
                // loaded, and reporting a timeout would be a lie about the page
                // rather than a fact about the network.
            }
            Err(_) => {
                return Err(Error::timeout(
                    "navigation",
                    u64::try_from(deadline.as_millis()).unwrap_or(u64::MAX),
                ));
            }
        }

        if wait_until == WaitUntil::NetworkIdle {
            // `networkIdle` fires the moment the counter reaches zero, which a
            // page that is about to issue its next request reaches too. The
            // grace period is what makes the mode mean "quiet" rather than
            // "momentarily between requests".
            tokio::time::sleep(NETWORK_IDLE_GRACE.min(deadline)).await;
        }

        Ok(status)
    }

    /// Closes the page, and the browser if this module launched it.
    ///
    /// Errors are swallowed: this is the teardown path, and a browser that has
    /// already gone is the outcome being asked for.
    pub(crate) async fn close(&self) {
        let _ = self
            .client
            .send(
                "Target.closeTarget",
                json!({ "targetId": self.target }),
                None,
                COMMAND_TIMEOUT,
            )
            .await;

        if let Some(launched) = self.launched.lock().await.take() {
            launched.shutdown().await;
        }
    }
}

/// The lifecycle event name a wait mode settles on, or `None` for a mode that
/// does not wait.
fn lifecycle_name(wait_until: WaitUntil) -> Option<&'static str> {
    match wait_until {
        WaitUntil::Commit => None,
        WaitUntil::DomContentLoaded => Some("DOMContentLoaded"),
        WaitUntil::Load => Some("load"),
        WaitUntil::NetworkIdle => Some("networkIdle"),
    }
}

/// Reads a required string out of a CDP result.
fn string_field(value: &Value, field: &str) -> Result<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::page(format!("browser replied without a {field}")))
}

/// Turns a `Runtime.evaluate` or `Runtime.callFunctionOn` result into a value or
/// an error.
///
/// CDP reports a thrown exception in the *result* rather than as a protocol
/// error, so a caller that only checks for a transport failure treats a
/// `ReferenceError` as a successful call returning `undefined`.
fn unwrap_evaluation(result: &Value) -> Result<Value> {
    if let Some(details) = result.get("exceptionDetails") {
        let message = details
            .get("exception")
            .and_then(|exception| exception.get("description"))
            .and_then(Value::as_str)
            .or_else(|| details.get("text").and_then(Value::as_str))
            .unwrap_or("uncaught exception");
        return Err(Error::page(message.to_string()));
    }

    Ok(result
        .get("result")
        .and_then(|inner| inner.get("value"))
        .cloned()
        .unwrap_or(Value::Null))
}

#[cfg(test)]
mod test;
