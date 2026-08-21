# Roadmap

What exists, what is next, and what is deliberately out of scope.

## Shipped

- a CDP engine: launching or attaching, one multiplexed socket, flat page
  sessions, and teardown that does not leave a browser behind
- accessibility snapshots rendered as indented text with `@ref` addressing,
  scoped to the generation that minted them
- interactions through real input events — click with hit-testing, fill, type,
  press, select, check, hover, scroll, wait — addressed by ref, CSS selector, or
  semantic locator
- extraction as text, Markdown, or serialized DOM, and JavaScript evaluation
- screenshots, held and collected in chunks with a digest to verify them
- an error taxonomy where every failure carries a stable wire name, and the
  contract says which of them an agent can act on
- the `TinyBus` module: twelve members, a version handshake, and a `cdylib` that
  CI loads through the real dynamic loader
- an end-to-end suite against a real Chrome, serving its own fixtures on
  loopback

## Next

- multiple tabs per session: `Tabs`, `SelectTab`, and adopting the page a click
  with `new_tab` opened, which currently opens a tab the session does not drive
- cookie and storage state a host can save and restore, so a session can resume
  an authenticated one without the module holding credentials
- network interception: refusing or recording requests, which is also the only
  way to make the origin policy a boundary rather than a guard rail
- `Snapshot` diffing, so an agent acting in a loop reads what changed rather than
  the whole tree each time

## Out Of Scope

- an agent, a model, or tool schemas. This module drives a browser and describes
  what it sees; deciding what to click is the host's job, and a module shipping
  its own prompt would be one more thing to keep in step with a model it cannot
  see.
- a sandbox. `allowed_origins` is a guard rail an in-page navigation can defeat,
  and it says so. A host that needs a boundary puts the browser in a network
  namespace.
- persistence. Sessions live in memory and end with the process.
- downloading a browser. Finding one is a fixed list and an environment
  variable; a module that fetches and executes a binary is a different kind of
  thing.
- anything that cannot be tested deterministically, or only against a live page
  on the public internet.
