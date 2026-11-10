# Agent relocation

Status: implemented.

Moving an Agent to another Machine (dragging it to another 工区) carries its
home directory along. The native Harness session is not carried; the Agent
starts a fresh session on the new Machine the same way an idle session is
rebuilt today.

## What moves

| Data | Where it lives | Handling |
|---|---|---|
| Profile, soul, chats, inbox, cursors, attention, secrets | Runtime database | Nothing to move |
| `AGENTS.md` | Agent home | Excluded; written from the soul on every run |
| `resources/` | Agent home | Excluded; downloaded again on demand |
| `memory/` and every other file in the Agent home | Agent home | Archived and moved |
| Native session (`~/.claude`, `~/.codex`, Pi) | Harness storage | Not moved; a fresh session is seeded instead |
| `cli_path` | `agents` row | Cleared; the target Machine's default is used |

Symlinks that resolve outside the Agent home are skipped when archiving.

## Flow

```text
drag → pick the AI → wait for the current run → export on source
     → stage on Runtime → import on target → commit → wake on target
     → first session starts → clean source
```

1. **Request.** `update_agent` with a new `machine_id` no longer writes it.
   It inserts one `agent_relocations` row (source, target, state, staged
   archive, error) together with the Harness, model and thinking level the
   Agent runs on the target; the office asks for them in a dialog when a
   member is dropped on another 工区. An Agent has at most one relocation. The
   target must list that Harness in its inventory, otherwise the request is
   rejected.
2. **Drain.** The scheduler does not start executions for a relocating Agent.
   New messages keep landing in the inbox and run after the move. A running
   execution is allowed to finish; it is not cancelled.
3. **Export.** Runtime sends `ExportAgentFiles` to the source Machine. The
   Machine closes the Agent's cached Harness session and packs the Agent home
   into a zip under `.machine/relocations/out/`, answering with its size and
   sha256. Runtime pulls it with `ReadAgentFiles` in 1 MiB chunks into
   `relocations/` next to its database, checks the hash, records the
   relocation as `staged`, and sends `ReleaseAgentFiles`. There is no size cap.
4. **Import.** Runtime pushes the archive to the target with
   `WriteAgentFiles`, one acknowledged chunk at a time. The chunk marked `eof`
   checks size and hash, unpacks into a sibling temporary directory, then
   swaps it in place of the Agent home, replacing anything left from an
   earlier stay.
5. **Commit.** One transaction sets `machine_id` (the existing trigger bumps
   `placement_generation`) and the picked Harness, model and thinking, clears
   `cli_path` and the native session, keeps the previous placement in
   `previous_json`, and moves the row to `arriving`. The staging file is
   removed and scheduling resumes.
6. **Wake.** The same transaction queues a `move` inbox item in the Agent's
   most recent chat. Its trigger reads "你刚从「源」工区搬到了「目标」工区。";
   the system prompt says the 工区 is the computer the Agent runs on, and the
   Agent decides whether to say anything. With no native session the target
   starts a fresh one, seeded from the moved `memory/MEMORY.md` and recovery.
   An Agent with no chat arrives at once.
7. **Arrival.** The row stays `arriving` until the Agent's native session on
   the target is bound. Then the row is deleted and the source home is queued
   in `agent_file_deletions`. If the first run fails or nothing starts within
   three minutes, the row turns `stalled`.

Relocation state survives restarts. When a Machine reconnects, and when
Runtime starts, Runtime resumes every relocation in progress, the same way
pending file deletions are retried. A pending deletion never runs on a Machine
an Agent is moving to.

## User controls

While moving and arriving, the member sits at the target desk with its things
boxed on the floor. Clicking the box offers:

- **Stay put**: sends `machine_id` equal to the source. Before commit this
  removes the row; while arriving it restores `previous_json`, so the Agent
  runs where it was with its untouched home, and the target copy is deleted.
- **Move without files**: offered only while the source Machine is offline.
  It skips the wait, export and import and commits within the request, so a
  dead Machine never traps its members. The run left on the old Machine is
  stopped by the placement change.

A stalled member shows an alert screen and a toast. The box offers **Try
again** (`retry_arrival`, which wakes it once more and restarts the timer) and
**Move back** (the same as Stay put).

A failure before commit marks the row `failed` and scheduling resumes at the
source; the office shows a toast and calls the move off.


## Harness switch

Switching Harness on the same Machine keeps the Agent home in place. The
config fingerprint changes, so the next run already starts a fresh seeded
session. Claude Code runs with `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` so it keeps
no memory outside the Agent home, and `memory/` stays the only long-term
store.

## Agent awareness

The Agent is not asked to tidy up before a move. A pre-move run costs tokens,
delays the move, and can delete files the owner wanted kept. It only learns
about the move afterwards, through the wake turn above.

## Changes by module

- `protocol`: `ExportAgentFiles`, `ReadAgentFiles`, `WriteAgentFiles`,
  `ReleaseAgentFiles` and `AgentFilesResult` link frames; `Agent.relocation`
  with its state, and `UpdateAgentRequest.relocate_without_files` and
  `retry_arrival`.
- `machine-core`: `host/relocation.rs` packs with exclusions, serves and
  collects chunks, and swaps the home in atomically. Exporting or replacing a
  home also forgets the native sessions the Machine remembered for it.
- `storage`: `agent_relocations` table in the baseline schema; relocating
  Agents are skipped by the scheduler and treated as paused when an execution
  is created.
- `runtime`: `relocation.rs` drives each move and watches the arrival;
  `update_agent` requests, retargets, cancels, retries and takes back moves.
- `web`: the AI dialog on drop, the moving and stalled seat in the office, its
  box menu, and the toasts.

## Verification

`placement_e2e::agent_moves_to_another_machine_with_its_home`: an Agent with
`memory/` on Machine A moves to Machine B. B has the files without
`resources/`, the move wakes it on B with a fresh session that includes the
memory index and the 工区 trigger, and A's home is removed once that session
has started.
