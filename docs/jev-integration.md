# How Jev controls TinyBrowser

`tinybrowser-control` is a host-side library that uses Jev to choose a browser
action from a small, explicit set. The `tinybrowser` engine executes the action
and enforces its normal browser rules. A host can call `JevController` directly
today; the `tinybrowser-skills` crate supplies instructions and tool schemas
for a host that wants to expose this as an agent tool.

## One decision at a time

1. The host opens a `tinybrowser::Browser` session, navigates to a permitted
   page, and creates a `TaskRequest` with an observable goal. If a form needs
   values, the host supplies them under meaningful names in `inputs`.
2. The controller takes a compact accessibility snapshot. It sends the goal,
   current URL, title, rendered tree, truncation flag, and up to eight recent
   actions to Jev through `tinyjevclient`. The tree is capped at 50,000
   characters. Page text is untrusted task data, but it still reaches the
   configured provider.
3. One Jev `EvaluationRequest` asks several typed questions together: which
   operation to use, which current ref to target for each applicable action,
   which named input to use for a fill, and an independent `goal_done` Noul
   probability. Operations available for the current page are `CLICK`, `FILL`,
   `CHECK`, `SCROLL_DOWN`, `SCROLL_UP`, `BACK`, `WAIT`, `DONE`, and `BLOCKED`.
   `CLICK`, `FILL`, and `CHECK` are offered only when matching accessible refs
   exist; `FILL` also requires a caller input. A target or input with only one
   candidate is resolved locally.
4. Rust decodes the selected operation and its relevant target. It accepts only
   a ref minted by this snapshot and only a named input supplied by the caller.
   Jev cannot emit a selector, coordinate, script, or new text value. A `DONE`
   choice succeeds only if `goal_done` meets the completion threshold; an
   unrelated high completion score never ends the task.
5. Before a browser action, Rust checks the consequential-click gate. The
   engine then checks origin policy, ref freshness, hit testing, and its other
   session rules. The controller takes a fresh snapshot after each attempted
   action, including recoverable browser errors. It stops after a finite step
   budget or repeated non-wait actions with no visible URL, title, or tree
   change.

This is an observe/decide/act loop with one provider evaluation per step,
apart from retries performed inside `tinyjevclient`. The default limits are 30
steps, three consecutive unchanged non-wait actions, a `goal_done` threshold of
0.5, and a 250 ms `WAIT` action. `ControlLimits` can change these values.

## Example

```rust,no_run
use std::collections::BTreeMap;
use tinybrowser::{Browser, NavigateRequest, SessionOptions};
use tinybrowser_control::{JevController, TaskRequest, TaskStatus};
use tinyjevclient::Client;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let browser = Browser::new();
let session = browser.open_session(SessionOptions::default()).await?;
browser.navigate(&session.id, &NavigateRequest::new("https://example.com")).await?;

let task = TaskRequest::new("Find the page that visibly confirms the search")
    .with_inputs(BTreeMap::from([("search query".into(), "tinybrowser".into())]));
let result = JevController::new(Client::from_env()?)
    .run(&browser, &session.id, &task).await?;

match result.status {
    TaskStatus::Done => println!("Confirmed at {}", result.final_snapshot.url),
    other => println!("Stopped with {other:?} at {}", result.final_snapshot.url),
}
browser.close_session(&session.id).await?;
# Ok(())
# }
```

The caller creates the client from its own provider configuration. The
controller neither opens nor closes sessions and does not store the API key in
TinyBus. A future separately loaded control module can receive sensitive
configuration through TinyBus private initialization; that design is described
in [private Jev module initialization](specs/private-jev-module-initialization.md).

## Results and host responsibilities

`TaskResult` returns `status`, the final snapshot, action `steps`, an optional
unexecuted `pending` decision, and an optional terminal `DONE` or `BLOCKED`
decision. `DoneUnconfirmed` means Jev selected `DONE` without enough separate
completion evidence. `NeedsConfirmation` means a likely consequential click
was stopped before execution. `Blocked`, `Stuck`, and `Budget` identify other
finite stops. A host should inspect visible evidence before reporting success
and use its own confirmation mechanism for consequential actions. The controller
pauses on action labels such as submit, transfer, authorize, save, share, and
delete, and on unnamed targets, including icon links and buttons. It leaves
named ordinary navigation and search available. Label based click detection is a
guard, not a complete authorization system. Routine labels such as
`Submit search` and `Submit filters` remain available; a bare `Submit` or a
submit label naming a payment, order, transfer, or other consequential form
pauses for confirmation.

Every recorded decision includes provider model, latency, attempts, token
usage, and request ID when available. To account for a run, sum usage from
`steps[*].decision` and from `terminal` or `pending` when present. The latter
are provider calls even though they did not execute a browser action. Price
that usage with the rate applicable to the resolved model and provider at the
time of the run; the controller does not calculate dollars or enforce a cost
budget. The `browser_task` JSON schema advertises `max_seconds` and
`max_cost_usd`, but the host must implement those limits when it exposes the
tool. The Rust controller itself enforces its step and unchanged-page limits.

Downloads have their own completion signal. A click can start a file transfer
without changing the page, so a host should use `WaitDownload` and
`ListDownloads` to track the required file and verify the completed handle.
See [download handles](specs/download-handles.md) for the wire behavior. The
agent [skill](../crates/tinybrowser-skills/skills/tinybrowser/SKILL.md) explains
how a harness should combine `browser_task`, low-level recovery, and user
confirmation. The [control specification](specs/jev-browser-control.md) is the
authoritative list of controller invariants and terminal statuses.
