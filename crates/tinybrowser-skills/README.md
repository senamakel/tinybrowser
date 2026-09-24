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
