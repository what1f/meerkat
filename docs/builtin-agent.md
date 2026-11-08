# Built-in Agent

The Built-in Agent is a Harness that ships with Meerkat, so a Computer can run
members without the user installing an AI CLI first.

| Item | Value |
|---|---|
| Harness id | `builtin` |
| Display name | Built-in Agent |
| Binary | `meerkat-agent`, built from the fork revision in `tools/builtin-agent/REVISION` |
| Adapter | `harnesses/builtin` (`plugin.js` over ACP plus a Rust wrapper) |

The upstream product name never reaches users: the adapter rewrites it in
messages, and reasons are fixed codes.

## Fork

We build a light fork of upstream: branch `meerkat` of `what1f/fx`
(`SOURCE_URL` in `tools/builtin-agent/build`), four commits on upstream
`main`:

| Patch | Why |
|---|---|
| Reasoning effort on custom (Chat Completions) connections | Upstream rejects effort there and drops any level the model metadata does not list. Meerkat owns the levels, so any `--effort` is sent as `reasoning_effort`; no effort sends no field. |
| Cache and reasoning usage from Chat Completions | `prompt_tokens_details.cached_tokens` (or DeepSeek's `prompt_cache_hit_tokens`), `cache_write_tokens` and `completion_tokens_details.reasoning_tokens` are read. |
| `FX_AUTO_COMPACT_TOKENS` | Compaction starts at a token count, capped by the context window, instead of a share of it. |
| Compaction reserve | Usable input is `window - min(output limit, 20000)`, so a 128k output limit no longer shrinks it to 72k; summary requests ask for at most 20000 output tokens. Normal requests still send the full `max_tokens`. |

On top of the fork revision, the build applies the patches in
`tools/builtin-agent/patches/` (see the README there):

| Patch | Why |
|---|---|
| `disable-response-language-check.patch` | Upstream rejects a reply whose script differs from the prompt's, retries once and then fails the turn with `ResponseLanguageMismatch`, streaming its failure notice as assistant text. A Meerkat prompt is a Chinese envelope plus ids like `channel_id=group_chat`, so an English message made a longer Chinese reply fail. The check is off in every mode. |

## Build and discovery

`tools/builtin-agent/build [debug|release]` fetches the pinned revision, builds
it with Zig 0.17 and copies the binary to `target/<profile>/meerkat-agent`. The
result is cached under `target/builtin-agent/` per revision and patch set. `just dev` runs the
debug build first; `scripts/build-release` runs the release build, and the
app bundle carries it in `Contents/Resources/bin` (`docs/packaging.md`).

The Machine looks for the binary next to its own executable, then in
`Contents/Resources/bin` of the app bundle, then on `PATH`. Without it the
Harness reports Unavailable.

## Home and environment

All members on a Computer share one home at
`<Machine workspaces>/.machine/builtin-agent` (0700). Every process the adapter
starts gets:

```text
FX_HOME=<home>  FX_AUTO_UPGRADE=0  FX_DISABLE_KEYCHAIN=1
FX_NO_OPEN_BROWSER=1  FX_HERDR=0
FX_AUTO_COMPACT_TOKENS=max(window - 33000, window / 2)
```

`window` is `MEERKAT_AGENT_CONTEXT_WINDOW` from the Machine's environment
(default 200000; anything but a positive integer is ignored).

Sessions start with `acp --full-access [--model] [--effort]`, with the cwd set to
the member's working directory. Full access is a process flag and is never saved.

Files in the home (all 0600, written as a temp file and then renamed):

| File | Content |
|---|---|
| `settings.json` | `provider`, `models.<provider>`, and for API keys `providers.custom` (base URL, Chat Completions, bearer from env, and `model_metadata` for the sign-in's model and each model a member starts with: `context_window` = window, `max_output_tokens` = 128000, `supports_tool_use`) |
| `chatgpt-auth.json` | ChatGPT OAuth tokens |
| `api-key` | The API key; passed to the process as `MEERKAT_AGENT_API_KEY` and never written to settings |

Account state in the inventory (`HarnessInfo.account`) comes from these files:
`{method: "chatgpt", label: <email from the token>}`,
`{method: "api_key", label: <base_url host>}`, or `null`. A signed-out member
fails with category `authentication`.

## Capabilities

System prompt (`session/new` `systemPrompt`), streaming, resume
(`session/resume`), steer (`session/prompt` with the steer flag; dropped steers
are sent again as the next prompt), cancel (`session/cancel`) and Meerkat tools
through the local MCP server. Native tools of the agent are not reported as
Meerkat tool calls.

Models are read only when the picker asks:

- ChatGPT: `models --json`; thinking levels from the model list, or else from
  the ACP effort option. The default model is `gpt-6-luna`.
- API key: `GET {base_url}/models`, id plus display name when present. Every
  model offers `disable, low, medium, high, xhigh, max`; `disable` is passed as
  `--effort none` (sent as `reasoning_effort: "none"`), the others verbatim, and
  no level passes no `--effort`.

## Sign-in

Management operations:

| Operation | Request | Response |
|---|---|---|
| `builtin_agent_sign_in` | `{workspace, machine_id, method: "chatgpt" \| "api_key", base_url?, api_key?, model?}` | `{sign_in_id, authorize_url: string \| null}` |
| `builtin_agent_sign_in_status` | `{sign_in_id}` | `{state: "pending" \| "succeeded" \| "failed" \| "expired", reason: string \| null}` |
| `builtin_agent_sign_in_cancel` | `{sign_in_id}` | same as status |
| `builtin_agent_sign_out` | `{workspace, machine_id}` | `{}` |

**ChatGPT.** The Runtime runs `login codex` in a fresh 0700 temp home and returns
the authorize URL. Only one login runs at a time, because the callback port
(1455) is fixed. After 10 minutes the sign-in expires. On success the Runtime
reads the tokens, sends them to the chosen Machine in a `SetHarnessCredential`
link frame and deletes the temp home. The Runtime keeps no copy.

**API key.** The web sends `base_url` and the key (it owns the provider table and
maps the host label back to a provider). The Machine checks
`GET {base_url}/models` with the bearer key (10 s timeout) before saving
anything. `model` is optional; without it the first listed id becomes
`models.custom`. Each member still passes its own `--model`.

**Sign-out** removes the token and key files and the provider settings.

Credentials go only to the Machine named in the request; nothing copies them to
another Computer. Keys never appear in reasons or logs.

Failure reasons: `cancelled`, `sign_in_failed`, `invalid_key` (401/403),
`unreachable` (network error or other non-success), `machine_offline`,
`machine_rejected`. Unknown or pruned sign-ins read as `expired`.

## Tests

- `harnesses/builtin/tests/e2e` drives the real binary against a local
  OpenAI-compatible endpoint: sign-in checks, model metadata, streaming,
  effort (none without a level, `none` for `disable`), `max_tokens` 128000, tool call, steer, resume, cancel
  and sign-out. It skips when the binary is not built.
- `tests/system/tests/builtin_sign_in_e2e.rs` covers the API-key sign-in through
  management HTTP and a real Machine link.

The ChatGPT browser flow has no hermetic test; it needs a real account.
