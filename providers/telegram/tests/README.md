Run the separate S2.2 and S2.3 business scenarios:

```sh
cargo test -p meerkat-provider-telegram --test mock_server_e2e
cargo test -p meerkat-provider-telegram --test mock_server_e2e a_group_message_mentioning_a_bot_reaches_its_agent -- --exact
cargo test -p meerkat-provider-telegram --test mock_server_e2e a_reply_with_an_attachment_reaches_the_chat_with_the_file -- --exact
```

The test starts a Bot API server on an ephemeral loopback port. It drives
`ProviderHost` with the default Telegram HTTP transport, real polling, temporary
SQLite storage, and the production resource store. No credentials or external
network are needed. The peer supports getMe, getUpdates, sendMessage,
sendDocument, sendPhoto, getFile/file download, profile updates, reactions, and
recall. Unknown requests fail. Server threads and temporary files are owned by
the scenario and cleaned up on exit.

Workspace Provider configuration accepts `api_endpoint`, for example
`{"api_endpoint":"http://127.0.0.1:8081"}` for a local Bot API deployment.
Omitting it keeps `https://api.telegram.org`. This setting covers Manager
verification/onboarding and Agent verification, polling, sends and downloads.
It accepts an HTTP(S) base URL, including a path prefix, without credentials,
query, or fragment.

S2.1 binds two Bot accounts to separate Agents and verifies that direct
messages and reactions identify the receiving Agent.
The live contract remains `provider-telegram` in `tests/e2e-modules.tsv`.

The loopback scenarios also cover permanent-event skips, temporary IM outages,
malformed events, and three Agents concurrently sending and reacting in one chat. Assertions use the public Provider host and platform wire
requests, including each sending account.

Event acceptance retries use workspace configuration: `event_retry_initial_ms`
(default 250), `event_retry_max_ms` (default 30000), and
`event_retry_max_attempts` (default 8). Backoff doubles until the maximum. An
event that still fails after the last attempt is logged and skipped, so it
never holds back later events.
A permanent error is logged once and skipped; its platform acknowledgement
lets later events flow. Telegram honors platform `retry_after` values.
Polling errors use capped backoff until recovery or shutdown; permanent polling
errors stop the receiver.
