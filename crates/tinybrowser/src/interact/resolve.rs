//! Turning a [`Target`] into a node the browser can act on.
//!
//! All three target forms end at the same place — a backend node id, which is
//! Chrome's stable handle for a DOM node — so every interaction resolves once
//! here and then works against a node rather than re-running a selector for each
//! step of what it is doing. That matters for more than tidiness: a click that
//! re-queries between measuring the element and dispatching at it can measure
//! one element and click another.

// Writing into a `String` cannot fail, so every `write!` below discards its
// result rather than propagating an error that does not exist.
use std::fmt::Write as _;

use serde_json::{Value, json};
use tinybrowser_bus::{LocateBy, Locator, Target};

use crate::error::{Error, Result};
use crate::session::Session;

use super::script;

/// Resolves `target` to a backend node id.
///
/// # Errors
///
/// [`Error::StaleRef`] for a ref from an earlier snapshot,
/// [`Error::NoSuchElement`] when nothing matches, and [`Error::PageError`] when
/// the page rejects the query — an invalid CSS selector, most often.
pub(crate) async fn resolve(session: &Session, target: &Target) -> Result<i64> {
    match target {
        Target::Ref { value } => {
            let map = session.refs().lock().await;
            map.resolve(value)
        }
        Target::Selector { value } => selector_node(session, value).await,
        Target::Locator { value } => locator_node(session, value).await,
    }
}

/// Resolves the first element matching `selector`.
///
/// # Errors
///
/// [`Error::NoSuchElement`] when nothing matches, and [`Error::PageError`] when
/// the selector is not valid CSS.
pub(crate) async fn selector_node(session: &Session, selector: &str) -> Result<i64> {
    let expression = format!(
        "document.querySelector({})",
        serde_json::to_string(selector).unwrap_or_else(|_| "null".to_string())
    );

    let object = evaluate_to_object(session, &expression)
        .await
        .map_err(|error| {
            // An invalid selector throws `SyntaxError` inside the page. Reported as
            // a page error it looks like the site's fault; reported as invalid input
            // it says what it is, which is that the caller wrote a bad selector.
            if error.to_string().contains("SyntaxError") {
                Error::invalid_input(format!("{selector} is not a valid css selector"))
            } else {
                error
            }
        })?;

    match object {
        Some(object_id) => describe(session, &object_id).await,
        None => Err(Error::NoSuchElement {
            target: selector.to_string(),
        }),
    }
}

/// Resolves an element named semantically.
///
/// # Errors
///
/// [`Error::NoSuchElement`] when nothing matches, and [`Error::InvalidInput`]
/// when the locator is empty.
async fn locator_node(session: &Session, locator: &Locator) -> Result<i64> {
    if locator.value.trim().is_empty() {
        return Err(Error::invalid_input("locator value is empty"));
    }

    let arguments = [
        json!(dimension(locator.by)),
        json!(locator.value),
        json!(locator.name),
        json!(locator.exact),
        json!(locator.index),
    ];

    // Called on `document` rather than on a node: there is no element yet, and
    // `callFunctionOn` needs something to be `this`.
    let document = evaluate_to_object(session, "document")
        .await?
        .ok_or_else(|| Error::page("page has no document".to_string()))?;

    let found = session
        .send_with_timeout(
            "Runtime.callFunctionOn",
            json!({
                "objectId": document,
                "functionDeclaration": script::LOCATE,
                "arguments": arguments.iter().map(|value| json!({ "value": value })).collect::<Vec<_>>(),
                "returnByValue": false,
            }),
            session.deadline(None),
        )
        .await?;

    let _ = session
        .send("Runtime.releaseObject", json!({ "objectId": document }))
        .await;

    let object_id = object_id_of(&found);
    match object_id {
        Some(object_id) => describe(session, &object_id).await,
        None => Err(Error::NoSuchElement {
            target: describe_locator(locator),
        }),
    }
}

/// Evaluates `expression` and returns the object id of its result, or `None`
/// when it evaluated to null or undefined.
async fn evaluate_to_object(session: &Session, expression: &str) -> Result<Option<String>> {
    let result = session
        .send_with_timeout(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": false }),
            session.deadline(None),
        )
        .await?;

    if let Some(details) = result.get("exceptionDetails") {
        let message = details
            .get("exception")
            .and_then(|exception| exception.get("description"))
            .and_then(Value::as_str)
            .unwrap_or("uncaught exception");
        return Err(Error::page(message.to_string()));
    }

    Ok(object_id_of(&result))
}

/// The object id in a `Runtime` result, when it names a live object.
///
/// A result of `null` still carries a `result` object — with `subtype: "null"`
/// and no `objectId` — so the presence of the field, not of the result, is what
/// distinguishes "found nothing" from "found something".
fn object_id_of(result: &Value) -> Option<String> {
    result
        .get("result")
        .and_then(|inner| inner.get("objectId"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Turns a `Runtime` object id into a backend node id, releasing the handle.
async fn describe(session: &Session, object_id: &str) -> Result<i64> {
    let described = session
        .send("DOM.describeNode", json!({ "objectId": object_id }))
        .await;

    let _ = session
        .send("Runtime.releaseObject", json!({ "objectId": object_id }))
        .await;

    described?
        .get("node")
        .and_then(|node| node.get("backendNodeId"))
        .and_then(Value::as_i64)
        .ok_or_else(|| Error::page("browser described a node without an identity".to_string()))
}

/// The wire spelling of a locator dimension, matching what [`script::LOCATE`]
/// switches on.
pub(crate) fn dimension(by: LocateBy) -> &'static str {
    match by {
        LocateBy::Role => "role",
        LocateBy::Text => "text",
        LocateBy::Label => "label",
        LocateBy::Placeholder => "placeholder",
        LocateBy::TestId => "test_id",
        LocateBy::AltText => "alt_text",
        LocateBy::Title => "title",
    }
}

/// How a locator reads in an error message.
pub(crate) fn describe_locator(locator: &Locator) -> String {
    let mut rendered = format!("{} {:?}", dimension(locator.by), locator.value);
    if let Some(name) = &locator.name {
        let _ = write!(rendered, " named {name:?}");
    }
    if locator.index > 0 {
        let _ = write!(rendered, " at index {}", locator.index);
    }
    rendered
}

/// How a target reads in an error message or an outcome.
pub(crate) fn describe_target(target: &Target) -> String {
    match target {
        Target::Ref { value } => format!("@{value}"),
        Target::Selector { value } => value.clone(),
        Target::Locator { value } => describe_locator(value),
    }
}
