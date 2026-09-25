# Jev browser control

**Status:** Implemented
**Owner:** TinyBrowser maintainers

## Problem

TinyBrowser exposes fast, typed browser primitives, but a host still spends a
general-purpose model turn choosing every click. Jev can make those bounded
choices much faster when it receives a closed operation set and the current
accessibility snapshot. The decision layer must not weaken TinyBrowser's stale
reference, navigation, or input safeguards, and it must not add a model or HTTP
client to the wire-contract crate.

## Goals

- Vendor pinned revisions of `agent-browser` and `tinyjevclient` beside
  `tinybus`.
- Add a Rust library that turns a goal and TinyBrowser snapshot into a typed
  next action through one Jev request.
- Ask independently whether the goal is complete; a `DONE` choice alone is not
  proof of completion.
- Execute bounded multi-step tasks against `tinybrowser::Browser` while keeping
  actions typed.
- Stop before likely irreversible clicks unless the caller explicitly allows
  them.
- Return a complete, inspectable step trace, terminal decision, and final
  snapshot.

## Non-goals

- No model, task loop, or provider dependency enters `tinybrowser-bus` or the
  `tinybrowser` engine crate.
- The controller does not invent form values. Callers supply named values; Jev
  chooses which supplied value belongs in which field.
- This change does not expose every `agent-browser` command. Its pinned source
  is a compatibility and implementation reference. Its current Rust package is
  a binary and cannot be linked as a library without an upstream API change.
- Jev does not grant user authority. It may choose among allowed actions, but
  deterministic policy owns budgets and irreversible-action approval.

## Proposed behavior

The `tinybrowser-control` crate exports `JevController`, `TaskRequest`,
`ControlLimits`, `Decision`, `TaskResult`, and their supporting enums.

For every step, the controller requests a bounded compact snapshot and sends
one `EvaluationRequest` containing:

- a closed `operation` choice containing only operations possible in the
  current snapshot;
- independent target choices for click, fill, and check operations when those
  target classes exist;
- an independent choice among caller-supplied input names when filling is
  possible; and
- a `goal_done` Noul question.

The selected operation determines which speculative target answer is used.
Jev never supplies selectors, coordinates, JavaScript, or text that will be
typed. Element choices map back to refs minted by the current snapshot, and
input choices map back to values held by the caller.

`JevController::run` repeats snapshot, decision, policy, and typed action until
one of these statuses is reached:

- `Done`: Jev chose `DONE` and the independent completion probability met the
  configured threshold.
- `DoneUnconfirmed`: Jev chose `DONE`, but the completion check did not meet the
  threshold and the decision budget cannot support another choice, or a retry
  still selected `DONE`. With budget remaining, the controller first asks Jev
  to choose another operation from the current snapshot without offering
  `DONE`.
- `Blocked`: Jev found no supported action that can make progress.
- `NeedsConfirmation`: a likely irreversible click was selected and approval
  was not present. This includes consequential labels (for example submit,
  transfer, authorize, save, share, and delete) and all unnamed click targets.
  Search and filter submissions are routine; a bare `Submit` or one naming a
  consequential form still requires confirmation.
  The pending decision is returned and nothing is clicked.
- `Stuck`: the configured number of non-wait actions produced no visible
  snapshot change.
- `Budget`: the step limit was exhausted.

An empty goal, no input choices for a selected fill, a missing speculative
answer, or an answer of the wrong type is an error. Provider and browser errors
remain distinct and retain their sources.

## Invariants and constraints

- One Jev request per decision, including an unconfirmed-completion retry and
  excluding provider retries internal to `tinyjevclient`.
- For a task with one caller-supplied input, a successful fill establishes a
  post-fill observation, even when entering the value changes the tree. `FILL`
  stays unavailable while later observations match it and the filled ref still
  names that field. A subsequent change or replacement permits another fill.
- Only refs from the snapshot used for the decision may be acted on.
- User-supplied input values are never used as Jev criterion identifiers. The
  model sees input names; the controller retains the values locally.
- A completion probability cannot convert any non-`DONE` operation into
  success.
- A Jev decision cannot bypass origin policy, hit testing, stale-ref checking,
  session limits, or any other TinyBrowser engine rule.
- Defaults are finite: 30 actions, three unchanged actions, a 0.5 completion
  threshold, and a short bounded wait.
- API keys are owned and redacted by `tinyjevclient`; TinyBrowser does not read
  or log them.

## Acceptance criteria

- Both new submodules are initialized by `git submodule update --init
  --recursive` and pinned by gitlinks.
- Unit tests pin question construction, role-to-operation filtering, decision
  decoding, local input lookup, completion gating, irreversible labels, and
  unchanged-page limits.
- The public API is documented and an example shows the full task loop.
- The four repository contract commands pass.

## Open questions

None block this implementation. Additional `agent-browser` features such as
tabs, uploads, annotated screenshots, and WebMCP can be added as separately
specified capabilities.
