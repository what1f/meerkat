# Lankide

[简体中文](README.zh-CN.md)

Lankide is a local multi-agent collaboration app for macOS. It brings Agent
harnesses, messaging providers, durable workspace state, and a native GPUI
desktop client into one product.

The current integrations are:

- Feishu and Telegram messaging providers;
- Pi, Codex, and Claude Code-compatible Agent harnesses;
- a macOS desktop app with per-user Runtime and Computer services;
- an execution-scoped `lankide` capability CLI for Agents.

Lankide is under active development. Configuration and compatibility may change
before the first stable release.

## Requirements

- macOS;
- the Rust toolchain pinned by `rust-toolchain.toml`;
- [just](https://github.com/casey/just).

Provider accounts and Agent CLIs are optional for local development. The
default verification suite does not access real accounts or model sessions.

## Build and verify

```bash
just check
just test
```

Run the development app:

```bash
just dev
```

Build a local macOS bundle:

```bash
just app
```

Build the DMG, ZIP, and SHA-256 file with `just package`. Run `just release-local`
to check, test, and verify the distribution. DMG packaging also requires
[uv](https://docs.astral.sh/uv/). See [Packaging](docs/packaging.md) for commands
and acceptance limits.

Live integration tests require explicit opt-in and isolated test accounts. See
`.env.example` and [the testing contract](docs/rust-workspace.md) before running
them.

## Workspace

The repository is one Cargo workspace. Domain crates live under `crates/`,
Provider and Harness adapters have their own top-level directories, and shipped
composition roots live under `apps/`. See [Workspace architecture](docs/rust-workspace.md).

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request. Please use
the repository's private security advisory flow for sensitive reports; details
are in [SECURITY.md](SECURITY.md).

## License

Lankide is available under the [MIT License](LICENSE). Bundled fonts, icons, and
other third-party components are covered by their own terms in
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
