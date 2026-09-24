# Jev browser control implementation plan

Specification: [`../specs/jev-browser-control.md`](../specs/jev-browser-control.md)

## Goal

Ship a bounded Jev-driven task loop over TinyBrowser's existing typed engine,
with pinned upstream sources and deterministic safety gates.

## Plan

1. Add `vendor/agent-browser` and `vendor/tinyjevclient` as pinned submodules;
   document why one is reference source and the other is a linked dependency.
2. Add the `tinybrowser-control` workspace crate and failing policy tests for
   operation/target question construction and response decoding.
3. Implement the pure policy builder and decoder so one validated Jev response
   becomes a typed `Decision`.
4. Add failing loop-policy tests for completion confirmation, irreversible
   clicks, and unchanged-page detection; implement the bounded runner over
   `tinybrowser::Browser`.
5. Add crate docs, a compiled example, and repository documentation describing
   the new layer and its deliberate boundary from the bus contract.
6. Run formatting, Clippy, all-target builds, tests, rustdoc, and doctests.

## Completion checklist

- [x] Pinned upstream submodules
- [x] Pure decision policy and tests
- [x] Bounded controller loop and tests
- [x] Public documentation and example
- [x] Full local validation
