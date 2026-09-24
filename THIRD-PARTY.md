# Third-Party Notices

## agent-browser

Copyright Vercel, Inc. Licensed under the Apache License, Version 2.0.

<https://github.com/vercel-labs/agent-browser>

The upstream source is pinned unchanged at `vendor/agent-browser` for feature
comparison, compatibility testing, and implementation reference. Its current
Rust package is a binary rather than a library, so no package in this workspace
links it. `crates/tinybrowser` remains an independent implementation, but its
design is taken from `agent-browser` and it would not look the way it does
without it. The specific debts:

- **The accessibility tree is what an agent reads.** Not the DOM, not a
  screenshot — the browser's own computed answer to "what is here and what does
  it do", rendered as indented text. This is the central idea, and it is theirs.
- **`@ref` addressing scoped to a snapshot.** Every actionable node in a
  snapshot carries a short ref an agent passes straight back, so the thing it
  acts on is the thing it saw, and a ref from an older view is refused rather
  than silently resolved against whatever now occupies that position.
- **Hit-testing before dispatching a click.** Measuring the click point,
  checking with `elementFromPoint` that the target is what sits there, and
  failing with the name of the covering element rather than delivering the click
  to a consent banner and reporting success.
- **Flat CDP session attachment**, with one socket multiplexing browser-level
  and page-level commands.
- **The interaction vocabulary** — click, fill, type, press, select, hover,
  scroll, wait, and semantic locators by role, text, label, placeholder, and
  test id — follows theirs closely, deliberately, so that a host can move
  between the two without relearning what the verbs mean.

No code is compiled or copied into TinyBrowser. The protocol conversation, the
error taxonomy, the session and held-output model, the origin policy, and the
whole `TinyBus` surface are this repository's own, and the licences differ —
`agent-browser` is Apache-2.0 and this project is GPL-3.0-only, a direction that
is compatible one way and not the other.

The Apache-2.0 licence requires that its notice travel with derivative work.
This file is that notice, and it is here whether or not the requirement strictly
attaches, because the credit is owed either way.

## TinyBus

`vendor/tinybus` is a git submodule of <https://github.com/tinyhumansai/tinybus>
and carries its own licence and copyright. It is pinned by gitlink and never
modified from this repository.

## TinyJevClient

`vendor/tinyjevclient` is a git submodule of
<https://github.com/tinyhumansai/tinyjevclient>, licensed GPL-3.0-only. It is
pinned by gitlink and linked by `tinybrowser-control` as the typed System One
transport. It remains provider transport only: task policy and browser effects
are owned by this repository.
