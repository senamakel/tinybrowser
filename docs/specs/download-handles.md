# Download event handles

**Status:** Implemented
**Owner:** TinyBrowser maintainers

## Problem

A browser click can start a download without changing the page. A controller
that observes only accessibility snapshots cannot distinguish “still waiting”
from “the file completed,” so it spends model calls until an unchanged-page or
step budget stops it. Hosts also cannot verify which file Chrome created
without polling an ambient directory outside the browser contract.

## Goals

- Let a session opt into an explicit absolute download directory.
- Track Chrome download events even when they occur between host calls.
- Return typed handles for in-progress, completed, and cancelled downloads.
- Let a host wait for the next unreported terminal download without polling.
- Carry the completed local path, source URL, suggested filename, byte counts,
  and stable Chrome download id across TinyBus.

## Non-goals

- Downloads are not copied into TinyBus frames or held in memory.
- TinyBrowser does not open, mount, install, or execute downloaded files.
- The download handle is not proof that a file is trusted; callers still verify
  checksums, signatures, and expected media type.
- No model decision grants filesystem access. The host chooses the directory
  when opening the session.

## Proposed behavior

`SessionOptions::download_dir` optionally names an absolute directory on the
module host. Opening the session creates it when absent and configures Chrome to
allow downloads there while emitting `Browser.download*` events.

Every observed download becomes a `DownloadInfo` with a monotonic sequence,
`DownloadId`, URL, suggested filename, `DownloadState`, received and total byte
counts, and the expected local path when a directory was configured.

Two additive TinyBus members are published:

- `ListDownloads(SessionId) -> Vec<DownloadInfo>` returns every retained handle
  in sequence order.
- `WaitDownload(SessionId, DownloadWaitRequest) -> DownloadInfo` blocks until
  the earliest completed or cancelled handle not returned by an earlier wait.
  An event that arrived before the call remains available, so callers cannot
  miss a fast download between click and wait.

`DownloadWaitRequest::timeout_ms` defaults to the session deadline. A timeout
uses the existing stable timeout error. Cancelled downloads are returned as
handles rather than hidden as transport failures.

## Invariants

- Relative download directories are rejected before Chrome is configured.
- Suggested filenames are reduced to their final path component before a local
  path is reported.
- Browser events are retained until the session closes.
- Each terminal handle is returned by `WaitDownload` at most once.
- Listing does not consume handles.
- Closing a session stops its event monitor but does not delete an explicit
  download directory or its files.
- Contract version is increased additively from `1.0` to `1.1`.

## Acceptance criteria

- Pure tests cover event ordering, retained fast events, one-time delivery,
  progress, cancellation, and filename sanitization.
- A live Chrome test downloads a loopback fixture and receives a completed
  handle without directory polling.
- The TinyBus adapter round-trips both new members.
- The existing four workspace contract commands pass.

## Open questions

None block this implementation. Streaming remote downloads over TinyBus can be
specified separately if a future host cannot access the module host's path.
