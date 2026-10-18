# Remote Machine design

Status: implemented.

Many Machines register with one Runtime, which can run locally or in the
cloud. Machine links use WebSocket; management and resources use HTTP.

## Goals

- A Machine starts on its own, pairs once, and reconnects to a Runtime over
  the network.
- An Agent can only be placed on a registered Machine.
- Every path from an Agent back to the Runtime goes through its Machine.
- Executions cannot hang forever, errors keep their cause, and retries are
  bounded.
- The local single-machine setup keeps working with the same protocol.

## Non-goals for this version

- Agent files (memory, skills, work files, native Harness sessions) stay on the
  Machine that owns the Agent. The Runtime does not back them up or sync them.
  Moving Agent files between Machines requires an explicit transfer.
- Mounting Agent or Workspace files into a Harness through FUSE. Agents on a
  Machine read local files: chat content arrives in the assignment snapshot,
  attachments are downloaded into the Agent home, and further content is
  fetched through the Machine's capability path. A FUSE view is only needed
  for Agents in diskless cloud sandboxes or for files shared across Machines,
  and would be added as another Machine-side file provider.
- Multiple Runtime instances behind a load balancer.
- LAN browser management. The Web UI is served by the Runtime on loopback;
  remote Machines use their separate pinned TLS entry point.

## Topology

```text
Tauri / Browser ──HTTP JSON + WS events──▶ Runtime ◀──WS (outbound from Machine)── Machine
                                     │                                          │
                                  storage                       local proxy socket
                                  resources                                     │
                                  IM Providers                     lankide-cli / MCP / extension
                                                                                │
                                                                         Harness process
```

The Machine always dials the Runtime. The Runtime never connects to a
Machine, so Machines behind NAT or a firewall work without extra setup.

## Entry points

The Runtime exposes three separate entry points with distinct access rules.
Machine links and resources still require their own credentials.

| Entry point | Caller | Transport | Access control |
|---|---|---|---|
| Management | Tauri / local browser / client | HTTP JSON requests, one WS for events | Local Host, Origin, and JSON checks |
| Machine | machine-daemon | One long-lived WS per Machine | Device credential from pairing |
| Resources | Machine | HTTP upload and download | Execution token |

Agent capabilities (send message, read content, invoke capability, upload and
download) are not a public entry point. They travel as frames on the Machine
WS, so they inherit the Machine's authentication and the Runtime checks the
execution token inside each frame.

## Machine wire protocol

The current `protocol_version` is 2. Capability payloads contain UTF-8 JSON
bytes. Any unequal protocol version is rejected during hello.

Frames are JSON objects with a `type` field, sent as WS text frames. The frame
set keeps the current semantics of `MachineFrame` and `RuntimeFrame`:

- Machine to Runtime: `hello`, `heartbeat`, `assignment_result`,
  `execution_event`, `delivery_result`, `capability_request`,
  `inventory_changed`, `delete_agent_files_result`.
- Runtime to Machine: `hello`, `assign_execution`, `deliver_steer`,
  `cancel_execution`, `capability_response`,
  `execution_event_ack`, `execution_committed`, `shutdown`,
  `delete_agent_files`.

Every frame carries `protocol_version`. Unknown frame types are rejected with a
typed error, not ignored.

Delivery rules:

- Execution events carry a per-attempt `sequence`. The Runtime sends
  `execution_event_ack` only after the events are committed to storage.
- The Machine keeps unacknowledged events and terminal outcomes in a durable
  outbox. Writes use temp file, `fsync`, rename, and `fsync` of the directory.
- After reconnecting, the Machine replays its outbox from the last
  acknowledged sequence. The Runtime ignores sequences it already stored.
- Both sides send heartbeats. A side that receives nothing for three heartbeat
  intervals closes the socket.
- Reconnect uses exponential backoff with jitter. The backoff resets only after
  the connection has stayed healthy for one minute.

## Registration and identity

1. The Desktop asks the Runtime for a pairing code. Codes are single-use and
   expire after ten minutes.
2. The Machine runs `lankide-machine pair <wss-url> <code> --cert-sha256 <hex>`. It stays in the foreground after pairing. On
   successful pairing the Runtime generates a random `machine_id`. The Machine
   stores it with the device
   credential in its home directory, readable only by its owner.
3. The Runtime stores the Machine in a `machines` table: `machine_id`,
   display name, credential hash, `paired_at`, `revoked_at`, `last_seen_at`,
   reported OS, version, and Harness inventory.
4. `hello` authenticates with the device credential. A revoked or unknown
   credential closes the socket with a terminal reason, and the Machine stops
   reconnecting.

"Registered" and "online" are separate. A registered Machine stays in the
catalog while offline. Online status comes from the live connection and
`last_seen_at`.

`CreateAgent` and `UpdateAgent` accept only a registered, non-revoked
`machine_id` whose inventory includes the chosen Harness. Messages for an Agent
whose Machine is offline stay queued until it reconnects.

The local setup uses the same flow. On first launch the Desktop pairs its own
Machine automatically, so the user never sees a code.

## Agent capability path

1. For each attempt, the Machine opens a private local socket inside the
   Agent's run directory and passes its path to the Harness.
2. `lankide-cli`, the Claude MCP bridge, and the Pi extension call that socket.
3. The Machine forwards each call as a `capability_request` frame and returns
   the matching `capability_response`.

The private Agent socket uses one request and one response per connection. A
four-byte big-endian length precedes each UTF-8 JSON document. Requests carry
`protocol_version`, `method`, `context`, and a JSON `payload`; responses carry
`Ok` with the result or `Err` with a typed code and message. Resource payloads
are JSON arrays of chunks with base64 bytes. Calls have a thirty-second deadline,
a bounded message length, and a maximum of sixteen concurrent local calls.
The socket directory is owner-only, the socket mode is 0600, and every request
must match the attempt's execution context. Closing an attempt removes its
socket and cancels pending calls. The Machine link keeps chunk offsets and
correlation IDs for large JSON capability payloads.

The Runtime has no UDS listener. Its instance lock owns the home, its discovery
files identify the HTTP listener, and health/readiness checks use local origin-checked
HTTP management calls. Desktop clients keep the same reuse-first startup flow.

The execution token is bound to execution, attempt, Machine, and the Agent's
placement generation. The Runtime rejects calls and late results from an older
generation, so revoking a Machine or moving an Agent invalidates every token
already issued.

## Resources

Upload and download use HTTP. Downloads support `Range` for resume. The
Machine verifies size and SHA-256 before an atomic rename into the Agent home.

## Agent files

- The Runtime owns Agent configuration: persona, role, team rules, tools, and
  Harness settings. Each assignment carries the configuration revision.
- The Machine owns the Agent home: memory, skills, work files, and native
  Harness sessions. The Runtime only asks the Machine to delete them.
- The Agent home path is a Machine detail. It never appears in Runtime
  storage or in protocol fields that the Runtime interprets.

## Execution liveness

Connection liveness and execution progress are tracked separately.

- The Machine renews an attempt's lease only while the Harness makes progress:
  any Harness event, tool start, or tool finish counts.
- Idle timeout: no progress for a configurable period, default 10 minutes,
  cancels the attempt.
- Total budget: a configurable maximum duration per attempt.
- Tool commands that the Machine runs itself have their own deadline.
- Cancellation sends the Harness cancel, waits a short grace period, then kills
  the process tree. The attempt then ends and the Agent can take its next
  message.

## Errors and retries

An attempt failure crosses the Machine-to-Runtime boundary as a structured
error:

```json
{
  "code": "harness_spawn_failed",
  "category": "configuration",
  "retryable": false,
  "fingerprint": "sha256 of code and normalized detail",
  "detail": "original error text, kept for diagnostics"
}
```

Categories: `configuration`, `authentication`, `rate_limit`, `transient`,
`agent`, `cancelled`, `timeout`.

The Runtime applies one policy per category:

- `configuration` and `authentication`: no automatic retry. The Agent is paused
  until its configuration changes.
- `rate_limit` and `transient`: exponential backoff, at most five attempts per
  inbox batch.
- The same fingerprint failing three times in a row pauses the Agent,
  whatever the category.
- When retries stop, the inbox batch is marked failed and the chat gets one
  short, localized notice. Repeated notices for the same Agent, chat, and
  fingerprint are suppressed for fifteen minutes. The notice never contains
  `detail`, paths, Harness names, or exit codes.

`detail` is stored with the attempt and shown in the Desktop.

## Observability

- Runtime and Machine log through `tracing` to rotating files under their
  homes. Each execution span carries `execution_id`, `attempt`, `agent_id`, and
  `machine_id`.
- The raw Harness output record is stored per attempt, records events but not
  streaming deltas, rotates by size, and is deleted after a retention period.
  Its path is reported in the attempt's diagnostics.

## Fixes that do not depend on the transport

These ship first because they cause the problems seen today:

1. Direct messages dispatch only to the Agent whose IM identity received the
   message. Provider events must carry the receiving account, and chats are
   keyed by it.
2. One launcher owns the Runtime on a machine, a second instance exits cleanly
   without touching the database, and shutdown releases storage only after
   every holder is gone.
3. Execution liveness, structured errors, and the retry policy above.
4. Harness native environment defaults: skills, context files, and project
   settings are enabled; permission modes follow one shared policy declared in
   the Harness contract.

## Harness integration

This version adds no new Harness. The Harness contract must stay open to new
adapters, including a future generic ACP adapter: adapters declare their
capabilities (for example whether a running turn accepts steering), and the
Runtime and Machine act on those declarations instead of on Harness IDs.

## Phases

| Phase | Scope | Main crates |
|---|---|---|
| 1 | Transport-independent fixes above | runtime, storage, machine, harnesses, providers |
| 2 | JSON frame types and WS transport for the Machine entry point; HTTP resources | protocol, runtime, machine, machine-daemon |
| 3 | Pairing, `machines` table, device credentials, placement generation, Agent placement validation | runtime, storage, machine-daemon, desktop |
| 4 | Machine local proxy; `lankide-cli`, MCP bridge, and Pi extension use it | machine, lankide-cli, harnesses |
| 5 | Management HTTP API and event WS; remove tonic and prost | client, runtime-daemon, desktop |

Each phase keeps `just check` and `just test` green and updates the module E2E
contracts in `tests/e2e-modules.tsv`.

## Fresh storage and local placement

`crates/storage/src/schema.rs` defines the single version-1 baseline. A fresh
SQLite database receives this schema. Version 1 opens without conversion; every
other non-zero version returns `StorageError::UnsupportedSchemaVersion`.

`agents.machine_id` references `machines.machine_id`; there is no implicit
placement. Management requires a registered, non-revoked Machine with the
selected Harness. The Desktop selects its locally paired Machine for a new
Agent. First start pairs the Machine using the private local pairing secret,
registers a random Machine ID and credential hash, and writes
`machine-credentials.json` in the Machine home with mode 0600. The local
credential uses `local_discovery: true` to rediscover the Runtime after restart.

Local management listens on `127.0.0.1:7440` by default. An occupied port
stops startup; there is no random-port fallback. Hermetic rigs explicitly select
port 0. Local clients read `management.url` from the run directory. There is no
owner credential. All local routes require Host `127.0.0.1:<port>` or
`localhost:<port>`. If Origin is present, it must be `http://127.0.0.1:<port>`
or `http://localhost:<port>`. POST requires `application/json` (optionally
`; charset=utf-8`). Event subscriptions use
`/api/v1/events?workspace=<id>&after=<seq>`; both parameters are optional.
Other OS users on the same host can access management.
`LANKIDE_RUNTIME_RUN` selects the run directory for an externally managed
Desktop connection. Native Agent calls use only their private attempt socket.

Live Telegram scenarios read an owner-only JSON fixture selected by
`LANKIDE_TELEGRAM_E2E_FIXTURE`. Its fields are `provider_id`, `agent_id`,
`account_id`, `username`, `token`, `manager_account_id`, `manager_token`, and
`chat_ids` (an array of authorized chat IDs). The Feishu team scenario reads two
application objects from the owner-only JSON array selected by
`LANKIDE_FEISHU_E2E_APPS`: `app_id`, `app_secret`, `domain`, `bot_open_id`, and
`bot_name`. These fixtures belong to isolated live test accounts.

The Feishu provider fixture root (`LANKIDE_E2E_TEAM_ROOT`) contains
`feishu-app.json` (`app_id`, `app_secret`, `domain`) and `feishu-e2e.json`
(test chat, Agent, task list, and user OAuth metadata). These are scenario
fixtures, separate from application configuration.

## Remote Machine access

Local Machine discovery and management remain on loopback. The Desktop's Add
Machine dialog uses local `get_remote_access` and
`set_remote_access` operations on local management. `set_remote_access` takes
`{ "address": "host:port" }`; a null or empty address disables remote access.
The address is persisted in `remote-access.json`. Changes take effect live.
The remote port binds on all interfaces and serves only Machine pairing and
Machine WebSocket connections, with TLS. It does not expose management or
resource HTTP routes. Secure assignment and Tool-result attachments reuse the
existing Machine resource capability and the same local path and integrity
checks as loopback downloads. They use the existing bounded capability payload;
loopback HTTP downloads keep their range/resume behavior.

On first enable, the Runtime atomically creates `machine-link-identity.json`
with the DER certificate and private key (0600); it reuses this identity across
address changes and restarts. The command contains its SHA-256 certificate pin.
The Machine verifies the pin and TLS handshake signature during pairing and
stores the pin with its credentials (0600). Every reconnect uses the same pin.
A `wss://` destination without a pin uses normal WebPKI validation. A `ws://`
destination is permitted only for loopback. Pairing codes retain 256 random
bits, encoded in 43 URL-safe characters, expire after the configured validity,
and can be used once. The Desktop renews expired commands while the dialog is
open and detects new online registrations by polling the public catalog.

No installer is included. The remote machine must already have
`lankide-machine` on PATH. Its `pair` command stays in the foreground.
