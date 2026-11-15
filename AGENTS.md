# Repository guidance

- Run Cargo, verification, and packaging commands from this repository root.
- Keep a single Cargo root (`Cargo.toml`); do not add another workspace root,
  production client, protocol model, or data path.
- Preserve module boundaries and dependency direction:
  - `crates/core` has no dependency on concrete Providers, Harnesses, storage,
    transport, or UI frameworks.
  - `crates/protocol`, `crates/provider`, `crates/harness`, `crates/storage`,
    `crates/runtime`, `crates/machine-core`, `crates/daemon-runtime`, and
    `crates/client` define shared contracts and core runtime/machine logic.
  - `providers/*` depend only on `core` and `provider`; `harnesses/*` depend
    only on `core` and `harness`.
  - `apps/daemon`, `apps/machine`, `apps/desktop`, and `apps/meerkat-cli` are
    the only composition roots assembling concrete Providers and Harnesses.
- Keep production packages free of mocks, demo identities, credentials, and test
  assets. Test support belongs under `tests/` or a dedicated non-production crate.
- Verify behavior through each module's public business scenario under `tests/`
  and keep `tests/e2e-modules.tsv` synchronized; do not add `#[cfg(test)]`
  modules under `src/`. The primary local gates are `just check` and `just test`.
- Live Provider and Harness tests require explicit opt-in (`MEERKAT_E2E_LIVE=1`)
  and isolated accounts.
- Keep user-facing copy concise and localized. Do not expose implementation
  details in UI copy or hard-code untranslated strings.
- Do not commit `.env`, local databases, build artifacts, or absolute user paths.
