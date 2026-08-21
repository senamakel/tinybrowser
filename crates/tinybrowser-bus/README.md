# tinybrowser-bus

Every type that crosses the tinybrowser module's `TinyBus` boundary, and the
names of the members that carry them.

tinybrowser ships as a loadable module so a host does not compile a browser:
`crates/tinybrowser` is built as a `cdylib` and exports one object. A host can
load that binary but cannot `use` anything out of it, so the payload vocabulary
has to be published as an ordinary library. This is it.

| module     | what it holds                                                  |
| ---------- | -------------------------------------------------------------- |
| `names`    | interface name, object path, one constant per member            |
| `session`  | opening, listing, and closing the browser a host drives         |
| `page`     | navigating, extracting a page as text, evaluating JavaScript    |
| `snapshot` | the accessibility tree, and the refs that address it            |
| `action`   | every interaction, and the three ways to name an element        |
| `output`   | screenshots, and the handle protocol that carries them          |
| `errors`   | the failure names, and which of them an agent can act on        |
| `version`  | `CONTRACT_VERSION` and the bind rule a host applies to it       |

Two dependencies, both pure Rust: `serde` and `serde_json`.

## This crate sits underneath `tinybrowser`

`tinybrowser` depends on this crate and re-exports all of it, so
`tinybrowser::Action` and `tinybrowser_bus::action::Action` are the *same type*,
not structural twins. Defining a parallel set of payload types for hosts would
mean a conversion at every call site that nothing checks. One definition, here,
at the bottom.

So: a module author depends on `tinybrowser` and gets behavior and vocabulary. A
host depends on `tinybrowser-bus` and gets vocabulary alone — which matters,
because that host is usually a binary that deliberately does not want a browser
stack in its build. That is the entire reason this split exists.

## No transport, on purpose

This crate holds no connection, client, or codec, and does not depend on
`tinybus`. A host already owns its connection — its reconnect policy, its
timeouts, its tracing — and the useful part is the vocabulary, not another
wrapper around it.

It is also a structural necessity: `tinybus` is vendored as a submodule whose
manifest inherits from its own nested `[workspace.package]`. A crate every
member can depend on has to stay transport-free, and CI asserts it does.

## Nothing here is `#[non_exhaustive]`

Both sides construct these types — a host builds the requests, the module builds
the replies — and the module is a different crate from this one. Non-exhaustive
types would leave the implementation unable to build its own replies, and would
turn `..Default::default()` into a compile error for every caller.

The evolution mechanism is `CONTRACT_VERSION` and the `is_compatible` bind rule
instead, which is the one that works across a dynamically loaded boundary. Every
request type carries `#[serde(default)]`, so an added field is additive; a host
deserializing a reply ignores what it does not know. Neither is something the
Rust attribute could have enforced through a `cdylib` anyway.
