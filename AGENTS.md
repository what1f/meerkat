# Repository guidance

- Run Cargo, verification, and packaging commands from this repository root.
- Use the existing workspace and module boundaries in `docs/rust-workspace.md`;
  do not add another Cargo root, production client, protocol model, or data path.
- Keep production packages free of mocks, demo identities, credentials, and test
  assets. Test support belongs under `tests/` or a dedicated non-production crate.
- Verify behavior through each module's public business scenario. The primary
  local gates are `just check` and `just test`.
- Live Provider and Harness tests require explicit opt-in and isolated accounts.
- Keep user-facing copy concise and localized. Do not expose implementation
  details in UI copy or hard-code untranslated strings.
- Do not commit `.env`, local databases, build artifacts, or absolute user paths.
