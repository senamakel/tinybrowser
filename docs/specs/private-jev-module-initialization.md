# Private Jev module initialization

**Status:** Accepted
**Owner:** TinyBrowser maintainers

## Problem

A future loadable browser-control module needs provider credentials and endpoint
policy without exposing secrets through ordinary TinyBus calls, monitoring,
argv, task traces, skills, or persisted module state.

## TinyBus mechanisms

TinyBus has two different protected paths, and the distinction is load-bearing:

- **Sensitive module control** carries initialization and reinitialization
  configuration. It terminates at the trusted broker/module host, which must
  read it to initialize an in-process module. Load and reinitialize control
  bodies are never fanned to signal subscribers or rendered by the monitor, and
  serialized ABI buffers are zeroized after use.
- **Confidential method calls** go only to an attested well-known module name.
  A method annotated `#[tinybus(confidential)]` rejects ordinary calls before
  typed argument decoding. Confidential signals do not exist: TinyBus refuses
  them because a broadcast has no single attested recipient.

Jev's initial and replacement configuration belongs in sensitive module
control, not in a public event or an ordinary method.

## Proposed control module

The browser engine remains provider-neutral. A separately loadable
`tinybrowser-control-module` will depend on `tinybrowser-control`,
`tinyjevclient`, and the pure `tinybrowser-bus` contract. It will declare a
typed setup configuration:

```rust,ignore
#[derive(serde::Deserialize)]
struct ControlConfig {
    provider: ProviderConfig,
    api_key: String,
    default_max_steps: usize,
    default_max_cost_usd: f64,
}

async fn setup(connection: tinybus::Connection, config: ControlConfig)
    -> tinybus::Result<()>;

tinybus_module::module_export! {
    setup = setup,
    config = ControlConfig,
    // manifest fields omitted
}
```

The host supplies JSON through `ModuleHost::set_config` / `with_config`,
`load_file_with_config`, or the CLI's `--config-file -`. Rotation uses
`ModuleHost::reinitialize` or `Connection::reinitialize_module`; the typed setup
function validates all replacement values before atomically swapping live
state.

## Secret lifetime

- Deserialize during setup, immediately move key bytes into a redacting,
  zeroizing secret holder, and drop the typed configuration value.
- Never implement `Debug`, serialization, cloning, or error text that exposes
  key bytes.
- Never write configuration to module-owned disk.
- Reinitialization builds and validates a complete replacement first, swaps it
  atomically, then drops the previous secret.
- Tasks bind to a configuration generation so an in-flight task is not partly
  evaluated with two providers.
- Traces report provider, resolved model, generation, token usage, latency, and
  cost only.

TinyBus's `tinybus::Secret` reduces in-process exposure but is not an isolation
boundary: an in-process module is already inside the host address space.

## Current architecture

Today `tinybrowser-control` runs in the harness, so no credential crosses the
bus. The host constructs `tinyjevclient::Client` from its own secret store and
only deterministic browser calls use TinyBus. Private module initialization is
needed when the controller becomes a separately loaded module, not before.

## Acceptance criteria for that future module

- Initial configuration is supplied only through sensitive module control.
- Live rotation succeeds through typed reinitialization without unloading the
  browser or dropping its bus connection.
- Invalid replacement configuration leaves the prior generation active.
- Monitor output, errors, traces, panic paths, and debug formatting contain no
  key material.
- An ordinary call to any runtime credential-update member is rejected before
  arguments are decoded; preferably no such public member exists.
- Tests use canary key material and scan every captured observation surface for
  its absence.
