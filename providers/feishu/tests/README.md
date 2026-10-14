Run the separate S2.2 and S2.3 business scenarios:

```sh
cargo test -p lankide-provider-feishu --test mock_server_e2e
cargo test -p lankide-provider-feishu --test mock_server_e2e a_group_message_mentioning_a_bot_reaches_its_agent -- --exact
cargo test -p lankide-provider-feishu --test mock_server_e2e a_reply_with_an_attachment_reaches_the_chat_with_the_file -- --exact
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

S2.1 is deliberately deferred. `FeishuServer::deliver(app_id, callback)` already
selects the receiving account. A later scenario can supply a direct-message
callback and assert the receiving-Agent field when that contract exists.
The live contract remains `provider-feishu` in `tests/e2e-modules.tsv`.
