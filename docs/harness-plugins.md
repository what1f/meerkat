# Harness plugins

Runtime keeps the existing Agent configuration and Runtime–Machine protocol.
Machine loads JavaScript adapters in QuickJS through the existing `HarnessFactory`
and `HarnessSession` contracts. The four native adapters contain JavaScript
protocol implementations. The Pi, Codex and Claude Code crates only assemble the
plugin factory; the Built-in Agent crate also stores its account and injects it
into the environment (see `docs/builtin-agent.md`).
Test plugins and scripted peers stay under `tests/`.

## Ownership

- Runtime owns business conversations, Agent configuration, prompts, tools and
  assigned skills. It never evaluates plugin JavaScript.
- Machine owns processes, process groups, private resources, deadlines, event
  ordering, execution ownership and session bindings.
- Plugins implement native CLI arguments, protocol messages, model discovery,
  thinking levels, tool adaptation and recovery decisions.
- `cli_path`, environment and argument arrays keep their existing meanings.
  Plugin `defaults.environment` and `defaults.launch_arguments` seed new Agent
  configuration only; saved user configuration is authoritative.
- `apiVersion` identifies the host contract; package versions describe plugin
  releases; `probe().version` reports the installed CLI version.

## Execution flow

```mermaid
sequenceDiagram
    participant R as Runtime
    participant M as Machine
    participant P as JS plugin
    participant H as Native CLI
    R->>M: Agent config, prompt, tools, session directive
    M->>P: start or resume(request)
    P->>M: prepare private resources; spawn CLI
    M->>H: child process with argv and ENV
    H-->>P: JSON notifications through Machine
    P-->>M: actual native session binding
    P-->>M: normalized output or tool request
    M-->>R: persisted execution event
    R->>M: tool result or control request
    M->>P: toolResult, steer, or cancel
    P->>H: native protocol through Machine
    M->>P: close
    M->>H: reclaim process group
```

Discovery uses separate workers and does not execute a model turn. The existing
Machine inventory carries CLI versions, capabilities, model catalogs and editable
defaults to Runtime. Runtime does not depend on plugin source or QuickJS.

## Isolation

Keep OS HOME and system tool configuration. Derive stable native state homes
inside the Machine-owned Agent home. Read the user's native settings as the base,
refresh private configuration before a new process, and apply per-run additions
only to private copies. Never install managed MCP servers, skills or instructions
in the user's global directories. Preserve user resources as inputs, including
relative configuration paths. Adapters declare the native path fields that need
rebasing; other settings retain their native values. Authentication is separate from configuration:
native credential files may be shared so native token refresh remains coherent;
session/history/memory directories must not be shared.

Per-run MCP files and generated extensions are resources owned by the host.
Persistent session data survives close; processes and listeners do not.

## Contract

The SDK provides process creation, JSON-line send/receive, bounded command
execution, managed files/configuration, native skill deployment, MCP tool
bridging, normalized event emission and session binding. Request correlation
belongs to the plugin because Pi and Codex use different response formats.
Waiting for an RPC response must continue dispatching unrelated messages.

Hooks: `probe`, `models`, `start`, `resume`, `execute`, `onMessage`,
`onExit`, `steer`, `toolResult`, `cancel`, `close`, `classifyError`, `test`.
Session hooks share plugin-local state. One session can own multiple sequential
process invocations (Claude); opening a session need not spawn a process.

`start` and `resume` return the actual native session binding. A business resume
may start a replacement native session, provided the plugin supplies the required
continuation context. The host emits the opening session binding automatically;
plugins do not need to repeat it. Late `ctx.bindSession(id)` calls emit a new binding
as well. Duplicate bindings within a turn are suppressed. Machine and Runtime
persist the newest binding without accepting events from stale attempts.

Only injected tools produce executable tool requests; native tool observations
must never be executed again. Tool results are idempotent, reject conflicts,
and may be buffered until the native reply channel exists (Pi).

Discovery never sends model prompts. Explicit tests use a plugin `test` hook to
choose the request/model and run inside the caller's deadline with cleanup.
Onboarding does not run an automatic model trial.

## Acceptance

Run the existing hermetic contracts through the plugin host, including the
scripted subprocess mock, isolation/config refresh, model catalogs, tool results,
streaming, resume/rotation, cancellation and child cleanup. Run `just check` and
`just test`. Only after hermetic success, run explicitly authorized live Pi and
Codex scenarios in managed temporary directories. Live Claude Code requires
separate authorization. The `plugin_live_e2e::dcc_isolated` acceptance uses a
DeepSeek-only wrapper with a clean environment, temporary HOME/configuration,
`--bare` (no OAuth/keychain reads), and `--strict-mcp-config`. It must not inherit
the user's Claude credentials or settings. The wrapper uses the DeepSeek
credential from `dcc` as an explicit API key because bare mode excludes OAuth
and bearer-token authentication. It verifies injected tools and session resume.

## Installing a local adapter

Bundled adapters are compiled from `harnesses/*/plugin.js`. A local package at
`<Machine home>/harnesses/<package>/plugin.json` can replace an adapter with the
same ID or add another adapter. Invalid packages fail startup with diagnostics.
The package entry must remain inside its directory. Restart Machine to reload.

```json
{"apiVersion":1,"id":"my_cli","name":"My CLI","entry":"plugin.js","cli":"my-cli"}
```

The entry calls `defineHarness({...})`; see `harnesses/codex/plugin.js` for a
complete implementation. `crates/harness/src/plugin/sdk.d.ts` describes the
injected globals. Hooks execute synchronously on a dedicated thread per session;
`receiveUntil` dispatches asynchronous native notifications during RPC waits.
Promises and Node.js imports are not part of this API. CPU loops can be interrupted
when the session closes. Native process groups are reclaimed on close, startup
failure, deadline expiry, or worker drop. Each worker owns a temporary discovery
and diagnostic directory; model discovery uses a private native home too.

The JS entry does not need a static CLI version. Optional package release versions
may be managed by distribution tooling. `probe` reads the installed CLI version.
Discovery must not send a model prompt; `test` is a separate, explicitly invoked
hook that returns the selected request and input. Default ENV/args are sent through
Machine inventory to the existing settings editor and saved with a new Agent.
Clearing either field leaves it empty at launch.
