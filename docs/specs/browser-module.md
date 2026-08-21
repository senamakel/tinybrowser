# Browser Module

## Purpose

Give an agent host a browser without putting a browser stack in its build: a
loadable TinyBus module that drives a real Chrome and describes what it sees in
terms an agent can act on.

## What a host gets

One interface, `ai.tinyhumans.tinybrowser.Browser`, with twelve members listed
in `tinybrowser_bus::names::METHODS`. The vocabulary is published as
`tinybrowser-bus`, a two-dependency crate with no transport, so the cost to the
host of linking it is a `serde` derive.

The loop the surface is shaped around:

```text
OpenSession -> Navigate -> Snapshot -> Perform -> Snapshot -> ... -> CloseSession
```

## Behaviour

### Sessions

- A session is one browser and the page it is driving. Every page member takes a
  `SessionId`; there is no implicit current page, because a host running two
  tasks at once must not have them fight over one.
- A session either launches a browser or attaches to a `DevTools` endpoint. A
  launched browser is ended and its temporary profile removed when the session
  closes; an attached one is left running.
- At most eight sessions are held at once. The ninth is refused before a browser
  is started, not after.
- `CloseSession` on a session that does not exist succeeds. A host retrying a
  close after a timeout must not have to tell "never existed" from "already
  cleaned up".

### Navigation

- Only `http`, `https`, and `about:blank` are navigable. A bare host becomes
  `https://`. `file:` would make the module a filesystem reader for whoever can
  reach the bus; `javascript:` would make navigation an evaluation channel that
  skips `Evaluate` and its deadline.
- `wait_until` settles at `commit`, `DOMContentLoaded`, `load`, or `networkIdle`.
  `networkIdle` is best-effort: a page that polls never goes quiet, and reporting
  a timeout would be a claim about the page rather than a fact about the network.
- A navigation retires every outstanding ref, because the document they named is
  gone.
- `allowed_origins`, when non-empty, refuses a navigation before the browser is
  asked to make a request. It is a guard rail, not a sandbox — see
  *Non-goals*.

### Snapshots and refs

- A snapshot renders the accessibility tree as indented text, one line per node,
  with `@eN` on every node an agent could address.
- Refs belong to the generation that minted them. A ref from an earlier
  generation is refused as `StaleRef`, never resolved against whatever now
  occupies that position — a stale ref that silently resolves is how an agent
  clicks the wrong thing and reports success.
- The generation counter moves for every snapshot and every navigation, so the
  first snapshot after a navigation is not number one.
- `interactive_only` keeps controls and drops prose. `compact` drops the
  structural scaffolding even when named.

### Interactions

- Clicks and keystrokes go through `Input.dispatchMouseEvent` and
  `Input.dispatchKeyEvent`, the same path a physical device takes. Synthetic
  `element.click()` and direct `value` assignment skip hit testing and skip the
  events a framework listens for, which makes them useful for driving a page and
  useless for checking one.
- A click is refused when something else occupies its click point, and the error
  names the covering element. Delivering the click to a consent banner and
  reporting success is the most expensive failure an agent can be handed.
- A key press fills in `key`, `code`, `windowsVirtualKeyCode`, and `text`
  together, because pages read all four.
- An element is named by snapshot ref, CSS selector, or semantic locator. The
  string form a tool receives is read by `Target::parse`: a leading `@` is a
  ref, anything else is CSS.

### Outputs

- A screenshot is held and collected in chunks, because a full-page capture does
  not fit a 16 MiB frame and a served object cannot open a stream to its caller.
- A handle carries the length and a SHA-256, so a host can verify what it
  reassembled. The length is a number the module sent: a host must treat it as a
  bound to check rather than one to size a buffer from.
- At most sixteen outputs are held, each at most 64 MiB, expiring after five
  minutes. Eviction is oldest-first, because the newest is the one somebody is
  about to read.

### Errors

- Every failure carries a stable name from `tinybrowser_bus::errors`, with the
  prose in the message. A host decides what to show a model by matching on the
  name; matching on wording breaks the first time wording changes.
- `errors::is_agent_recoverable` answers the only question a host tool has: can
  the model fix this by choosing differently, or does it need an operator?
- A lost connection reports as `NoSuchSession`, because the remedy is to open a
  new session rather than retry into a socket that will never answer.

## Non-goals

- **A sandbox.** `allowed_origins` refuses navigations the module is asked to
  make. A page's own JavaScript can navigate around it. A deployment that needs
  a boundary puts the browser in a network namespace.
- **An agent.** No model, no prompt, no tool schemas. Deciding what to click is
  the host's job.
- **Persistence.** Sessions are memory and end with the process.
- **Downloading a browser.** Discovery is a fixed list plus an environment
  variable. A module that fetches and executes a binary is a different kind of
  thing.

## Invariants a change must not break

1. `tinybrowser-bus` links no transport, no runtime, no HTTP client, no native
   library. CI asserts it.
2. `names::METHODS`, the interface macro's dispatch table, and the module
   manifest agree. The unit tests assert both relationships.
3. Every `Error` variant maps to a published wire name. The unit tests assert it
   for every variant, so a new one cannot be added without deciding what a host
   sees.
4. The unit suites need no browser. Judgement that can only be exercised against
   a live Chrome is judgement in the wrong place.
5. The live suite fails rather than skips once opted in. A suite that quietly
   does nothing reports green for a build in which nothing was checked.

## Acceptance

- `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features --
  -D warnings`, `cargo build --all-targets --all-features`, and
  `cargo test --all-features`, all green with no browser present.
- `TINYBROWSER_LIVE_TESTS=1 cargo test -p tinybrowser --test live_chrome` green
  against a real Chrome, serving its fixtures on loopback.
- `cargo run -p tinybrowser --example verify_module -- <cdylib>` loads the built
  artifact through the real dynamic loader and completes a call.
