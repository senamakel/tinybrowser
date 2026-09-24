# tinybrowser-skills

This crate packages the agent-facing assets that sit above TinyBrowser:

- `skills/tinybrowser/SKILL.md` teaches an agent when and how to use the
  high-level task tool and the low-level browser escape hatch.
- `schemas/browser_task.schema.json` is the compact default tool: one
  outcome-oriented goal, finite budgets, and no model-controlled authority.
- `schemas/browser.schema.json` is the low-level navigate, observe, and act
  surface used for recovery and unsupported workflows.

The Rust library embeds those files so a harness can install or register the
exact assets compiled with its TinyBrowser dependency. It does not start a
browser, call a provider, or hold credentials.

## Get started as an agent

When your harness exposes `browser_task` and `browser`, read
[`skills/tinybrowser/SKILL.md`](skills/tinybrowser/SKILL.md), then call
`browser_task` with a goal that can be checked on the final page. For example:

```json
{
  "goal": "Find visible one-way flight options from Mumbai (BOM) to Goa Dabolim (GOI). Do not book or purchase.",
  "start_url": "https://www.trip.com/flights/city-bom-airport-goi/",
  "allowed_origins": ["trip.com"]
}
```

Use `inputs` for exact caller-supplied form values. If the task stops without
confirmed success, use `browser` to inspect the latest snapshot or recover
with a current element ref. Downloads require a completed `wait_download`
handle. Follow the skill's confirmation rules before any consequential action.

## Connect a harness

1. Depend on `tinybrowser-skills`, `tinybrowser-control`, and the browser API
   appropriate for the host. Open a browser session with the host's origin and
   download policy.
2. Register `tool_schema("browser_task")` and `tool_schema("browser")` as the
   agent's tool definitions. Load `TINYBROWSER_SKILL` into its instructions,
   or install the relative files returned by `skill_assets()` into the host's
   skill directory.
3. Dispatch `browser_task` through `JevController::run` on the current
   session. Dispatch `browser` to the corresponding typed TinyBrowser calls.
   The host supplies the Jev client and closes sessions when finished.
4. Enforce the schema's `max_seconds` and `max_cost_usd` in the host. The Rust
   controller enforces its own step and unchanged-page limits. Return the
   result status, final URL and evidence, action trace, and provider usage to
   the agent. Route any pending consequential action through the host's user
   confirmation mechanism.

The schemas describe a tool interface; this crate does not implement a tool
server or dispatcher. See the [runtime guide](../../docs/jev-integration.md)
for the exact Jev request, stop conditions, and usage accounting.

```rust
use tinybrowser_skills::{TINYBROWSER_SKILL, tool_schema};

assert!(TINYBROWSER_SKILL.contains("browser_task"));
assert!(tool_schema("browser_task").is_some());
```

Runnable demonstrations live in `examples/`:

```sh
cargo run -p tinybrowser-skills --example print_skill
cargo run -p tinybrowser-skills --example list_assets
```
