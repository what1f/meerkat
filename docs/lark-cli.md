# lark-cli for Feishu Agents

A Feishu Agent can run the user's own `lark-cli` as its bot or on behalf of a
person. Meerkat never configures lark-cli and never stores credentials on the
Machine. Each call gets short-lived credentials from Runtime through the
existing `meerkat` CLI and Agent socket, then execs the real lark-cli with
lark-cli's environment credential provider.

The user installs lark-cli. If it is not on the Machine's executable path,
nothing in this document is active and the prompt does not mention it.

## Flow

```
Harness shell ─ lark-cli <args>             (shim, first on PATH)
             └ meerkat exec 'exec: feishu lark-cli' -- <real lark-cli> <args>
                 ├ Agent socket ─ Runtime ─ Feishu Provider handler
                 │     returns { environment, arguments } or { message }
                 └ execve(real lark-cli, arguments, inherited env + environment)
```

Secrets travel only inside the `meerkat exec` process and the exec'd lark-cli
environment. They are never printed, logged, written to `run-context.json`, or
added to Harness configuration, so Harness session reuse and config
fingerprints are unaffected.

## 1. Generic wrapped CLI commands (provider, protocol, machine, meerkat CLI)

Provider-agnostic. Feishu is the only user.

- `CliCommandDefinition` and protocol `CliCommand` gain `wraps: String`
  (empty for ordinary commands). A non-empty value names a user executable
  this command wraps, e.g. `"lark-cli"`. Runtime names such a capability
  `exec: <id>` instead of `cli: <id>`.
- Machine (`capability_cli::inject`), for each command with `wraps`:
  - Find the executable on the Machine's user executable path (the login-shell
    PATH already stored in Machine config), skipping Meerkat's own shim
    directory. Not found: skip the command entirely (no shim, no prompt line).
  - Write a shim named after `wraps` into `.machine/bin` under the Machine
    root and put that directory first on PATH, before the existing
    `meerkat` directory entry. Shim content, absolute paths shell-quoted:
    ```sh
    #!/bin/sh
    exec '/…/meerkat' exec 'exec: feishu lark-cli' -- '/…/lark-cli' "$@"
    ```
    Rewrite the file only when its content changes, mode 0755. The content is
    stable for a Machine, so PATH and the session identity stay stable.
  - Prompt line uses the plain executable instead of `meerkat invoke`:
    `- \`lark-cli\`：<description>`.
- `meerkat exec <capability> -- <program> [args…]` (new subcommand in
  `apps/meerkat-cli`):
  - Invokes the capability with `args` as the JSON string array, the same
    request `meerkat invoke` sends.
  - Result `{ "environment": {…}, "arguments": […] }`: exec `program` with
    `arguments`, the inherited environment plus `environment`. Use `execvp`
    semantics on Unix so lark-cli owns stdin/stdout/stderr, signals and the
    exit code.
  - Result `{ "message": "…" }`: print the message to stderr and exit 1.
    Used for refusals and authorization prompts.
  - Never print the result JSON.
- `meerkat invoke` on an `exec:` capability is refused with a usage error, so
  credentials cannot reach the transcript through the generic path.

## 2. Feishu Provider command `feishu lark-cli`

Declared in `cli_commands` for every Feishu Agent, with
`wraps: "lark-cli"`. Description (prompt copy, Chinese like the other lines):

> 飞书命令行。默认以你自己的机器人身份执行；要以某位成员的身份操作，加 `--as user`。群聊里有多位成员发言时，再加 `--person <成员ID>` 指明是谁。首次以成员身份使用，或提示缺少权限时，运行 `lark-cli auth login --scope "<权限>"`，对方会收到授权链接。

The handler receives the lark-cli argument array and the frozen
`AgentRuntimeContext` of the attempt.

### Argument handling

- Remove `--person <id>` / `--person=<id>` before returning `arguments`.
- Read `--as user|bot|auto` / `--as=…`. Missing or `auto` means `bot`. Leave
  `--as` in `arguments`.
- `auth login …` is handled by Meerkat (see Authorization) and never reaches
  lark-cli.
- Refuse with a message: `auth logout`, `config …`, `profile …`. These would
  change the user's own lark-cli setup.
- Everything else passes through unchanged.

### Bot identity

Return the Agent app's tenant token from the Provider's existing per-Agent
client cache (`tenant_token()`, refreshed 60 s before expiry):

```
LARKSUITE_CLI_APP_ID            <agent app_id>
LARKSUITE_CLI_TENANT_ACCESS_TOKEN <tat>
LARKSUITE_CLI_BRAND             feishu | lark   (from the Agent's base URL)
LARKSUITE_CLI_DEFAULT_AS        bot
LARKSUITE_CLI_STRICT_MODE       bot
LARKSUITE_CLI_NO_UPDATE_NOTIFIER 1
LARKSUITE_CLI_NO_SKILLS_NOTIFIER 1
```

Never return `LARKSUITE_CLI_APP_SECRET`.

### User identity

Resolve the person:

- Candidates are the human team members who sent a message in this attempt's
  frozen `unread_snapshot` (trigger and mentioned messages). Senders of steer
  messages that arrive later are not candidates. External members never are.
- `--person` given: must be a candidate.
- No `--person`: exactly one candidate is used; zero or several returns a
  message asking for `--person` and listing the candidates as `@名称[成员ID]`.

Token record, one Provider secret per Agent and person
(`agent:{agent_id}:lark_user:{person_id}`), JSON:
`access_token, refresh_token, expires_at, refresh_expires_at, scopes[]`.

- Valid for 5 more minutes: use it.
- Otherwise refresh through the accounts host `/oauth/v3/token`
  (`grant_type=refresh_token`, app id and secret). Store the rotated pair
  before returning. Serialize refresh per Agent and person inside the
  Provider; refresh tokens are single-use.
- Refresh fails or no record: return a message telling the Agent to run
  `lark-cli auth login --scope "<权限>"` for that person.

Environment as for the bot, with `LARKSUITE_CLI_USER_ACCESS_TOKEN` instead of
the tenant token and `DEFAULT_AS` / `STRICT_MODE` set to `user`.

Secrets are written through the `InitContext` secret writer the Provider
already keeps per workspace; `AgentRuntimeContext` stays read-only.

### Authorization (`lark-cli auth login`)

- Accept `--scope "<a b>"` (space or comma separated) and `--person`. Refuse
  `--domain`, `--recommend` and `--exclude`, or a missing `--scope`, with a
  message asking for `--scope`. Other lark-cli login flags such as `--json`
  or `--no-wait` are ignored.
- Person resolution as above.
- Requested scopes = scopes already in that person's record ∪ `--scope` ∪
  `offline_access`. lark-cli replaces scopes on each login; Meerkat keeps them
  additive.
- Start device authorization (`/oauth/v1/device_authorization`, app id and
  secret, no DPoP). Upstream rejects scopes the app has not enabled: return a
  message naming them, so the Agent can ask the owner to enable them.
- Send the person a private message from the Agent's bot with
  `verification_uri_complete`. Copy, in the same language as the Provider's
  other Agent-facing lines:
  > 我需要以你的身份使用飞书，请点这里授权：<link>
- Poll the token endpoint in a Provider background task until success,
  denial or `expires_in`, honoring `interval` and `slow_down`. On success store
  the record. One pending authorization per Agent and person; a new
  `auth login` cancels the previous one.
- The handler returns immediately with a message for the Agent:
  > 已向 @名称[成员ID] 发送授权链接，对方授权后再试。

The Agent's turn usually ends there; the person's next message starts a new
execution in which the token exists.

## 3. Not in scope

- Bundling, installing or updating lark-cli, or its skills.
- Building a Meerkat-specific Feishu CLI.
- Telegram or other Providers.
- Owner-side UI for authorized people. Revocation is the person's Feishu
  authorization settings; a revoked token fails refresh and the Agent asks
  again.

## 4. E2E scenarios

Use the existing mock Feishu server and scripted Harness. A test lark-cli is a
script under `tests/` that writes its argv and the `LARKSUITE_CLI_*`
environment to a file; it is never a production asset.

1. lark-cli absent: assignment has no shim, the prompt has no lark-cli line,
   and the run succeeds.
2. Bot call: the Agent runs `lark-cli im +messages-send …`; the test binary
   sees the original arguments, the app id, a tenant token, `bot` modes and
   both notifier switches; no app secret; the transcript and Machine logs
   contain no token.
3. User call, one sender, valid token: user token passed, `--person` absent.
4. User call, two senders, no `--person`: exit 1 with both candidates;
   repeated with `--person`: succeeds as that person; with a non-candidate
   `--person`: refused.
5. Expired access token: one refresh request, rotated pair stored, two
   concurrent calls refresh once.
6. `auth login --scope "calendar:calendar:read"` for a person with
   `docx:document:readonly`: device request carries both scopes plus
   `offline_access`, the person receives the link privately, polling stores
   the record, and the next `--as user` call uses it.
7. `auth logout`, `config init`, `profile use`: refused, real lark-cli never
   runs.
8. `meerkat invoke 'exec: feishu lark-cli'`: refused.
9. Session reuse: two executions on a reused Harness session get credentials
   for their own senders.
