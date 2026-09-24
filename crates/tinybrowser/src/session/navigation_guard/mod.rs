//! CDP interception of document requests for sessions with an origin allowlist.
//!
//! Checking only explicit `Navigate` calls misses redirects, clicks, and page
//! script. `Fetch.requestPaused` fires before Chrome sends each document request;
//! the guard continues admitted destinations and fails refused ones in Chrome.

use std::sync::{Arc, Weak};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::cdp::{CdpClient, CdpEvent};

use super::policy;

#[cfg(test)]
mod test;

const GUARD_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether a paused document request may reach the network.
fn admitted(params: &Value, allowed: &[String]) -> bool {
    // Fetch.enable requests Document only, but fail closed if Chrome delivers a
    // malformed event. A request with no parseable URL must never be continued.
    let Some(url) = params.pointer("/request/url").and_then(Value::as_str) else {
        return false;
    };
    let Ok(url) = url::Url::parse(url) else {
        return false;
    };
    policy::check_allowed(&url, allowed).is_ok()
}

/// Guard one attached page's document requests until its session closes.
pub(super) fn spawn(
    client: &Arc<CdpClient>,
    page_session: String,
    allowed: Vec<String>,
    mut events: broadcast::Receiver<CdpEvent>,
) -> tokio::task::JoinHandle<()> {
    let weak: Weak<CdpClient> = Arc::downgrade(client);
    tokio::spawn(async move {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // A missed paused request stays blocked. Continuing later
                    // requests does not turn event loss into policy bypass.
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            };
            if event.method != "Fetch.requestPaused"
                || event.session_id.as_deref() != Some(&page_session)
            {
                continue;
            }
            let Some(request_id) = event.params.get("requestId").and_then(Value::as_str) else {
                continue;
            };
            let Some(client) = weak.upgrade() else { break };
            let (method, params) = if admitted(&event.params, &allowed) {
                ("Fetch.continueRequest", json!({ "requestId": request_id }))
            } else {
                (
                    "Fetch.failRequest",
                    json!({
                        "requestId": request_id,
                        "errorReason": "BlockedByClient",
                    }),
                )
            };
            if client
                .send(method, params, Some(&page_session), GUARD_COMMAND_TIMEOUT)
                .await
                .is_err()
            {
                // A failed guard cannot safely continue later requests. The
                // browser's paused requests stay blocked until session close.
                break;
            }
        }
    })
}
