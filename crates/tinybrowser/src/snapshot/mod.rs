//! Capturing the accessibility tree of a page.
//!
//! # Why the accessibility tree and not the DOM
//!
//! An agent needs to know what is on a page and what it can do with it. The DOM
//! answers a different question — how the page is built — and answers it at ten
//! to a hundred times the size, most of it wrappers, styling hooks, and inline
//! scripts. The accessibility tree is the browser's own answer to "what is here
//! and what does it do", computed after layout, with hidden nodes already gone
//! and every control carrying its role, name, and state.
//!
//! It is also the same tree a screen reader uses, which means a page that
//! snapshots badly is usually a page that is genuinely inaccessible rather than
//! one this module reads wrong.
//!
//! # Layout
//!
//! - [`types`] — the nodes as Chrome reports them.
//! - [`render`] — the pure part: nodes in, indented text and refs out.

pub(crate) mod render;
pub(crate) mod types;

use serde_json::{Value, json};
use tinybrowser_bus::{Snapshot, SnapshotRequest};

use crate::error::{Error, Result};
use crate::session::Session;

use types::AxNode;

/// Snapshots the session's active page.
///
/// The refs this mints replace the session's previous ones, so anything held
/// from an earlier snapshot reports as stale from here on.
///
/// # Errors
///
/// [`Error::NoSuchElement`] when `request.selector` matches nothing,
/// [`Error::PageError`] when the browser will not produce a tree, and
/// [`Error::Timeout`] when it does not answer in time.
pub(crate) async fn capture(session: &Session, request: &SnapshotRequest) -> Result<Snapshot> {
    // Enabling per snapshot rather than at attach time: the accessibility tree
    // is computed lazily, and leaving the domain on makes every layout in the
    // session pay for a tree nobody asked for.
    session.send("Accessibility.enable", json!({})).await?;

    // Resolved before the tree is fetched so a selector that matches nothing
    // fails without paying for a document-sized response first.
    let root = match &request.selector {
        Some(selector) => Some(super::interact::resolve::selector_node(session, selector).await?),
        None => None,
    };

    let response = session
        .send("Accessibility.getFullAXTree", json!({}))
        .await?;

    let nodes: Vec<AxNode> = serde_json::from_value(
        response
            .get("nodes")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    )
    .map_err(|error| Error::page(format!("accessibility tree could not be read: {error}")))?;

    let rendered = render::render(&nodes, request, root).ok_or_else(|| Error::NoSuchElement {
        target: request
            .selector
            .clone()
            .unwrap_or_else(|| "document".to_string()),
    })?;
    let sequence = session.refs().lock().await.replace(rendered.nodes);
    let page = session.page_state().await?;

    Ok(Snapshot {
        url: page.url,
        title: page.title,
        sequence,
        tree: rendered.tree,
        refs: rendered.refs,
        truncated: rendered.truncated,
    })
}

#[cfg(test)]
mod test;
