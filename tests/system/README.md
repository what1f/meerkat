# System business scenarios

This package is test support only. It runs the real Runtime daemon, Computer
daemon, storage, and management client. Only the IM platform and native Harness
process are scripted. Each scenario owns private temporary Runtime and Computer
homes. No live credentials or model are required.

Run from the workspace root:

```sh
cargo test -p lankide-system-e2e
```

The `system` row in `tests/e2e-modules.tsv` also runs these scenarios in
`./scripts/verify-all --gate hermetic` and `just test`.

- S0.1: `mentioning_an_agent_replies_once_in_the_group_without_running_other_agents`
- S0.2 / S7.1: `s7_1_a_message_received_without_steer_waits_for_the_next_run`
- S0.3: `restarting_the_computer_during_a_run_delivers_the_reply_exactly_once`

Pass a scenario name after `--test system_e2e` to run it alone.

`SystemRig::start`, `workspace`, `agent(...).harness(...).create()`,
`group_message`, `mention`, and `direct_message` are business steps. `settle`
checks a read-only, consistent database snapshot for pending, claimed, or
delivered inbox items and nonterminal executions. It fails after the configured
hard deadline. Execution observations include frozen trigger messages.

Each bot account is bound to one Agent. Outbound records keep the account,
chat, operation, content, message identity, native reply parent, and send key.
Transcripts merge incoming messages and outbound sends/replies in time order.
Direct-message injection records the actual receiving bot but uses the platform
person's shared chat ID. It does not add the routing contract planned in Task 1.

Scripts are selected per Agent through normal Harness configuration. Available
behaviours are reply (with a delay and optional streaming), spawn failure,
mid-turn failure, hang, timed progress, child process, and CLI capability calls.
Scenarios can hold a reply with `Script::reply(...).held()` and release it with
`agent.release_run()`; queue and restart steps do not depend on sleep timing.
Only reply, streaming, queueing, and restart are exercised in Task 0. Replies use
the real execution-scoped `lankide-cli` capability API, then emit terminal output
`OK`, as required by the existing completion protocol.

The Computer daemon runs on a separate Tokio executor thread in the test process.
A restart drops that executor and its tasks, then starts the daemon again with
the same Computer home and a new instance ID. It does not send an OS signal or
restart the Runtime. The next heartbeat marks the interrupted attempt lost and
requeues its message.
S0.3 checks that this attempt fails with `computer_lost`, a new execution succeeds,
and only one reply reaches the chat. The default hard deadline is 15 seconds;
the three scenarios normally finish together in about six seconds.

The interrupted implementation returned the chat reply as terminal output. The
Computer rejected it, requested correction, and eventually failed with
`completion_protocol` (`crates/computer/src/host/events.rs`). Runtime then retried
the unread message. This rig bug caused all three settle timeouts.

Disconnect keeps the scheduler's completion waiter and marks the execution
recovering. The Agent remains reserved until completion, lease expiry, or a
heartbeat reporting the attempt absent. A reconnected Computer becomes available
only after its first active-attempt report is reconciled.

## Execution liveness (Task 6)

Run S6.1–S6.4 with:

```sh
cargo test -p lankide-system-e2e --test execution_liveness_e2e
```

Each test name begins with `s6_1_`, `s6_2_`, `s6_3_`, or `s6_4_`.
Append that prefix to run one scenario. The scenarios wait for saved events,
terminal management details, chat replies, and process exit with bounded waits.
S6.1 proves a timeout retries the frozen first batch after configured backoff,
then runs the next queued message without a failure notice. S6.4 cancels through the management `CancelAttempt` RPC.
Silent and child scripts ignore cancel to exercise forced cleanup. Progress
scripts keep emitting events until the total budget expires.

The rig writes the same `computer.json` file read by `DaemonConfig::for_home`
and the production daemon. The file belongs in the Computer home. For example:

```json
{
  "execution_liveness": {
    "idle_timeout": { "secs": 600, "nanos": 0 },
    "total_budget": { "secs": 7200, "nanos": 0 },
    "cancel_grace": { "secs": 2, "nanos": 0 }
  }
}
```

Absent fields use these defaults. Restart the Computer to apply settings.
The liveness scenarios use short limits in isolated homes. Two additional
scenarios remove Cancel and verify timeout and management cancellation close
the native session and kill its child without a native cancel call. Native Harness sessions
run in their own process groups; the Computer captures the group before cancel,
waits the grace period, kills the group, and closes the session before reporting
a timeout or cancellation.

Task 7 also verifies that a second message runs separately when Steer is absent.
The native peer records controls on a reader thread, including steers received
while a reply is held. `Agent::received_steers` observes that boundary journal.
`RigConfig::harness_capabilities` configures the peer's declared inventory; the
additional scenario removes Resume and Cancel, verifies fresh starts, then
deletes a running Agent without sending an unsupported native cancel.

Task 5 retry scenarios are in `tests/retry_policy_e2e.rs`. Run them with:

```sh
cargo test -p lankide-system-e2e --test retry_policy_e2e
```

They set `RuntimeConfig.retry` through the daemon's optional `retry-policy.json`
in each private Runtime home. The file accepts `base_ms`, `max_ms`,
`notice_dedupe_ms`, and `poll_ms`. Production defaults are 30000, 600000,
900000, and 1000 milliseconds. All values must be positive; maximum backoff
must be at least the base. Omitted fields use their production defaults.

A Runtime restart in these scenarios opens the same database and reconnects
management. The graceful shutdown frame also ends the Computer daemon, so the
rig restarts that peer with its existing private home, as a service supervisor
would. Assertions check saved attempt times and deduplicated chat notices.

## Network delivery (Task 10)

```sh
cargo test -p lankide-system-e2e --test network_delivery_e2e
```

Scenario prefixes `s10_1_` through `s10_4_` select each acceptance scenario.
S10.4 keeps one held execution alive through ten short reconnects and verifies
its single successful completion. `offline_past_attempt_lease_fails_the_run`
uses a two-second lease to verify that prolonged absence still fails the run.
`silent_tcp_and_ws_peers_release_runtime_connections` also verifies upgrade and
hello deadlines against silent peers. All waits use observed state and a hard
limit. The network scenarios use `RigConfig::network_proxy`; the reusable
`net_proxy::NetProxy` exposes `pause`, `pause_replies`, `cut`, `restore`,
`attempts` (connection timestamps), `address` (for HTTP or WS callers), and
`active_connections`. A cut closes
existing connections and rejects new ones until restore. A pause stalls bytes
without closing connections. `pause_replies` stalls only Runtime-to-Computer
traffic, so S10.3 can hold acknowledgements after terminal commit.

S10.3 selects the test-only `system-computer` child binary via
`RigConfig::computer_process`. It runs the real daemon with the scripted native
Harness, sends SIGKILL with `Child::kill`, reaps the process, and restarts with
the same private home. It verifies the persisted terminal event before the kill
and the empty outbox after restart. Production does not inspect test state.

The Runtime reads `runtime.json` with a `computer_link` object containing
`heartbeat_interval` (default 5 seconds) and `handshake_timeout` (10 seconds).
Its top-level `attempt_lease_ms` defaults to 15000; `RigConfig::attempt_lease`
sets this duration for scenarios. Computers report active attempts immediately
after each handshake, then at each heartbeat interval.
The Computer reads `computer.json` with a `reconnect` object containing
`initial` (50 milliseconds), `maximum` (5 seconds), `multiplier` (2),
`healthy_duration` (60 seconds), and `handshake_timeout` (10 seconds).
Durations use Serde's `{ "secs": 5, "nanos": 0 }` representation. Absent fields
keep these production defaults. Each reconnect delay has 75%-125% jitter.

Task 12 registration scenarios:

```sh
cargo test -p lankide-system-e2e --test pairing_e2e
```

Prefixes `s12_1_` through `s12_5_` select the five acceptance scenarios.
`local_self_pairing_moves_legacy_agents_and_reuses_credentials` starts without
credentials or a code, discovers the local Runtime, verifies legacy Agent
placement through management, receives a reply, and reuses its credentials
across a Runtime restart. The final scenario rejects the local pairing secret
as a link credential and rejects a credential used with another device ID.

`RigConfig::pairing_validity` sets `runtime.json`'s `pairing_validity_ms`
(default 600000). The expiry scenario waits on the returned expiry timestamp
with a bounded deadline. The Runtime stores code and device credential hashes.
The Computer stores `computer-credentials.json` through a synced temporary file,
atomic rename and directory sync, with mode 0600. Local devices rediscover the
Runtime URL; paired remote devices use the stored URL. Revocation stops the
real Computer daemon with `computer_revoked`, including a child process using a
copied home.

Recovery regression scenarios:

```sh
cargo test -p lankide-system-e2e --test reconnect_recovery_e2e
cargo test -p lankide-system-e2e --test retry_policy_e2e ownership_conflicts
```

The first holds an active native run through four observed disconnect/re-adoption
cycles while new messages arrive for the same Agent. The original batch replies
once; queued messages then run together in a second batch and reply once in their
own chat. Every offline and re-adopted state is observed before the next step.
The second supplies six identical structured ownership conflicts at the native
Harness boundary, then a transient failure and success. Real Runtime policy
keeps the work queued without spending the failure budget or sending a notice.
The Computer module's parallel-run business scenario separately verifies that a
real `agent_busy` rejection has category `conflict` and is retryable.

The recovery file also holds a run offline past its lease, reconnects the same
Computer, and verifies forced cleanup of the expired run, release of its durable
terminal outbox, and exactly one successful queued retry.
