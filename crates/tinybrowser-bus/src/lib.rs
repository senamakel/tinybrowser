//! Every type that crosses the tinybrowser module's `TinyBus` boundary, and the
//! names of the members that carry them.
//!
//! tinybrowser drives a real Chrome over the Chrome DevTools Protocol and
//! publishes that as a handful of bus members an agent host can build tools on:
//! open a session, navigate, snapshot the accessibility tree, act on a ref, read
//! the page, screenshot it, close. This crate is the vocabulary those members
//! exchange.
//!
//! It ships as a loadable `TinyBus` module: `crates/tinybrowser` is built as a
//! `cdylib` and exports one object. A host that loads that binary can call into
//! it but cannot `use` anything out of it, so the payload vocabulary has to be
//! published as an ordinary library. This is that library.
//!
//! # What is here
//!
//! - [`names`] — the interface name, the object path, and one constant per
//!   member, plus [`names::METHODS`] listing them in dispatch order.
//! - [`session`] — opening, listing, and closing the browser a host drives.
//! - [`page`] — navigating, extracting a page as text, evaluating JavaScript.
//! - [`snapshot`] — the accessibility tree, and the refs that address it.
//! - [`action`] — every interaction, and the three ways to name an element.
//! - [`output`] — screenshots, and the handle protocol that carries them.
//! - [`errors`] — the failure names, and which of them an agent can act on.
//! - [`version`] — [`CONTRACT_VERSION`] and the [`is_compatible`] bind rule.
//!
//! # What is deliberately not here
//!
//! **No behavior.** No CDP, no browser launching, no element resolution. All of
//! that lives in `crates/tinybrowser`, which depends on this crate and
//! re-exports it. A payload type describes what a frame carries, not what the
//! module does with it.
//!
//! **No transport.** This crate does not depend on `tinybus` and holds no
//! connection, client, or codec. A host already owns its connection — its
//! reconnect policy, its timeouts, its tracing — and the useful part is the
//! vocabulary, not another wrapper around it.
//!
//! That is also a structural necessity, not only a preference: `tinybus` is
//! vendored as a submodule whose manifest inherits fields from its own nested
//! `[workspace.package]`. A crate that every workspace member can depend on has
//! to stay transport-free, and staying transport-free is what keeps this crate
//! down to two pure-Rust dependencies — which matters, because the host linking
//! it is a binary that deliberately does *not* want a browser stack in its
//! build.
//!
//! # This crate sits underneath the implementation, not beside it
//!
//! `tinybrowser` **depends on this crate and re-exports all of it**, so
//! `tinybrowser::Action` and `tinybrowser_bus::action::Action` are the *same
//! type*, not structural twins. Defining a parallel set of payload types for
//! hosts would mean a conversion at every call site that nothing checks. One
//! definition, here, at the bottom.
//!
//! So: a module author depends on `tinybrowser` and gets behavior and
//! vocabulary. A host depends on `tinybrowser-bus` and gets vocabulary alone.
//!
//! # Staying in step with the module
//!
//! [`names::METHODS`] lists every member. `crates/tinybrowser` asserts its
//! served members against that list, in order, so a method added to the
//! interface without an entry here fails that crate's tests rather than
//! surfacing as an unknown method in a host at runtime.
//!
//! # Example
//!
//! Building the frame bodies for the loop an agent actually runs — open,
//! navigate, snapshot, click — without a bus in sight:
//!
//! ```
//! use tinybrowser_bus::{
//!     names, Action, NavigateRequest, SessionId, SessionOptions, SnapshotRequest, Target,
//! };
//!
//! let session = SessionId::new("s-1");
//!
//! let open = serde_json::to_value((SessionOptions::default(),))?;
//! let navigate = serde_json::to_value((&session, NavigateRequest::new("https://example.com")))?;
//! let snapshot = serde_json::to_value((&session, SnapshotRequest::interactive()))?;
//! let click = serde_json::to_value((
//!     &session,
//!     Action::Click { target: Target::parse("@e12"), new_tab: false },
//! ))?;
//!
//! assert_eq!(names::methods::OPEN_SESSION, "OpenSession");
//! assert_eq!(navigate[1]["wait_until"], "load");
//! assert_eq!(snapshot[1]["interactive_only"], true);
//! assert_eq!(click[1]["target"], serde_json::json!({ "kind": "ref", "value": "e12" }));
//! # let _ = open;
//! # Ok::<(), serde_json::Error>(())
//! ```

pub mod action;
pub mod errors;
pub mod names;
pub mod output;
pub mod page;
pub mod session;
pub mod snapshot;
pub mod version;

pub use action::{Action, ActionOutcome, LocateBy, Locator, ScrollDirection, Target, WaitState};
pub use names::{INTERFACE, METHODS, OBJECT_PATH};
pub use output::{ImageFormat, OutputChunk, OutputId, OutputRef, ScreenshotRequest};
pub use page::{
    EvaluateRequest, NavigateRequest, PageState, PageText, ReadFormat, ReadRequest, WaitUntil,
};
pub use session::{SessionId, SessionInfo, SessionOptions, Viewport};
pub use snapshot::{ElementRef, Snapshot, SnapshotRequest};
pub use version::{CONTRACT_VERSION, is_compatible};
