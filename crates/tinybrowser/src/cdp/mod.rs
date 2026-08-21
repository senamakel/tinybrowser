//! The Chrome DevTools Protocol: the socket, the browser behind it, and the
//! commands this crate sends over it.
//!
//! # Why CDP and not WebDriver
//!
//! WebDriver is a request-per-action HTTP protocol with a session-shaped API and
//! a separate driver binary to install, version-match, and keep alive. CDP is a
//! single bidirectional socket straight into the browser: one connection
//! multiplexes every command and every event, the accessibility tree and the
//! screenshot compositor are first-class, and there is no third process to go
//! out of step with Chrome. For a module that must launch, drive, and tear down
//! a browser from inside somebody else's daemon, that is the smaller moving
//! part.
//!
//! # Layout
//!
//! - [`client`] — the socket: request/response multiplexing and event fan-out.
//! - [`endpoint`] — turning what a host configured into a browser socket URL.
//! - [`launch`] — finding a Chrome on this host and starting one.
//!
//! # Credit
//!
//! The protocol usage here — flat session attachment, the accessibility tree as
//! the snapshot source, hit-testing a click point before dispatching to it —
//! follows the approach taken by Vercel's `agent-browser`
//! (<https://github.com/vercel-labs/agent-browser>, Apache-2.0). See
//! `THIRD-PARTY.md` at the repository root.

pub(crate) mod client;
pub(crate) mod endpoint;
pub(crate) mod launch;

pub(crate) use client::{CdpClient, CdpEvent};
