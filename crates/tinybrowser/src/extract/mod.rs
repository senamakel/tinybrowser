//! Reading a page as something a model can consume.
//!
//! # Layout
//!
//! - [`script`] — the traversal, which runs in the page.

pub(crate) mod script;

use serde_json::{Value, json};
use tinybrowser_bus::{PageText, ReadFormat, ReadRequest};

use crate::error::{Error, Result};
use crate::session::Session;

/// Reads the session's active page.
///
/// # Errors
///
/// [`Error::NoSuchElement`] when `request.selector` matches nothing,
/// [`Error::PageError`] when the page cannot be traversed, and
/// [`Error::Timeout`] when it does not answer in time.
pub(crate) async fn read(session: &Session, request: &ReadRequest) -> Result<PageText> {
    let page = session.page_state().await?;
    let deadline = session.deadline(None);

    let content = match request.format {
        ReadFormat::Html => html(session, request.selector.as_deref(), deadline).await?,
        ReadFormat::Text | ReadFormat::Markdown => {
            match extract(session, request, deadline).await? {
                Value::String(text) => text,
                Value::Null => {
                    return Err(Error::NoSuchElement {
                        target: request
                            .selector
                            .clone()
                            .unwrap_or_else(|| "document.body".to_string()),
                    });
                }
                other => other.to_string(),
            }
        }
    };

    let (content, truncated) = truncate(content, request.max_chars);

    Ok(PageText {
        url: page.url,
        title: page.title,
        format: request.format,
        content,
        truncated,
    })
}

/// Runs the extraction script against the page.
async fn extract(
    session: &Session,
    request: &ReadRequest,
    deadline: std::time::Duration,
) -> Result<Value> {
    let format = match request.format {
        ReadFormat::Markdown => "markdown",
        _ => "text",
    };

    let expression = format!(
        "({})({}, {})",
        script::EXTRACT,
        serde_json::to_string(format).unwrap_or_else(|_| "\"text\"".to_string()),
        serde_json::to_string(&request.selector).unwrap_or_else(|_| "null".to_string()),
    );

    session.evaluate(&expression, false, deadline).await
}

/// Serialises the live DOM.
async fn html(
    session: &Session,
    selector: Option<&str>,
    deadline: std::time::Duration,
) -> Result<String> {
    let node = match selector {
        Some(selector) => super::interact::resolve::selector_node(session, selector).await?,
        None => {
            let document = session.send("DOM.getDocument", json!({ "depth": 0 })).await?;
            document
                .get("root")
                .and_then(|root| root.get("backendNodeId"))
                .and_then(Value::as_i64)
                .ok_or_else(|| Error::page("page has no document".to_string()))?
        }
    };

    let outer = session
        .send_with_timeout(
            "DOM.getOuterHTML",
            json!({ "backendNodeId": node }),
            deadline,
        )
        .await?;

    Ok(outer
        .get("outerHTML")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

/// Cuts `content` to `max_chars`, reporting whether anything was lost.
///
/// Counted in characters rather than bytes: a byte-wise cut through a multi-byte
/// character produces invalid UTF-8, and the limit exists to bound what a model
/// reads, which is not measured in bytes either.
pub(crate) fn truncate(content: String, max_chars: usize) -> (String, bool) {
    if content.chars().count() <= max_chars {
        return (content, false);
    }

    (content.chars().take(max_chars).collect(), true)
}

#[cfg(test)]
mod test;
