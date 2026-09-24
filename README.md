# tinybrowser

A browser for agents, shipped as a loadable [TinyBus](https://github.com/tinyhumansai/tinybus)
module.

`tinybrowser` drives a real Chrome over the Chrome DevTools Protocol — launching
it, navigating, snapshotting the accessibility tree, dispatching real input
events, extracting text, taking screenshots — and publishes all of it as a
handful of bus members. A host loads one `cdylib` and gets a browser without a
browser stack in its build.

```text
OpenSession  ->  Navigate  ->  Snapshot  ->  Perform  ->  Snapshot  ->  ...
                                   |             |
                               ReadPage      Screenshot
```

## Why it exists

An agent host that wants to look at a web page has bad options. Shelling out to
a browser CLI means a subprocess, a JSON parser around its output, and a binary
to install and version-match. Linking a browser stack in means dragging a
WebSocket client, a TLS stack, an image codec and a protocol surface into a
binary that mostly does something else — and a crash anywhere in it is a crash
in the host.

This is the third option: the browser lives behind a wire. The host keeps a
proxy and a `serde` derive.

## What an agent sees

A snapshot, not HTML:

```text
- RootWebArea "Example Domain"
  - heading "Example Domain" @e1
  - paragraph "This domain is for use in illustrative examples." @e2
  - link "More information..." @e3
```

That is the browser's own accessibility tree — an order of magnitude smaller
than the DOM, with hidden nodes already gone and every control carrying its
role, name, and state. The agent picks `@e3` and passes it straight back as the
target of a click, so the thing it acts on is the thing it saw. A ref from an
older view is refused rather than resolved against whatever now occupies that
position.

## The surface

| Member | What it does |
| --- | --- |
| `OpenSession` / `CloseSession` / `ListSessions` | Launch or attach a browser, and give it back |
| `Navigate` | Go somewhere, waiting as far as `commit`, `load`, or `networkIdle` |
| `Snapshot` | The accessibility tree, with refs |
| `Perform` | Click, fill, type, press, select, check, hover, scroll, wait, read, go back |
| `ReadPage` | The page as text, Markdown, or serialized DOM |
| `Evaluate` | JavaScript in, value out |
| `Screenshot` + `ReadOutput` / `ReleaseOutput` | An image, collected in chunks |
| `ListDownloads` / `WaitDownload` | Retained Chrome download events and completed-file handles |
| `ContractVersion` | What a host checks before its first real call |

Every name and payload is published by `tinybrowser-bus`, a two-dependency crate
a host links instead of repeating string literals.

## Using it

### From Rust, directly

```rust,no_run
use tinybrowser::{Action, Browser, NavigateRequest, SnapshotRequest, Target};

# async fn example() -> tinybrowser::Result<()> {
let browser = Browser::new();
let session = browser.open_session(Default::default()).await?;

browser.navigate(&session.id, &NavigateRequest::new("https://example.com")).await?;
let snapshot = browser.snapshot(&session.id, &SnapshotRequest::interactive()).await?;
println!("{}", snapshot.tree);

browser
    .perform(&session.id, &Action::Click { target: Target::parse("@e1"), new_tab: false })
    .await?;
browser.close_session(&session.id).await?;
# Ok(())
# }
```

### From a host, over the bus

`crates/tinybrowser/examples/over_the_bus.rs` is the reference: load the module,
wait for it to claim its name, check the contract version, then call. Run it
against a module you have built:

```sh
cargo build -p tinybrowser --release
cargo run -p tinybrowser --example over_the_bus -- \
  target/release/libtinybrowser.so https://example.com
```

`docs/openhuman-integration.md` covers wiring it into an OpenHuman host and the
agent-facing tool that sits on top.

### Agentic control with Jev

`tinybrowser-control` is the optional fast decision layer. It keeps
`tinybrowser-bus` and the browser engine model-free, while one Jev request per
step chooses an operation and speculative targets from the current snapshot.
An independent completion answer checks `DONE`; deterministic Rust code owns
step limits, unchanged-page detection, and the stop before likely irreversible
clicks.

```rust,no_run
use std::collections::BTreeMap;
use tinybrowser_control::{JevController, TaskRequest};
use tinyjevclient::Client;

# async fn control(browser: &tinybrowser::Browser, session: &tinybrowser::SessionId)
#     -> Result<(), Box<dyn std::error::Error>> {
let task = TaskRequest::new("Search for TinyBrowser")
    .with_inputs(BTreeMap::from([("search query".into(), "TinyBrowser".into())]));
let result = JevController::new(Client::from_env()?)
    .run(browser, session, &task)
    .await?;
println!("{:?}", result.status);
# Ok(())
# }
```

The controller never accepts model-generated selectors, coordinates, scripts,
or form text. It maps closed choices back to current snapshot refs and values
the caller supplied locally. See the
[`Jev browser-control specification`](docs/specs/jev-browser-control.md) for the
full policy and stop conditions.

`tinybrowser-skills` packages the loadable agent instructions and JSON tool
schemas for `browser_task` and the low-level `browser` escape hatch. Its
compiled examples show how a harness discovers and installs those versioned
assets:

```sh
cargo run -p tinybrowser-skills --example list_assets
cargo run -p tinybrowser-skills --example print_skill
```

## Requirements

A Chrome or Chromium on the host, or a DevTools endpoint to attach to. The
module looks in the conventional places; `TINYBROWSER_CHROME` names one
explicitly, and `TINYBROWSER_CHROME_ARGS` adds launch flags every session needs
— `--no-sandbox` on a host where unprivileged user namespaces are restricted,
most often.

A host that already runs a browser should point the module at it instead, with
`SessionOptions::endpoint`.

## Development

```sh
git submodule update --init --recursive

cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --all-targets --all-features
cargo test --all-features
```

The end-to-end suite drives a real browser and is opt-in, because a runner
without one would fail it for the wrong reason:

```sh
TINYBROWSER_LIVE_TESTS=1 cargo test -p tinybrowser --test live_chrome
```

Credentialed controller checks are separately opt-in. They read
`OPENROUTER_API_KEY` from the environment and never print it:

```sh
TINYBROWSER_OPENROUTER_LIVE_TESTS=1 \
  cargo test -p tinybrowser-control --test live_control \
  live_openrouter_completes_a_multi_step_browser_task -- --nocapture

TINYBROWSER_TRIP_LIVE_TESTS=1 \
  cargo test -p tinybrowser-control --test live_trip_com -- --nocapture

TINYBROWSER_DOWNLOAD_LIVE_TESTS=1 \
TINYBROWSER_DOWNLOAD_DIR=/absolute/download/directory \
  cargo test -p tinybrowser-control --test live_tinyhumans_download -- --nocapture
```

`AGENTS.md` is the full working agreement. `CLAUDE.md` is a symlink to it.

## Credit

The design owes a great deal to Vercel's
[`agent-browser`](https://github.com/vercel-labs/agent-browser): the
accessibility tree as the thing an agent reads, `@ref` addressing scoped to a
snapshot, and hit-testing a click point before dispatching at it. See
`THIRD-PARTY.md`. Its pinned source lives at `vendor/agent-browser` for feature
comparison and compatibility work. The Jev transport is pinned separately at
`vendor/tinyjevclient` and linked only by `tinybrowser-control`.

## License

GPL-3.0-only. See `LICENSE`.
