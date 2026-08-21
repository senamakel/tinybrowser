# TinyBus Adapter

This module is the boundary between the browser engine and TinyBus module ABI
v1. `BrowserService` exposes each method of `crate::Browser` as a typed bus
member, and `setup` registers its object and claims the well-known interface
name. Neither the name, the object path, nor the payload types are spelled here:
they come from `tinybrowser-bus`, so a rename is a compile error in every
consumer instead of an `UnknownMethod` at runtime.

## There are no decisions in this layer

Each member deserializes its arguments, calls one method on the engine, and maps
an `Error` to a wire error name. That is deliberate. Anything decided here could
only be tested through a bus, and a Rust caller using `Browser` directly would
not get it — so a rule that belongs to the browser goes in the engine, and this
file stays a translation.

The one thing it does own is the engine's lifetime. A process-wide `OnceLock`
holds it, because the sessions it keeps are browser processes: a per-call engine
would launch and discard a browser for every navigation, and a per-connection
one would strand sessions the moment a host reconnected. TinyBus never unloads a
module, so "the life of the process" and "the life of the module" are the same
thing.

## Errors travel as names

A host does not show a model a raw failure string; it decides what *kind* of
failure happened. A bad selector is something a model can fix by taking a fresh
snapshot. A browser that will not launch is not. A navigation refused by policy
must never be retried. `to_bus` puts a stable name from
`tinybrowser_bus::errors` on every failure and leaves the prose in the message,
so a host matches on a constant rather than on wording that will be reworded.

## Keeping the surface honest

`tinybus_module::module_export!` emits the descriptor, embedded manifest, and
initialization symbols consumed by the dynamic loader. The manifest method list
must stay aligned with the interface macro's dispatch table and with
`tinybrowser_bus::names::METHODS`; the unit tests check both relationships, so a
member added to one and forgotten in another fails the build rather than
surfacing as an unknown method in a host at runtime.

Integration tests use TinyBus's in-memory transport and need no browser: the
members they exercise are the ones that reach a decision before a session is
looked up, which is exactly what this layer is responsible for.
`crates/tinybrowser/examples/verify_module.rs` loads a compiled `cdylib` through
the real dynamic loader before a release archive is accepted, and
`examples/over_the_bus.rs` drives a real page through it end to end.

Code here must not retain Rust-owned data across the ABI boundary or bypass the
SDK exports with an ad hoc FFI surface.
