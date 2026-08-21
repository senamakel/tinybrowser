# Wiring tinybrowser into OpenHuman

This is the host side: what to add to an OpenHuman build so an agent can use a
browser, and what the tool on top of it should look like. It assumes familiarity
with `src/openhuman/modules/` — the registry, the loader, and the two modules
already wired through it (`tinydocs` and `tinywallet`) are the pattern this
follows.

Nothing here has to be invented. The three pieces are a registry entry, a call
module, and a tool, and each has a close precedent in the tree already.

## 1. The registry entry

`src/openhuman/modules/registry.rs` holds the compiled-in table of loadable
modules. Add one entry, with the digests taken verbatim from the release's own
`checksum.toml` — not computed from a local build, which would agree with itself
no matter what the release served:

```rust
/// The `tinybrowser` module: a Chrome driven over CDP, behind the bus.
///
/// Lazy, and more so than the others: a session is a browser process. A user
/// who never asks for a web page should not pay a download, a `dlopen`, and a
/// library that is never unloaded, let alone a Chrome.
const TINYBROWSER: ModuleRecord = ModuleRecord {
    id: "tinybrowser",
    description: "Browser automation: navigate, snapshot, act, read, screenshot",
    bus_name: "ai.tinyhumans.tinybrowser.Browser",
    object_path: "/ai/tinyhumans/tinybrowser/Browser",
    version: "<release version>",
    release_url: "https://github.com/tinyhumansai/tinybrowser/releases/tag/v<version>",
    assets: &[/* one PlatformAsset per published archive */],
    load: LoadPolicy::Lazy,
};
```

Take the names from `tinybrowser_bus::names` rather than retyping them, if the
host links the contract crate — which it should, and which is the next point.

## 2. The dependency

```toml
# The wire contract for the tinybrowser module: member names and payload types.
# Two pure-Rust dependencies, no transport, no browser — the whole reason the
# module is loadable rather than linked.
#
# Pinned to the tag the registry entry above downloads its artifact from. An
# unpinned git dependency resolves to whatever the default branch holds at build
# time, so a host would eventually compile against payload types newer than the
# module it actually loads — and the mismatch surfaces as a decode error at
# runtime, in a call, rather than as a build failure.
tinybrowser-bus = { git = "https://github.com/tinyhumansai/tinybrowser", tag = "v<version>" }
```

Move the tag and the registry entry's `version` together, in one commit. They are
the same decision written twice, and `ContractVersion` is what catches it when
they drift anyway — but catching it in review is cheaper than catching it in a
session.

`tinybrowser-bus`, never `tinybrowser`. The second one is the engine, and
linking it would put a WebSocket client, a TLS stack and a CDP surface back into
the binary this arrangement exists to keep them out of.

## 3. The call module

`src/openhuman/modules/browser.rs`, alongside `documents.rs` and `wallet.rs`.
Its job is to hold the session, make the calls, and turn a wire error name into
something the tool can act on. Its shape:

```rust
/// Registry id of the module these calls go to.
const MODULE_ID: &str = "tinybrowser";

/// Why a browser call did not produce what was asked for.
///
/// The three variants are not a taxonomy of what went wrong — they are what the
/// tool does next. `Recoverable` goes back to the model as a tool result it can
/// act on; `Failed` is a tool error; `Unavailable` means the capability is not
/// present and the model should stop asking.
pub enum BrowserCallError {
    Unavailable(String),
    Recoverable(String),
    Failed(String),
}

fn classify(error: &tinybus::Error) -> BrowserCallError { /* see below */ }
```

`classify` is the one piece worth writing carefully, and the contract already
did most of it. Every failure arrives with a stable name, and
`tinybrowser_bus::errors::is_agent_recoverable` answers the only question the
tool has:

```rust
use tinybrowser_bus::errors;

fn classify(error: &tinybus::Error) -> BrowserCallError {
    let tinybus::Error::MethodFailed { name, message } = error else {
        return BrowserCallError::Failed(error.to_string());
    };

    match name.as_str() {
        errors::BROWSER_UNAVAILABLE => BrowserCallError::Unavailable(message.clone()),
        // Never retried and never offered back to the model: the answer will not
        // change, and a model that retries turns a refusal into a loop.
        errors::BLOCKED_BY_POLICY => BrowserCallError::Failed(message.clone()),
        other if errors::is_agent_recoverable(other) => {
            BrowserCallError::Recoverable(message.clone())
        }
        _ => BrowserCallError::Failed(message.clone()),
    }
}
```

### Session lifetime

The module holds sessions; the host has to decide who owns one. The arrangement
that fits OpenHuman is **one session per conversation**, opened lazily on the
first browser tool call and closed when the conversation ends — an agent that
opens a page and then reasons for three turns must find the same page there, and
one that finishes must not leave a Chrome running.

Two things follow. `ensure_ready` should run *outside* the tool's own deadline,
as `documents.rs` does and for the same reason: a first use may download and
verify an artifact and launch a browser, and charging that to a navigation
timeout means the first page a user ever asks for is the one that fails. And the
close belongs on the same path that tears down the conversation, not only on the
success path — a session leaked by an early return is a browser process nobody
will ever end.

### Collecting a screenshot

`Screenshot` returns a handle, not an image, for the same reason `tinydocs`
returns one: a full-page capture does not fit in a 16 MiB frame, and a served
object cannot open a stream back to its caller. Read it in chunks and release it
in a `defer`-shaped position — the module expires an abandoned output, but until
then the slot and the memory are spent.

`crates/tinybrowser/examples/over_the_bus.rs` in this repository is a complete,
working version of that loop, including the two checks worth copying: do not
size a buffer from a length a peer sent, and verify the reassembled length
against the handle.

## 4. The tool

OpenHuman already has `BrowserTool` in
`src/openhuman/tools/impl/browser/`, with a `BrowserAction` enum and several
backends behind it — `agent_browser` shelling out to a CLI, `playwright` driving
a Node sidecar, `rust_native` on fantoccini. This module is a fourth backend,
and the one that removes the most machinery: no subprocess, no sidecar, no
driver binary, no JSON parsed out of somebody's stdout.

The mapping is close to one-to-one, because `tinybrowser_bus::Action` was shaped
to match:

| `BrowserAction` | Bus call |
| --- | --- |
| `Open { url }` | `Navigate` with `NavigateRequest::new(url)` |
| `Snapshot { interactive_only, compact, depth }` | `Snapshot` |
| `Click`, `Fill`, `Type`, `Hover`, `Press`, `Scroll`, `IsVisible`, `GetText` | `Perform` with the matching `Action` |
| `Find { by, value, action, fill_value }` | `Perform` with `Target::locator(..)` |
| `Wait { selector, text, ms }` | `Perform` with `Action::WaitFor` |
| `GetTitle`, `GetUrl` | any outcome's `page` field — no call needed |
| `Close` | `CloseSession` |

Two of those are worth noticing. `GetTitle` and `GetUrl` stop being calls at
all: every `Perform` and `Navigate` returns the page state it left behind, so
the tool answers from what it already has. And the `selector` string a model
passes goes through `Target::parse`, which reads a leading `@` as a snapshot ref
and anything else as CSS — the same rule the existing tool surface already
implies, written once in the contract instead of at each call site.

### What to tell the model

The tool description should say three things, because each one prevents a
specific failure:

1. **Snapshot before acting, and act on refs.** A ref names an element the agent
   has actually seen. A CSS selector guessed from a page description names
   whatever happens to match.
2. **A stale ref means snapshot again.** The module distinguishes "no such
   element" from "that ref is from an older view of this page", and the second
   has exactly one remedy.
3. **A refused click names what covered it.** `not actionable: covered by
   div#consent "We use cookies"` is an instruction: deal with the banner, take a
   fresh snapshot, then retry. A model that is not told this will retry the same
   click.

## 5. Configuration

`src/openhuman/config/schema/modules.rs` already gates modules; the browser
needs two host-level facts beyond that, and both belong in the environment
rather than in per-call config, because both describe the machine:

- `TINYBROWSER_CHROME` — the browser to launch, when it is not in a conventional
  location.
- `TINYBROWSER_CHROME_ARGS` — extra launch flags. `--no-sandbox` belongs here in
  a container without the right capabilities, or on Ubuntu 23.10 and later where
  unprivileged user namespaces are restricted by AppArmor. The module reports
  that case by name rather than silently applying the flag, because removing the
  renderer's isolation from the pages it visits is not a default a module gets
  to choose on a host's behalf.

`SessionOptions::allowed_origins` is available per session and is worth setting
when a deployment knows where an agent should be going. Set it knowing what it
is: it refuses navigations the module is *asked* to make, and a page's own
JavaScript can navigate around it. A deployment that needs a boundary puts the
browser in a network namespace.

## Order of work

1. Add the dependency and the registry entry, and confirm `modules.list` reports
   the capability and `ensure_loaded` resolves it on the host's platform.
2. Add `browser.rs` with `ensure_ready`, `open_session`, `close_session`, and
   `navigate`, and a test that a call into a module that is not loaded reports
   `Unavailable` rather than hanging.
3. Add the session-per-conversation lifetime, with the close on the teardown
   path, before adding any further members. A leak here is a browser process,
   and it is much easier to not introduce than to find later.
4. Add `Snapshot` and `Perform`, and only then the rest.
