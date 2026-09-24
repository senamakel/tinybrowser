# Download event handles implementation plan

Specification: [`../specs/download-handles.md`](../specs/download-handles.md)

1. Add download payloads and serde contract tests to `tinybrowser-bus`; add
   `download_dir` to session options and bump the contract minor version.
2. Add a session-owned download tracker with pure state-transition tests.
3. Subscribe before enabling Chrome download events, retain handles, and expose
   list/wait operations through `engine::Browser`.
4. Add the two translation-only TinyBus members and adapter tests.
5. Replace the TinyHumans live download test's directory polling with
   `WaitDownload`, then verify the official checksum.
6. Update public documentation and run full validation.

## Completion checklist

- [x] Contract payloads and member names
- [x] Engine event tracker
- [x] TinyBus adapter
- [x] Live handle verification
- [x] Documentation and full validation
