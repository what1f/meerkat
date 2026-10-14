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
- S0.2: `a_message_received_during_a_run_waits_for_the_next_run`
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

During S0.3, Runtime can log `storage_invalid: active execution` until the next
Computer heartbeat. Disconnect removes the scheduler's pending completion and
marks the execution recovering (`crates/runtime/src/computer_broker.rs`), while
`crates/runtime/src/application.rs` starts scheduling again before heartbeat
reconciliation releases that execution. Storage correctly rejects the second
active execution (`crates/storage/src/execution.rs`). This transient log does
not block recovery. Task 0 makes no production change for it.

## Execution liveness (Task 6)

Run S6.1–S6.4 with:

```sh
cargo test -p lankide-system-e2e --test execution_liveness_e2e
```

Each test name begins with `s6_1_`, `s6_2_`, `s6_3_`, or `s6_4_`.
Append that prefix to run one scenario. The scenarios wait for saved events,
terminal management details, chat replies, and process exit with bounded waits.
S6.1 proves the next queued message runs; the failed message follows the existing
Runtime requeue policy. S6.4 cancels through the management `CancelAttempt` RPC.
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
The four scenarios use short limits in isolated homes. Native Harness sessions
run in their own process groups; the Computer captures the group before cancel,
waits the grace period, kills the group, and closes the session before reporting
a timeout or cancellation.
