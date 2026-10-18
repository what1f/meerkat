# Remote Machine refactor plan

Status: Tasks 0–17 implemented; see "Open items" at the end. Design rationale
lives in `docs/remote-machine.md`.

Historical compatibility code has been removed. Storage uses one version-1
baseline; Agents use registered Machines, and unequal protocol versions are rejected.

This plan splits the refactor into tasks that one implementer can finish and
that a reviewer can accept by running business scenarios, without reading the
code.

## Rules for every task

1. **No unit tests of code execution logic.** Do not add `#[test]` functions
   that exercise private helpers, parsers, or branches. Do not add
   `#[cfg(test)]` modules under `src/`. Every new test is a business scenario
   entered through a public module API, a daemon binary, or the system rig.
2. **Mock only outer boundaries.** Allowed: the native Harness process
   (scripted Harness), the IM platform (scripted Provider or a mock IM HTTP
   server), the network (a test TCP proxy), and the clock (configurable
   durations). Everything between them runs for real: storage, runtime,
   machine, protocol, daemons.
3. **Configuration, not test hooks.** Durations such as retry backoff, idle
   timeout, and heartbeat interval become real configuration with production
   defaults. Tests set small values. Production code never checks whether it
   runs under test.
4. **Production stays clean.** Test support lives under `tests/` or in a
   test-only crate listed in `scripts/verify-production-boundary`.
5. **Gates.** `just check` and `just test` pass at the end of every task. Each
   task updates the scenario text in `tests/e2e-modules.tsv` for the modules it
   changes.
6. **Scope.** Touch only the crates listed for the task. If a task needs a
   contract change in another crate, make the smallest change and say so in the
   task report.
7. **Report.** End with: changed files, new or changed scenarios and the exact
   command that runs each, and anything left undone.

## Module boundaries

The refactor keeps the existing crates and makes these seams explicit.

| Seam | Owner | Consumers | Contract |
|---|---|---|---|
| IM boundary | `lankide-provider` | runtime, providers | `ProviderEvent` carries the receiving Agent for direct messages |
| Harness boundary | `lankide-harness` | machine, harnesses | `HarnessFactory` and `HarnessSession`, plus declared capabilities and a structured `HarnessFailure` |
| Machine link | `lankide-protocol` | runtime, machine | JSON frames over a `MachineLink` (send frame, receive frame, close with reason); WS is one implementation |
| Attempt outcome | `lankide-protocol` | runtime, machine | `AttemptFailure { code, category, retryable, fingerprint, detail }` |
| Execution policy | `lankide-runtime` | runtime only | Retry, pause, and notice decisions keyed by failure category and fingerprint |
| Agent capability | `lankide-machine-core` | cli, MCP bridge, Pi extension | Local proxy socket per attempt; the Agent never sees the Runtime address |
| Management | `lankide-client` | desktop | `ManagementClient` trait; transport is an adapter detail |

Rules across seams:

- The Runtime never branches on a Harness ID. It acts on declared
  capabilities.
- The Machine never interprets Provider data. It sees messages only through
  the assignment snapshot.
- Paths on the Machine never cross the Machine link as values the Runtime
  interprets.

## Shared test rig

Task 0 builds the rig every later task uses. It is a test-only crate at
`tests/system` with package name `lankide-system-e2e`.

It composes, in one test process:

- the real Runtime daemon through `lankided::run_daemon`, with a
  scripted IM Provider registered in `ProviderFactoryRegistry`;
- the real Machine daemon through
  `lankide_machine::run_with_harnesses`, with the scripted Harness
  process from `crates/harness/tests/support/process.rs`;
- `lankide-client` as the management caller, like the Desktop.

Scripted IM Provider:

- Several bot accounts, each bound to one Agent, like Feishu Apps or Telegram
  bots.
- Scenario steps: a person sends a group message, mentions an Agent, or sends a
  direct message to one bot account.
- It records every outbound send, reply, reaction, and recall per account and
  per chat, so scenarios assert what people in each chat would see.

Scripted Harness behaviours, chosen per Agent by the scenario:

- reply with text, optionally after streaming events;
- call a Lankide capability through the Agent capability path;
- fail at spawn or mid-turn with a given error text;
- stay silent without progress (hang);
- keep making progress for a given duration;
- spawn a child process that outlives the parent unless killed.

Scenario API shape, so tests read as business steps:

```rust
let rig = SystemRig::start(RigConfig::default()).await?;
let team = rig.workspace("team").await?;
let bruce = team.agent("bruce").harness(Script::reply("hi")).create().await?;
let clark = team.agent("clark").harness(Script::reply("hey")).create().await?;
team.direct_message(&bruce, "alice", "hello").await?;
rig.settle().await?;
assert_eq!(team.dm_transcript(&bruce, "alice"), ["hello", "hi"]);
assert!(team.dm_transcript(&clark, "alice").is_empty());
assert_eq!(clark.executions().await?.len(), 0);
```

`settle` waits until the Runtime inbox is empty and no execution is running,
with a hard timeout that fails the test.

## Phase 1: fixes that do not depend on the transport

### Task 0: system rig

Crates: new `tests/system`, `Cargo.toml` workspace members,
`scripts/verify-production-boundary`, `tests/e2e-modules.tsv`.

Build the rig described above.

Acceptance scenarios (all must pass, `cargo test -p lankide-system-e2e`):

- S0.1 A person mentions an Agent in a group chat; that Agent replies once in
  the same chat; no other Agent runs.
- S0.2 Two messages arrive while an Agent is running; the Agent replies to the
  first, then runs once more for the queued message.
- S0.3 The Machine daemon restarts during a run; the reply still reaches the
  chat exactly once.

Known defects are written as scenarios in the tasks that fix them, not here.

### Task 1: direct messages reach only the receiving Agent

Crates: `lankide-provider`, `lankide-runtime`, `lankide-storage`,
`providers/feishu`, `providers/telegram`.

Contract:

- `ProviderEvent` gains the receiving Agent for direct messages. Feishu fills
  it from the App that received the event; Telegram from the bot that received
  the update.
- Direct-message chats are keyed by provider chat ID and receiving Agent, so
  one person talking to two bots on Telegram creates two chats.
- A direct message without a receiving Agent is dropped with a warning log.

Acceptance scenarios (system rig):

- S1.1 Alice DMs bruce. Only bruce runs and replies. clark and diana have no
  executions, no inbox items, and no sends.
- S1.2 Alice DMs bruce and clark from the same person ID. Each Agent sees only
  its own conversation in its snapshot; each reply goes to its own chat.
- S1.3 A group message that mentions nobody still follows each Agent's
  attention policy, unchanged.

### Task 2: mock IM servers for the real Providers

Crates: `providers/feishu`, `providers/telegram`, their `tests/`.

Contract:

- Each Provider accepts an API endpoint in its configuration (Telegram's local
  Bot API server, Feishu or Lark private deployment). The default stays the
  public endpoint.
- Each Provider crate gets a hermetic mock server under `tests/` that speaks
  the subset of the platform API the Provider uses: event delivery, send,
  reply, reaction, recall, file upload and download.

Acceptance scenarios (hermetic, per Provider):

- S2.1 A direct message delivered to bot B produces an event whose receiving
  Agent is B's Agent.
- S2.2 A group message mentioning bot B produces a mention of B's Agent.
- S2.3 A reply with an attachment reaches the mock server with the right chat,
  parent message, and file bytes.

`tests/e2e-modules.tsv` gains hermetic rows for both Providers; live rows stay.

### Task 3: one Runtime owner and clean shutdown

Crates: `apps/daemon`, `apps/desktop` (service start only),
`lankide-runtime`.

Observed: logs repeat `runtime_daemon_already_running` and
`runtime_storage_still_shared`, launchd reports exit code 1, and the Desktop
can spawn its own Runtime while launchd also starts one.

Contract:

- Exactly one launcher owns the Runtime and Machine services on a machine. The
  Desktop connects to a running service and never starts a second instance.
- Shutdown closes storage after every holder has released it, including live
  Machine streams and Provider tasks.
- A second instance exits with status 0 and a single log line when another
  healthy instance owns the home, so launchd does not treat it as a crash loop.

Acceptance scenarios (runtime-daemon E2E):

- S3.1 Shut down the Runtime while a Machine is connected and a Provider is
  running. The process exits 0, the log has no `still_shared`, and the next
  start opens the same database.
- S3.2 Start a second Runtime on the same home. It exits 0 without opening the
  database, and the first keeps serving management calls.
- S3.3 Kill the Runtime with SIGKILL and start it again. It recovers the lock
  and serves.

### Task 4: structured attempt failures

Crates: `lankide-harness`, `lankide-machine-core`, `lankide-protocol`,
`lankide-runtime`, `lankide-storage`, `lankide-client`, the three Harness
adapters.

Contract:

- `HarnessFailure { code, category, retryable, detail }` returned by Harness
  adapters. Categories: `configuration`, `authentication`, `rate_limit`,
  `transient`, `agent`, `cancelled`, `timeout`.
- The Machine adds `fingerprint` (hash of code and normalized detail) and
  reports `AttemptFailure` in the assignment result and terminal outcome. It
  logs the failure with execution, attempt, Agent, and Machine IDs.
- The Runtime stores the failure with the attempt. The management API exposes
  it in execution details.

Acceptance scenarios (system rig):

- S4.1 A Harness that fails at spawn with "model not found" ends the attempt
  with category `configuration`; execution details through the client show the
  original text; the Machine log contains it with the execution ID.
- S4.2 A Harness that fails mid-turn with a rate-limit message is categorised
  `rate_limit`.
- S4.3 No failure text reaches any chat.

### Task 5: retry policy and chat notice

Crates: `lankide-runtime`, `lankide-storage`, `lankide-core` (localized notice
text), `lankide-client`.

Contract:

- Policy per category as in `docs/remote-machine.md`, with backoff base and
  maximum as Runtime configuration.
- Paused Agents resume when their configuration changes or the owner resumes
  them through management.
- One localized notice per Agent, chat, and fingerprint per fifteen minutes,
  sent through the Agent's own Provider identity.

Acceptance scenarios (system rig, short backoff):

- S5.1 Configuration failure: one attempt, Agent paused, one notice in the
  chat, later messages queue without running. Fixing the Agent's model resumes
  it and the queued message runs.
- S5.2 Transient failure that succeeds on the third attempt: three attempts,
  one reply, no notice.
- S5.3 Transient failure forever: five attempts, batch marked failed, one
  notice.
- S5.4 The same fingerprint three times in a row pauses the Agent even when
  retryable.
- S5.5 Ten failing messages within fifteen minutes in one chat produce one
  notice.
- S5.6 The notice text contains no path, Harness name, exit code, or failure
  detail.

### Task 6: execution liveness

Crates: `lankide-machine-core`, `lankide-runtime`, `lankide-protocol`,
`apps/machine`.

Contract:

- Idle timeout and total budget per attempt, configured on the Machine, with
  production defaults of ten minutes idle and a total budget chosen in the
  task.
- Any Harness event or tool start or finish counts as progress.
- Timeout and cancel run Harness cancel, wait the grace period, then kill the
  whole process tree.
- A timed-out attempt reports category `timeout`.

Acceptance scenarios (system rig, short timeouts):

- S6.1 A Harness that hangs is stopped after the idle timeout; the Agent then
  runs its next queued message.
- S6.2 A Harness that keeps making progress is not stopped by the idle timeout
  but is stopped by the total budget.
- S6.3 After a timeout, the Harness and its spawned child are gone.
- S6.4 Cancelling an execution from management stops it within the grace
  period plus one second.

### Task 7: Harness native defaults and capability declarations

Crates: `lankide-harness`, the three Harness adapters, `lankide-machine-core`,
`lankide-runtime`.

Contract:

- One shared permission policy in the Harness contract. The default grants full
  access. Each adapter maps it to native flags: Pi `--approve`, Claude Code
  `bypassPermissions`, Codex `never` with `danger-full-access`.
- Native skills, context files, and project settings are enabled. Pi loads
  skills from the Agent home. Remove `--no-skills`, `--no-context-files`,
  `--no-prompt-templates`, and Claude's empty `--setting-sources`.
- Every adapter declares only the capabilities it implements. The Runtime and
  Machine check declarations before steering, resuming, or cancelling. A
  message for a running Agent whose Harness cannot steer waits for the next
  run.

Acceptance scenarios:

- S7.1 (system rig) An Agent whose scripted Harness does not declare steering
  receives a second message during a run; the message runs next, with no steer
  frame sent.
- S7.2 (hermetic per adapter) A fake native CLI placed at the configured CLI
  path records the arguments and environment it was started with; the recorded
  start shows full access and native skills enabled, and a skill directory in
  the Agent home is passed or discoverable.
- S7.3 (live, opt-in) A real Pi Agent with a skill in its home uses that skill
  when asked.

### Task 8: Harness output record retention

Crates: `harnesses/pi`, `lankide-machine-core`.

Contract:

- The raw output record keeps events but not streaming deltas, rotates by
  size, and is deleted after a retention period. Its path is in the attempt's
  diagnostics.

Acceptance scenarios:

- S8.1 (hermetic Pi adapter with a fake Pi CLI) A turn that streams ten
  thousand deltas produces a record under the size limit that still contains
  the turn's final message.
- S8.2 Records older than the retention period are removed when the Machine
  starts.

## Phase 2: WebSocket Machine link

### Task 9: JSON frames and the `MachineLink` seam

Crates: `lankide-protocol`, `lankide-runtime`, `lankide-machine-core`.

Contract:

- Serde JSON frame types for every Machine and Runtime frame, each carrying
  `protocol_version`. Unknown types close the link with a typed reason.
- `MachineLink` abstracts one connection. The Runtime broker and the Machine
  client depend only on it. Implement it over WS (`tokio-tungstenite`) on TCP.
- The Runtime daemon listens on a configured address. The Machine daemon
  connects to a configured URL. Local default: loopback.
- gRPC for the Machine path is removed in this task. Management stays on gRPC
  until Task 16.

Acceptance scenarios: every existing `machine`, `machine-daemon`, and
`runtime-daemon` scenario passes over WS, plus:

- S9.1 A Machine with a newer `protocol_version` is rejected with a reason the
  Desktop can show.
- S9.2 Both daemons on different homes, connected only through loopback WS,
  run S0.1.

### Task 10: delivery guarantees over an unreliable network

Crates: `lankide-machine-core`, `lankide-runtime`, `tests/system`.

Contract:

- Outbox writes use temp file, `fsync`, rename, directory `fsync`.
- Acknowledge events only after commit. Replay from the last acknowledged
  sequence. Duplicate sequences are ignored.
- Two-sided heartbeats; three missed intervals close the link. Jittered
  backoff that resets after one healthy minute.

The rig gains a TCP proxy between Machine and Runtime that can pause, drop,
or cut traffic.

Acceptance scenarios:

- S10.1 Cut the network mid-turn for longer than three heartbeats. Both sides
  notice, the Machine reconnects, and the chat receives the reply exactly
  once.
- S10.2 Pause the network so frames stall without a close. Both sides detect
  it within three heartbeat intervals.
- S10.3 Kill the Machine daemon with SIGKILL after the Harness finished but
  before the acknowledgement. After restart the reply is delivered exactly
  once.
- S10.4 Flap the network ten times in one minute. The Machine's reconnect
  attempts follow the backoff and the run completes.

### Task 11: resources over HTTP

Crates: `lankide-runtime`, `apps/daemon`, `lankide-machine-core`,
`lankide-protocol`.

Contract: upload and download over HTTP with the execution token, `Range` for
resume, size and SHA-256 checks before atomic rename.

Acceptance scenarios:

- S11.1 An image attachment in a chat message is available in the Agent's
  snapshot on the Machine with the right hash.
- S11.2 A download cut halfway by the proxy resumes and completes with the
  right hash.
- S11.3 An expired or foreign execution token is rejected.

## Phase 3: registration

### Task 12: pairing and device credentials

Crates: `lankide-storage`, `lankide-runtime`, `apps/daemon`,
`apps/machine`, `lankide-client`.

Contract:

- `machines` table as in `docs/remote-machine.md`.
- Management issues single-use pairing codes valid for ten minutes.
- `lankide-machine pair <runtime-url> <code>` stores a random
  `machine_id` and device credential in the Machine home, owner-readable
  only. The Runtime stores only the credential hash.
- `hello` authenticates. Unknown or revoked credentials close the link with a
  terminal reason and the Machine stops reconnecting.
- Agent placement uses a registered Machine ID. New Agent forms select the locally paired Machine.

Acceptance scenarios:

- S12.1 Pair a Machine with a fresh code; it appears in the catalog as
  registered and online.
- S12.2 Reusing a code, or using one older than ten minutes, fails.
- S12.3 Stop the Machine; it stays registered and shows offline.
- S12.4 Revoke a Machine; its link closes and it does not reconnect.
- S12.5 A second Machine with a copied home but a revoked credential cannot
  connect.

### Task 13: placement

Crates: `lankide-runtime`, `lankide-storage`, `lankide-machine-core`.

Contract:

- Creating or updating an Agent requires a registered, non-revoked Machine
  whose inventory has the chosen Harness.
- Agents carry a placement generation. Execution tokens bind execution,
  attempt, Machine, and generation.
- Messages for an Agent whose Machine is offline stay queued.

Acceptance scenarios:

- S13.1 Creating an Agent on an unknown Machine or with a Harness the Machine
  lacks fails with a typed error.
- S13.2 With two Machines, each Agent runs only on its own Machine.
- S13.3 A message for an Agent whose Machine is offline runs when the
  Machine comes back.
- S13.4 After revoking a Machine, a late result from it is rejected and no
  reply reaches the chat.

### Task 14: local Machine auto-pairing in the Desktop

Crates: `apps/web`, `apps/desktop`, `lankide-client`.

Contract: on first launch the Desktop pairs its own Machine without showing a
code. The Agent form lists registered Machines with online state. Remote
pairing shows one pinned command to copy.

Acceptance: Web business E2Es (`just web-e2e`) cover the Machine list with an
offline Machine and the address and command pairing flow. D2 covers page visual
acceptance. C2 covers the Desktop shell window; local auto-pairing and helper
lifecycle remain covered by the Desktop bootstrap scenarios.

## Phase 4: Agent capability path

### Task 15: Machine local proxy

Crates: `lankide-machine-core`, `apps/lankide-cli`, `harnesses/claude-code` (MCP
bridge), `harnesses/pi` (extension), `lankide-protocol`.

Contract:

- Each attempt gets a private local socket in its run directory. The Harness
  environment carries that socket path and the execution token, and no Runtime
  address.
- The Machine forwards calls as `capability_request` frames and returns
  `capability_response`.

Acceptance scenarios:

- S15.1 (system rig) A scripted Harness uses `lankide` to send a message with
  an attachment and read chat content; both work with the Runtime reachable
  only over WS.
- S15.2 The Harness environment contains no Runtime address.
- S15.3 A token from a finished attempt is rejected.
- The existing `cli` module scenarios pass through the proxy.

## Phase 5: management API

### Task 16: management HTTP API and event WS

Crates: `lankide-runtime`, `apps/daemon`, `lankide-client`,
`apps/desktop`.

Contract:

- Management operations as JSON over loopback HTTP; one WS for events.
  Validate Host, Origin and JSON content type without an owner credential.
- `lankide-client` keeps its trait; the gRPC adapter is replaced by an HTTP
  adapter. The Desktop changes only its connection setup.

Acceptance: every `client` and `runtime-daemon` management scenario passes over
HTTP; local requests without credentials pass and cross-origin requests fail.

### Task 17: remove gRPC

Crates: all users of `tonic` and `prost`.

Acceptance: `cargo tree` shows no `tonic` or `prost`; `just check`,
`just test`, and Web business E2Es pass. D2 covers page visual acceptance;
C2 covers the Desktop shell window.

## Order and review

Tasks run in order. Tasks 1 and 2, 4 and 6, and 7 and 8 touch separate files
and may run in parallel after Task 0. Each task ends with a review: run the
listed scenarios, read the report, and run `just check` and `just test`.

## Open items

- Page behavior for Tasks 14–17 is verified through Web business E2Es
  (`just web-e2e`); page visual acceptance belongs to D2.
- The Tauri shell window is reviewed in C2 through
  `./apps/desktop/window-e2e.sh`; this entry point covers shell scenarios only.
- Remote Machine access now uses a separate TLS listener with persisted identity
  and certificate pinning; management and local discovery remain on loopback.
- HTTP resource downloads read the whole resource into memory before serving
  a range.
- Steer requests to a Harness without steering support are skipped at
  delivery, so their rows can linger.
