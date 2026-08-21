//! Performing one interaction against a page.
//!
//! # The shape every action shares
//!
//! Resolve the target, do the thing, report where the page ended up. That last
//! part is why every action returns a [`ActionOutcome`] carrying page state a
//! caller did not ask for: an agent that clicks a link and then has to make a
//! second call to discover it navigated will sometimes not make that call, and
//! will then reason about the previous page.
//!
//! # Real input events, not synthetic ones
//!
//! Clicks and keystrokes go through `Input.dispatchMouseEvent` and
//! `Input.dispatchKeyEvent` — the same path a physical device takes — rather
//! than through `element.click()` or an assignment to `value`. The synthetic
//! versions skip hit testing, skip the events a framework listens for, and work
//! on elements a person could not reach, which makes them useful for driving a
//! page and useless for checking one. Every place this crate departs from that —
//! [`crate::interact::script::SELECT_OPTIONS`] is the main one — is a place
//! where the protocol has no input path at all.
//!
//! # Layout
//!
//! - [`resolve`] — target to node.
//! - [`keys`] — chord to key events.
//! - [`script`] — the JavaScript that answers what only the page knows.

pub(crate) mod keys;
pub(crate) mod resolve;
pub(crate) mod script;

use std::time::Duration;

use serde_json::{Value, json};
use tinybrowser_bus::{Action, ActionOutcome, ScrollDirection, Target, WaitState};

use crate::error::{Error, Result};
use crate::session::Session;

/// How often to re-test a condition while waiting for it.
///
/// Fast enough that a wait does not add a visible pause to a page that is
/// already ready, slow enough that a long wait is not a hot loop of protocol
/// round trips.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Performs `action` and reports what it did.
///
/// # Errors
///
/// Any of [`Error::NoSuchElement`], [`Error::StaleRef`],
/// [`Error::NotActionable`], [`Error::PageError`], or [`Error::Timeout`],
/// depending on the action and where it failed.
pub(crate) async fn perform(session: &Session, action: &Action) -> Result<ActionOutcome> {
    match action {
        Action::Click { target, new_tab } => {
            click(session, target, *new_tab, 1).await?;
            acted(session, Some(target)).await
        }
        Action::DoubleClick { target } => {
            click(session, target, false, 2).await?;
            acted(session, Some(target)).await
        }
        Action::Hover { target } => {
            hover(session, target).await?;
            acted(session, Some(target)).await
        }
        Action::Focus { target } => {
            focus(session, target).await?;
            acted(session, Some(target)).await
        }
        Action::Fill { target, value } => {
            fill(session, target, value).await?;
            acted(session, Some(target)).await
        }
        Action::Type {
            target,
            text,
            delay_ms,
        } => {
            if let Some(target) = target {
                focus(session, target).await?;
            }
            type_text(session, text, *delay_ms).await?;
            acted(session, target.as_ref()).await
        }
        Action::Press { key } => {
            press(session, key).await?;
            acted(session, None).await
        }
        Action::Select { target, values } => {
            select(session, target, values).await?;
            acted(session, Some(target)).await
        }
        Action::Check { target, checked } => {
            check(session, target, *checked).await?;
            acted(session, Some(target)).await
        }
        Action::Scroll {
            direction,
            pixels,
            target,
        } => {
            scroll(session, *direction, *pixels, target.as_ref()).await?;
            acted(session, target.as_ref()).await
        }
        Action::GetText { target } => {
            let text = on_node(session, target, script::TEXT, Vec::new()).await?;
            read(session, target, text).await
        }
        Action::GetAttribute { target, attribute } => {
            let value = on_node(session, target, script::ATTRIBUTE, vec![json!(attribute)]).await?;
            read(session, target, value).await
        }
        Action::IsVisible { target } => {
            let visible = is_visible(session, target).await;
            read(session, target, Value::Bool(visible)).await
        }
        Action::WaitFor {
            target,
            text,
            state,
            ms,
            timeout_ms,
        } => {
            wait_for(
                session,
                target.as_ref(),
                text.as_deref(),
                *state,
                *ms,
                *timeout_ms,
            )
            .await?;
            acted(session, target.as_ref()).await
        }
        Action::Back => history(session, -1).await,
        Action::Forward => history(session, 1).await,
        Action::Reload => reload(session).await,
    }
}

/// Runs one of the [`script`] functions against a target.
async fn on_node(
    session: &Session,
    target: &Target,
    function: &str,
    args: Vec<Value>,
) -> Result<Value> {
    let node = resolve::resolve(session, target).await?;
    session
        .call_on_node(node, function, args, session.deadline(None))
        .await
}

/// Moves the pointer over an element, firing the handlers a menu needs.
async fn hover(session: &Session, target: &Target) -> Result<()> {
    let (x, y) = click_point(session, target).await?;
    session
        .send(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseMoved", "x": x, "y": y }),
        )
        .await?;
    Ok(())
}

/// Gives an element keyboard focus without clicking it.
async fn focus(session: &Session, target: &Target) -> Result<()> {
    let node = resolve::resolve(session, target).await?;
    session
        .send("DOM.focus", json!({ "backendNodeId": node }))
        .await?;
    Ok(())
}

/// Chooses options in a `<select>`.
async fn select(session: &Session, target: &Target, values: &[String]) -> Result<()> {
    on_node(session, target, script::SELECT_OPTIONS, vec![json!(values)]).await?;
    Ok(())
}

/// Leaves a checkbox or radio in the wanted state.
async fn check(session: &Session, target: &Target, checked: bool) -> Result<()> {
    let current = on_node(session, target, script::CHECKED, Vec::new()).await?;

    // Clicking unconditionally would toggle a box that is already in the wanted
    // state, which is the opposite of what was asked for.
    if current.as_bool().unwrap_or(false) != checked {
        click(session, target, false, 1).await?;
    }
    Ok(())
}

/// Whether a target is present and rendered.
///
/// A target that does not resolve is not visible; that is an answer, not a
/// failure. Returning an error here would make the one action whose job is to
/// test for absence unable to report it.
async fn is_visible(session: &Session, target: &Target) -> bool {
    on_node(session, target, script::IS_VISIBLE, Vec::new())
        .await
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// Reloads the page and retires the refs the old document minted.
async fn reload(session: &Session) -> Result<ActionOutcome> {
    session.send("Page.reload", json!({})).await?;
    retire_refs(session).await;
    acted(session, None).await
}

/// Retires every outstanding ref, because the document they named is gone.
async fn retire_refs(session: &Session) {
    session
        .refs()
        .lock()
        .await
        .replace(std::collections::HashMap::new());
}

/// Scrolls to the element, checks nothing covers it, and dispatches a click.
async fn click(session: &Session, target: &Target, new_tab: bool, count: u32) -> Result<()> {
    let (x, y) = click_point(session, target).await?;

    // Ctrl on Linux and Windows, Meta on macOS: this is the modifier that turns
    // a click into "open in a new tab" on each platform, and the browser is on
    // the same host as this module.
    let modifiers = if new_tab {
        if cfg!(target_os = "macos") { 4 } else { 2 }
    } else {
        0
    };

    // Sampled before the click, so a navigation it starts can be recognised by
    // the document changing out from under this value.
    let before = session.document_status().await;

    for kind in ["mousePressed", "mouseReleased"] {
        session
            .send(
                "Input.dispatchMouseEvent",
                json!({
                    "type": kind,
                    "x": x,
                    "y": y,
                    "button": "left",
                    "clickCount": count,
                    "modifiers": modifiers,
                }),
            )
            .await?;
    }

    if let Some((href, _)) = before {
        session.settle_after_input(&href).await;
    }
    Ok(())
}

/// Where a target can be clicked, or why it cannot be.
async fn click_point(session: &Session, target: &Target) -> Result<(f64, f64)> {
    let node = resolve::resolve(session, target).await?;
    let point = session
        .call_on_node(
            node,
            script::CLICK_POINT,
            Vec::new(),
            session.deadline(None),
        )
        .await?;

    if point.get("ok").and_then(Value::as_bool) != Some(true) {
        let reason = point
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("element cannot be clicked");
        return Err(Error::not_actionable(format!(
            "{}: {reason}",
            resolve::describe_target(target)
        )));
    }

    let (Some(x), Some(y)) = (
        point.get("x").and_then(Value::as_f64),
        point.get("y").and_then(Value::as_f64),
    ) else {
        return Err(Error::not_actionable(format!(
            "{}: browser reported no click point",
            resolve::describe_target(target)
        )));
    };

    Ok((x, y))
}

/// Clears a field and types a value into it.
async fn fill(session: &Session, target: &Target, value: &str) -> Result<()> {
    let node = resolve::resolve(session, target).await?;

    session
        .send("DOM.focus", json!({ "backendNodeId": node }))
        .await?;
    session
        .call_on_node(node, script::SELECT_ALL, Vec::new(), session.deadline(None))
        .await?;

    if value.is_empty() {
        // `insertText` with an empty string is a no-op, so the selection would
        // survive and the field would keep its old value.
        return press(session, "Delete").await;
    }

    session
        .send("Input.insertText", json!({ "text": value }))
        .await?;
    Ok(())
}

/// Types text, either in one insertion or key by key.
async fn type_text(session: &Session, text: &str, delay_ms: Option<u64>) -> Result<()> {
    let Some(delay) = delay_ms else {
        // One insertion is both faster and closer to a paste, which is what most
        // fields want. Per-key events are for the fields that react to each one.
        session
            .send("Input.insertText", json!({ "text": text }))
            .await?;
        return Ok(());
    };

    for character in text.chars() {
        press(session, &character.to_string()).await?;
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
    }
    Ok(())
}

/// Presses a key or chord.
async fn press(session: &Session, chord: &str) -> Result<()> {
    let stroke = keys::parse(chord)?;

    // Pressing Enter in a form submits it, which is a navigation as much as a
    // click on the submit button is.
    let before = session.document_status().await;

    for kind in ["keyDown", "keyUp"] {
        let mut params = json!({
            "type": kind,
            "modifiers": stroke.modifiers,
            "key": stroke.key,
            "code": stroke.code,
            "windowsVirtualKeyCode": stroke.key_code,
            "nativeVirtualKeyCode": stroke.key_code,
        });

        // `text` on a keyUp would insert the character a second time.
        if kind == "keyDown"
            && let Some(text) = &stroke.text
        {
            params["text"] = json!(text);
        }

        session.send("Input.dispatchKeyEvent", params).await?;
    }

    if let Some((href, _)) = before {
        session.settle_after_input(&href).await;
    }
    Ok(())
}

/// Scrolls the page, or an element within it.
async fn scroll(
    session: &Session,
    direction: ScrollDirection,
    pixels: Option<u32>,
    target: Option<&Target>,
) -> Result<()> {
    let distance = f64::from(pixels.unwrap_or(0));
    let viewport = f64::from(session.options().viewport.height);
    let step = if distance > 0.0 { distance } else { viewport };

    let (x, y) = match direction {
        ScrollDirection::Down => (0.0, step),
        ScrollDirection::Up => (0.0, -step),
        ScrollDirection::Right => (step, 0.0),
        ScrollDirection::Left => (-step, 0.0),
        // A very large number rather than `scrollTo`: the same JavaScript then
        // serves both the page and an element, and every scroll container
        // clamps to its own extent.
        ScrollDirection::Bottom => (0.0, f64::from(i32::MAX)),
        ScrollDirection::Top => (0.0, f64::from(i32::MIN)),
    };

    match target {
        Some(target) => {
            let node = resolve::resolve(session, target).await?;
            session
                .call_on_node(
                    node,
                    script::SCROLL_BY,
                    vec![json!(x), json!(y)],
                    session.deadline(None),
                )
                .await?;
        }
        None => {
            session
                .evaluate(
                    &format!("(window.scrollBy({x}, {y}), true)"),
                    false,
                    session.deadline(None),
                )
                .await?;
        }
    }

    Ok(())
}

/// Blocks until a condition holds, or the deadline expires.
async fn wait_for(
    session: &Session,
    target: Option<&Target>,
    text: Option<&str>,
    state: WaitState,
    ms: Option<u64>,
    timeout_ms: Option<u64>,
) -> Result<()> {
    // A flat delay is the only thing a caller with neither a target nor text can
    // have meant, and it is occasionally the honest answer for an animation
    // nothing else observes.
    if target.is_none() && text.is_none() {
        tokio::time::sleep(Duration::from_millis(ms.unwrap_or(0))).await;
        return Ok(());
    }

    let deadline = session.deadline(timeout_ms);
    let condition = async {
        loop {
            if let Some(target) = target {
                let holds = match resolve::resolve(session, target).await {
                    Ok(node) => match state {
                        WaitState::Attached => true,
                        WaitState::Detached => false,
                        WaitState::Visible | WaitState::Hidden => {
                            let visible = session
                                .call_on_node(
                                    node,
                                    script::IS_VISIBLE,
                                    Vec::new(),
                                    session.deadline(None),
                                )
                                .await
                                .ok()
                                .and_then(|value| value.as_bool())
                                .unwrap_or(false);
                            (state == WaitState::Visible) == visible
                        }
                    },
                    // Not resolving satisfies exactly the two absence states.
                    Err(_) => matches!(state, WaitState::Detached | WaitState::Hidden),
                };

                if holds {
                    return Ok(());
                }
            }

            if let Some(text) = text {
                let expression = format!(
                    "document.body ? document.body.innerText.includes({}) : false",
                    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string())
                );
                if session
                    .evaluate(&expression, false, session.deadline(None))
                    .await?
                    .as_bool()
                    == Some(true)
                {
                    return Ok(());
                }
            }

            tokio::time::sleep(POLL_INTERVAL).await;
        }
    };

    match tokio::time::timeout(deadline, condition).await {
        Ok(result) => result,
        Err(_) => Err(Error::timeout(
            "wait_for",
            u64::try_from(deadline.as_millis()).unwrap_or(u64::MAX),
        )),
    }
}

/// Moves `offset` entries through the session's history.
async fn history(session: &Session, offset: i64) -> Result<ActionOutcome> {
    let history = session.send("Page.getNavigationHistory", json!({})).await?;

    let current = history
        .get("currentIndex")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let entries = history
        .get("entries")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);

    let wanted = current + offset;
    if wanted < 0 || usize::try_from(wanted).unwrap_or(usize::MAX) >= entries {
        return Err(Error::invalid_input(if offset < 0 {
            "there is nothing to go back to"
        } else {
            "there is nothing to go forward to"
        }));
    }

    let entry = history
        .get("entries")
        .and_then(Value::as_array)
        .and_then(|entries| entries.get(usize::try_from(wanted).unwrap_or(0)))
        .and_then(|entry| entry.get("id"))
        .and_then(Value::as_i64)
        .ok_or_else(|| Error::page("history entry has no id".to_string()))?;

    session
        .send("Page.navigateToHistoryEntry", json!({ "entryId": entry }))
        .await?;
    retire_refs(session).await;

    acted(session, None).await
}

/// An outcome for an action that only acted.
async fn acted(session: &Session, target: Option<&Target>) -> Result<ActionOutcome> {
    let outcome = ActionOutcome::acted(session.page_state().await?);
    Ok(match target {
        Some(target) => outcome.matching(resolve::describe_target(target)),
        None => outcome,
    })
}

/// An outcome for an action that read a value.
async fn read(session: &Session, target: &Target, value: Value) -> Result<ActionOutcome> {
    Ok(ActionOutcome::read(session.page_state().await?, value)
        .matching(resolve::describe_target(target)))
}

#[cfg(test)]
mod test;
