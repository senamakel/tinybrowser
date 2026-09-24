# TinyBrowser Module

This package contains the native `tinybrowser` module for TinyBus module ABI
v1. Install only the archive matching the host operating system and
architecture.

The module claims `ai.tinyhumans.tinybrowser.Browser`, serves the object at
`/ai/tinyhumans/tinybrowser/Browser`, and provides twelve methods:
`OpenSession`, `CloseSession`, `ListSessions`, `Navigate`, `Snapshot`,
`Perform`, `ReadPage`, `Evaluate`, `Screenshot`, `ReadOutput`, `ReleaseOutput`,
and `ContractVersion`. Every payload type, the interface name, the object path,
and the member names are published as the `tinybrowser-bus` crate, so a host
names them from a library rather than by string literal.

## What it needs from the host

A Chrome or Chromium, or a DevTools endpoint to attach to. The module looks in
the conventional install locations for the platform. Two environment variables
override that, and both describe the machine rather than any one caller:

- `TINYBROWSER_CHROME` — the browser binary to launch.
- `TINYBROWSER_CHROME_ARGS` — extra launch flags for every session.
  `--no-sandbox` belongs here on a host where unprivileged user namespaces are
  restricted, which is the default on Ubuntu 23.10 and later and in containers
  without the right capabilities.

A launched browser gets a fresh profile directory that is removed when its
session closes, so one session's cookies and logins never reach the next. A
session that attaches to an endpoint leaves that browser running when it closes.

## Operational notes

A session is a browser process. The module holds at most eight at once and
refuses the ninth rather than starting it; a host is expected to close what it
opens. Screenshots are held for collection, capped at 16 outstanding and expired
after five minutes, and a host that reads one to completion should release it.

The `allowed_origins` on a session refuses explicit navigations and intercepts
document requests from clicks, redirects, and page scripts before Chrome sends
them. It is a navigation boundary, not a whole-network sandbox: subresources
and scripts can still contact other hosts. A host needing that stronger boundary
isolates the browser process.

Use `https://.example.com` to admit one HTTPS host and its subdomains while
refusing HTTP. A bare `.example.com` matches both schemes for hosts that need
that behavior.

## Installing

The archive contains one `.so`, `.dylib`, or `.dll` plus `modules.toml`. Keep
those files together when copying them into a TinyBus module directory. The
allowlist binds the native library filename to its SHA-256 digest so TinyBus can
reject a missing, renamed, or modified artifact before initialization.

The GitHub release also publishes `checksum.toml` as a separate asset. TinyBus
checks that manifest before downloading and extracting the selected platform
archive. Install directly from a tagged release with:

```sh
tinybus modules load-github \
  https://github.com/tinyhumansai/tinybrowser/releases/tag/v0.1.0 \
  tinybrowser-0.1.0-ubuntu-24.04-x86_64.tar.gz \
  <archive-sha256>
```

TinyBus modules are trusted in-process code. Install release artifacts only
from a trusted source and restart the host after replacing a loaded module.
