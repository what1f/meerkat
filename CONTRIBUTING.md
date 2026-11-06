# Contributing

Thank you for helping improve Meerkat.

## Development setup

1. Install `just`. Rust is selected automatically through `rust-toolchain.toml`.
2. Run `just check` to validate formatting, compilation, public-source hygiene,
   production boundaries, and E2E contract metadata.
3. Run `just test` for the hermetic business scenarios.

The default gates must not access real Provider accounts, credentials, user
workspaces, or model sessions.

## Changes

- Keep the dependency direction documented in `docs/rust-workspace.md`.
- Add or update a module-level business scenario for behavior changes.
- Keep test-only data and failure doubles outside production packages.
- Do not commit credentials, local databases, generated bundles, or machine-
  specific paths.
- Keep user-facing copy concise and localized; do not hard-code new UI strings.
- Avoid adding proprietary artwork or dependencies with licenses that change the
  distribution terms without an explicit project decision.

## Pull requests

Describe the user-visible outcome, the affected modules, and the verification
commands you ran. Keep unrelated refactors out of the same pull request.

Live tests are optional for ordinary contributions. When a change requires one,
use an isolated account, opt in explicitly with `MEERKAT_E2E_LIVE=1`, and never
attach credentials or private account identifiers to the pull request.
