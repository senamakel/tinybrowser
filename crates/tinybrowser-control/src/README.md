# TinyBrowser control internals

This crate is the optional host-side decision layer. `lib.rs` owns the bounded
observe-decide-act loop; `policy.rs` is pure construction and decoding of one
System One request; `types.rs` exposes goals, finite limits, decisions, traces,
and terminal statuses; `error.rs` preserves the distinction between invalid
policy, provider failures, and browser failures.

Each ambiguous step is one Jev request. The operation, each applicable target
class, and goal completion are independent heads over closed criteria. A target
or caller input with only one candidate is resolved locally because a Jev
Choice requires at least two options. No provider answer becomes a selector,
coordinate, script, or text value.

The runner then applies deterministic policy: accept completion only when the
independent Noul clears the threshold, and give one bounded reconsideration
without `DONE` when it does not. A newly selected click still uses a ref from
the current snapshot and passes the irreversible-click gate. The runner also
stops offering `FILL` after a single caller-supplied input was successfully
entered while the resulting page snapshot remains the same and still contains
the filled ref, so repeated fills cannot consume the unchanged-page budget. A
changed page makes the input available again. It
retries only browser errors the wire contract calls recoverable, stops after
repeatedly unchanged snapshots, and enforces a finite decision budget. The
browser engine still owns origin policy, current-ref validation, hit testing,
and input dispatch.

`test.rs` exercises policy and the loop with in-memory seams. The opt-in
`tests/live_control.rs` test adds a real Chrome plus local page and mock System
One server, proving the public integration without external credentials.
