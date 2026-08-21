//! Turning what a host configured into a browser socket URL.
//!
//! A host may hand over either form of endpoint, and both are reasonable things
//! to have: an operator knows `http://127.0.0.1:9222`, because that is the flag
//! Chrome was started with, while an orchestrator that already asked the browser
//! about itself holds the `ws://…/devtools/browser/<id>` URL. This module
//! accepts both and returns the second.

use std::time::Duration;

use serde::Deserialize;

use crate::error::{Error, Result};

/// How long to wait for the browser's HTTP endpoint to answer.
///
/// This is a request to a socket on the same host — or to a container next
/// door — that answers from memory. Anything slower than this is not slow, it
/// is broken.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);

/// What `/json/version` reports.
#[derive(Debug, Deserialize)]
struct BrowserVersion {
    #[serde(rename = "webSocketDebuggerUrl")]
    websocket_url: Option<String>,
}

/// Resolves `endpoint` to a browser WebSocket URL.
///
/// A `ws://` or `wss://` endpoint is already one and is returned unchanged. An
/// `http://` or `https://` endpoint is asked for its `/json/version`.
///
/// # Errors
///
/// [`Error::InvalidInput`] when `endpoint` carries a scheme this cannot use,
/// and [`Error::BrowserUnavailable`] when the browser does not answer or
/// answers without a debugger URL.
pub(crate) async fn resolve(endpoint: &str) -> Result<String> {
    let endpoint = endpoint.trim().trim_end_matches('/');

    if endpoint.starts_with("ws://") || endpoint.starts_with("wss://") {
        return Ok(endpoint.to_string());
    }

    if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
        return Err(Error::invalid_input(format!(
            "endpoint {endpoint} must be an http(s) devtools address or a ws(s) browser socket"
        )));
    }

    let client = reqwest::Client::builder()
        .timeout(DISCOVERY_TIMEOUT)
        .build()
        .map_err(|error| Error::browser_unavailable(format!("http client: {error}")))?;

    let version: BrowserVersion = client
        .get(format!("{endpoint}/json/version"))
        .send()
        .await
        .map_err(|error| {
            Error::browser_unavailable(format!("{endpoint}/json/version: {error}"))
        })?
        .json()
        .await
        .map_err(|error| {
            Error::browser_unavailable(format!("{endpoint}/json/version returned: {error}"))
        })?;

    version.websocket_url.ok_or_else(|| {
        Error::browser_unavailable(format!(
            "{endpoint} answered without a webSocketDebuggerUrl; it is reachable but not a \
             devtools endpoint"
        ))
    })
}

#[cfg(test)]
mod test {
    //! Tests for endpoint resolution.
    //!
    //! Only the cases that need no browser: a `ws://` endpoint is returned
    //! unchanged and a bad scheme is refused before any request is made. The
    //! HTTP path is covered by the `live-chrome` suite, where there is something
    //! to ask.

    use super::resolve;
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
}
