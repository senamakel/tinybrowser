//! A Chrome DevTools Protocol browser engine for agents, packaged as an
//! installable `TinyBus` module.
//!
//! # What this is for
//!
//! An agent host that wants to look at a web page has bad options. Shelling out
//! to a browser CLI means a subprocess, a JSON parser around its output, and a
//! binary to install and version-match. Linking a browser stack into the host
//! means dragging a WebSocket client, a TLS stack, an image codec and a protocol
//! surface into a binary that mostly does something else — and a crash anywhere
//! in it is a crash in the host.
//!
//! This crate is the third option. It is a complete browser engine — launching,
//! navigation, accessibility snapshots, real input events, extraction,
//! screenshots — that ships as a `cdylib` a host loads over `TinyBus`. The
//! host's build stays as it was; what it gains is a handful of bus members and
//! [`tinybrowser_bus`], a dependency-light vocabulary crate, to spell them with.
//!
//! # The loop it is shaped around
//!
//! ```text
//! OpenSession  ->  Navigate  ->  Snapshot  ->  Perform  ->  Snapshot  ->  ...
//!                                    |             |
//!                                ReadPage      Screenshot
//! ```
//!
//! [`Snapshot`] is the important one. It renders the page's accessibility tree
//! as indented text with a `@e12` ref on everything actionable, which is an
//! order of magnitude smaller than the DOM and already excludes what a screen
//! reader would not announce. An agent reads that, picks a ref, and passes it
//! straight back as a [`Target`] — so the thing it acts on is the thing it saw.
//!
//! # Entry points
//!
//! - [`Browser`] — the engine. One method per bus member; usable directly from
//!   Rust with no bus involved.
//! - [`Error`] — every failure, each mapping to one published wire name.
//! - Everything from [`tinybrowser_bus`], re-exported: [`Action`],
//!   [`SessionOptions`], [`Snapshot`], [`Target`], and the rest.
//!
//! # Example
//!
//! ```no_run
//! use tinybrowser::{Action, Browser, NavigateRequest, SnapshotRequest, Target};
//!
//! # async fn example() -> tinybrowser::Result<()> {
//! let browser = Browser::new();
//! let session = browser.open_session(Default::default()).await?;
//!
//! browser
//!     .navigate(&session.id, &NavigateRequest::new("https://example.com"))
//!     .await?;
//!
//! let snapshot = browser.snapshot(&session.id, &SnapshotRequest::interactive()).await?;
//! println!("{}", snapshot.tree);
//!
//! if let Some(link) = snapshot.refs.iter().find(|element| element.role == "link") {
//!     browser
//!         .perform(
//!             &session.id,
//!             &Action::Click { target: Target::reference(&link.id), new_tab: false },
//!         )
//!         .await?;
//! }
//!
//! browser.close_session(&session.id).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # What is deliberately not here
//!
//! **No agent, no model, no tool schemas.** This crate drives a browser and
//! describes what it sees. Deciding what to click is the host's job, and a
//! module that shipped its own prompt would be one more thing to keep in step
//! with a model it cannot see.
//!
//! **No sandbox.** The origin allowlist in [`SessionOptions`] is a guard rail
//! against an agent wandering off, not a boundary — a page's own JavaScript can
//! navigate around it. A host that needs a real boundary puts the browser in a
//! network namespace. Saying so plainly is more useful than implying otherwise.
//!
//! **No persistence.** Sessions are held in memory and end with the process.
//! Cookies and logins live in the profile directory for the life of a session
//! and are removed with it, unless the host named its own.
//!
//! # Credit
//!
//! The design owes a great deal to Vercel's `agent-browser`
//! (<https://github.com/vercel-labs/agent-browser>, Apache-2.0): the
//! accessibility tree as the thing an agent reads, `@ref` addressing scoped to a
//! snapshot, and hit-testing a click point before dispatching at it are all
//! taken from it. See `THIRD-PARTY.md` at the repository root.

mod capture;
mod cdp;
mod engine;
pub mod error;
mod extract;
mod interact;
mod session;
mod snapshot;

pub mod tinybus_module;

pub use engine::Browser;
pub use error::{Error, Result};

/// The wire contract, re-exported whole.
///
/// `tinybrowser::Action` and `tinybrowser_bus::Action` are the *same type*, not
/// structural twins: this crate depends on the contract rather than restating
/// it, so a host and a module cannot drift.
pub use tinybrowser_bus;

pub use tinybrowser_bus::{
    Action, ActionOutcome, CONTRACT_VERSION, ElementRef, EvaluateRequest, INTERFACE, ImageFormat,
    LocateBy, Locator, METHODS, NavigateRequest, OBJECT_PATH, OutputChunk, OutputId, OutputRef,
    PageState, PageText, ReadFormat, ReadRequest, ScreenshotRequest, ScrollDirection, SessionId,
    SessionInfo, SessionOptions, Snapshot, SnapshotRequest, Target, Viewport, WaitState, WaitUntil,
    errors, is_compatible, names,
};
