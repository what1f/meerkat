# Workspace architecture

This document describes Lankide's public module boundaries, dependency graph,
and verification contract.

## Workspace layout

The root `Cargo.toml` is the only Cargo workspace. The new implementation is
split into small crates and application composition roots:

```text
crates/core                 pure domain vocabulary
crates/protocol             Runtime-Computer/client wire DTOs
crates/provider             Provider contracts and normalized events
crates/harness              Pi/Codex/Claude Code execution contract
crates/storage              Runtime database boundary
crates/runtime              scheduling, persistence, Provider lifecycle
crates/computer             Agent home and Harness execution boundary
crates/daemon-runtime       shared daemon filesystem and instance ownership
crates/client               management client contract and test support
crates/desktop-ui           reusable GPUI shell and presentation primitives
crates/testkit              shared business-scenario fixtures

providers/feishu            Feishu composition adapter
providers/telegram          Telegram composition adapter
harnesses/pi                Pi composition adapter
harnesses/codex             Codex composition adapter
harnesses/claude-code       Claude Code composition adapter

apps/runtime-daemon         Runtime composition root
apps/computer-daemon        Computer composition root
apps/desktop                GPUI composition root
apps/lankide-cli            Agent-run-scoped capability CLI
```

Dependency direction is expressed as “A depends on B”:

```text
protocol  -> core
provider  -> core, protocol
harness   -> core, protocol
storage   -> core
runtime   -> core, protocol, provider, storage
computer  -> core, protocol, harness
daemon-runtime -> core
client         -> protocol
desktop-ui     -> GPUI

providers/*  -> core, provider
harnesses/*  -> core, harness

apps/runtime-daemon  -> runtime + daemon-runtime + concrete providers
apps/computer-daemon -> computer + daemon-runtime + concrete harnesses
apps/desktop         -> client + desktop-ui + GPUI
apps/lankide-cli     -> protocol
```

`core` never depends on a concrete Provider, Harness, database, transport, or
GPUI crate. Concrete Providers and Harnesses are assembled only in application
composition roots. A package may have additional strictly narrower dependencies
only when its public contract needs them; it must not reverse the arrows above.

## Test rule

The workspace does not use function-level unit tests. Do not add
`#[cfg(test)]` modules or unit-test functions under `src/`.

Every module gets a module-level end-to-end test under its `tests/` directory.
That test must enter through the module's public API and cover a real business
scenario. The purpose is to prove the feature works, not to maximize line or
branch coverage.

The machine-readable source is
[tests/e2e-modules.tsv](../tests/e2e-modules.tsv). Every listed module contract
has an implemented business E2E. The Desktop contract remains a separate
actual-window gate because compilation and headless interaction do not prove
visual acceptance. A pending contract is a visible failure state, not a fake
passing test.

Inspect every contract:

```bash
./scripts/verify-module --list
./scripts/verify-module runtime
```

Run one implemented module:

```bash
./scripts/verify-module --run testkit
```

Run the engineering gate:

```bash
./scripts/verify-all --gate hermetic
```

The other explicit gates are:

```bash
LANKIDE_E2E_LIVE=1 ./scripts/verify-all --gate live-provider
LANKIDE_E2E_LIVE=1 ./scripts/verify-all --gate live-harness
./scripts/verify-all --gate desktop-visual
LANKIDE_E2E_LIVE=1 ./scripts/verify-all --gate full-release
```

`just check` verifies open-source and production boundaries, formatting, the
workspace build, module contracts, and the Desktop UI boundary. `just test`
(`verify-all --gate hermetic`) runs all workspace integration scenarios once with the Computer and Harness
`test-support` features. Each crate has one `tests/e2e/main.rs` integration
target, with scenario modules running on parallel test threads. The gate builds
Cargo's integration targets once, then runs up to three independent crate suites
together, keeping each suite's output and exit status. Empty source
unit-test harnesses are disabled.
Ignored live tests stay opt-in. The daemon self-spawning helpers are non-shipping Cargo examples, built with
the integration suites in the same Cargo invocation; the prerequisite probe and CLI forwarding examples are already
covered by integration scenarios. The system rig builds the shipped CLI
entrypoint as its test-only `system-cli` binary, without a nested Cargo build.
Focused module commands remain available
through `verify-module --run <id>`. A pending hermetic contract fails the gate.

`just release-local` additionally builds and verifies the release package and
runs `verify-all --gate release-regressions`: real Desktop process bootstrap
and verification-entrypoint scenarios, using the release binaries. Full release
runs these checks against its isolated release build too. Live gates require
explicit opt-in and probe prerequisites before their first external side effect.
No gate turns missing credentials into a green run.

## Required fields for every module

Before a module is handed to another Agent, its row in
`tests/e2e-modules.tsv` must state:

| Field | Required meaning |
| --- | --- |
| `command` | One CLI command that starts the module-level E2E test. It must not require page operation, except the desktop/UI visual contract. |
| `scenario` | The user-visible business behavior being proven. |
| `prerequisites` | Exact credentials, binaries, isolated accounts, fixtures, or files the user must supply. `None` is explicit. |
| `classification` | `hermetic` for deterministic local fixtures or `live` for real IM/Harness/model integrations. |
| `unattended` | Whether the command can run without an operator after prerequisites are installed. |
| `status` | `implemented` only when the command runs a real business E2E; otherwise `pending`. |

Hermetic tests must not touch a user's real Workspace, home, Provider account,
or model session. Live tests must fail before an external side effect when a
listed prerequisite is absent, and must use an isolated test account. The
test command must be safe to repeat in an unattended development/verification
loop.

## Delivery gate for a module

A module is not complete merely because its crate compiles. Its delivery must
include:

1. A public module API that can be entered from a CLI test.
2. A real scenario in `tests/<scenario>_e2e.rs`, included by that crate's
   `tests/e2e/main.rs` (or the documented equivalent for an application process).
3. A manifest row with all fields above.
4. A command that proves the scenario and exits non-zero on a business failure.
5. A report of external prerequisites and whether the test is unattended.
6. No `#[cfg(test)]` source module and no placeholder assertion.

The GPUI application is the one exception to the CLI-only rule: its visual and
interaction acceptance may use a live macOS window. Its data contract still
comes from `lankide-client`, so the test support and Runtime client can be
verified independently.

Hermetic system rigs configure `runtime.json` with `remote_access_port: 0`.
Automatic remote access still detects the LAN address, but the OS assigns its
port while the listener stays bound. Product homes default to port 7443. This
keeps repeated and concurrent scenarios independent of local product instances.
