# Contributing

Thank you for helping improve Meerkat.

## Development setup

Prerequisites:

- Rust toolchain pinned in `rust-toolchain.toml` (`1.96.0`, selected automatically by `rustup`)
- [just](https://github.com/casey/just)
- [pnpm](https://pnpm.io/) (for `apps/web` and `site/`)
- [uv](https://docs.astral.sh/uv/) (optional, for macOS DMG packaging)

Verification commands:

1. Run `just check` to validate formatting, compilation, public-source hygiene,
   production boundaries, web lint/types, and E2E contract metadata.
2. Run `just test` for the hermetic module-level business scenarios.

The default gates must not access real Provider accounts, credentials, user
workspaces, or model sessions.

## Architecture and changes

- Keep a single Cargo workspace rooted at `Cargo.toml`:
  - `crates/` holds domain vocabulary (`core`), wire contracts (`protocol`),
    interfaces (`provider`, `harness`, `storage`), services (`runtime`,
    `machine-core`, `daemon-runtime`, `client`), and test/asset helpers.
  - `providers/*` and `harnesses/*` implement concrete adapters without
    reversing dependencies back into `runtime` or `machine-core`.
  - `apps/` holds the composition roots (`daemon`, `machine`, `desktop`,
    `meerkat-cli`, and `web`).
- Add or update a module-level business scenario under `tests/` and keep
  `tests/e2e-modules.tsv` updated for behavior changes (do not add `#[cfg(test)]`
  unit tests under `src/`).
- Keep test-only data and failure doubles outside production packages.
- Do not commit credentials, local databases, generated bundles, or
  machine-specific paths.
- Keep user-facing copy concise and localized; do not hard-code new UI strings.
- Avoid adding proprietary artwork or dependencies with licenses that change the
  distribution terms without an explicit project decision.

## Pull requests

Describe the user-visible outcome, the affected modules, and the verification
commands you ran. Keep unrelated refactors out of the same pull request.

Live tests are optional for ordinary contributions. When a change requires one,
use an isolated account, opt in explicitly with `MEERKAT_E2E_LIVE=1`, and never
attach credentials or private account identifiers to the pull request.
