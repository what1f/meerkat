Run the separate S2.2 and S2.3 business scenarios:

```sh
cargo test -p meerkat-provider-feishu --test mock_server_e2e
cargo test -p meerkat-provider-feishu --test mock_server_e2e a_group_message_mentioning_a_bot_reaches_its_agent -- --exact
cargo test -p meerkat-provider-feishu --test mock_server_e2e a_reply_with_an_attachment_reaches_the_chat_with_the_file -- --exact
```

The test starts HTTP and WebSocket servers on ephemeral loopback ports. It
drives `ProviderHost` with the default HTTP transport and native Feishu
WebSocket source, including endpoint discovery, pbbp2 callback frames and
acknowledgements. Storage uses temporary SQLite and the production resource
store. No credentials or external network are needed. The peer supports tenant
authentication, bot/user lookup, send/reply, reaction, recall, file/image upload,
and message-resource download. Unknown requests fail. All server threads and
temporary files belong to the scenario and are cleaned up on exit.

Workspace Provider configuration accepts `api_endpoint`, for example
`{"api_endpoint":"https://open.example.org"}` for a private deployment.
It overrides the REST base URL and the `/callback/ws/endpoint` discovery URL;
the discovery response supplies the WebSocket address. Omitting it preserves
the existing `domain` setting (`feishu`, `lark`, or a custom base URL) and its
public Feishu default. The endpoint accepts an HTTP(S) base URL, including a
path prefix, without credentials, query, or fragment. Existing-App onboarding
and permission inspection use the same deployment. New-App device registration
uses Feishu's separate public registration service.

S2.1 binds two App accounts to separate Agents and verifies that direct
messages and reactions identify the receiving Agent.
The live contract remains `provider-feishu` in `tests/e2e-modules.tsv`.

The loopback scenarios also cover permanent-event skips, temporary IM outages,
malformed events, and three Agents concurrently sending and reacting in one chat. Assertions use the public Provider host and platform wire
requests, including each sending account.

Event acceptance retries use workspace configuration: `event_retry_initial_ms`
(default 250), `event_retry_max_ms` (default 30000), and
`event_retry_max_attempts` (default 8). Backoff doubles until the maximum. An
event that still fails after the last attempt is logged and skipped, so it
never holds back later events.
A permanent error is logged once and skipped; its platform acknowledgement
lets later events flow. Source reconnects use capped backoff until recovery or
shutdown.

New Agents use Feishu's official device registration, followed by the official
permissions confirmation link and published-capability polling. Each creator
joins the team once by union identity; the first creator becomes the lead.
Profiles use the contact API and enterprise checks use the official tenant key.

```sh
cargo test -p meerkat-system-e2e --test e2e provider_management_e2e::
pnpm --dir apps/web exec playwright test e2e/rig.spec.ts e2e/onboarding.spec.ts e2e/members.spec.ts
```

The scenarios replace only external Feishu network, IM and Harness boundaries.
They cover expiry, refusal, administrator approval waiting, cross-enterprise
rejection, profile fallback and repeated creators. No user OAuth is requested.
