<div align="center">

# Meerkat

**Turn Claude Code, Codex, and Pi on your machine into real teammates inside Feishu & Telegram.**

One continuous memory across chats, zero-`@` room awareness, and autonomous agent-to-agent handoffs

<p align="center">
  <a href="https://meerkat.shens.dev/en/">Website</a> &nbsp;|&nbsp;
  <a href="https://github.com/what1f/meerkat/releases/latest">Download</a> &nbsp;|&nbsp;
  <a href="README.zh-CN.md">简体中文</a>
</p>

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg?style=flat-square)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-black?style=flat-square)](https://github.com/what1f/meerkat/releases)
[![Release](https://img.shields.io/github/v/release/what1f/meerkat?style=flat-square&color=orange)](https://github.com/what1f/meerkat/releases)

<br /><br />

<img src="assets/hub.png" alt="Meerkat Overview" width="850" />

</div>

<br />

Meerkat is an open-source desktop app for local multi-agent collaboration across macOS, Windows, and Linux. Using nothing more than standard Feishu and Telegram bots, it brings the local coding agents already installed on your machine—Claude Code, Codex, and Pi—into your daily chat as a cohesive team, with zero cloud servers and zero proprietary agent-platform lock-in.

## Why Teammates—Not Just Another Chat Bot?

Wiring a CLI agent to a chat webhook is easy; working with a stateless, command-only bot is exhausting. Meerkat is built so AI members behave like real colleagues in your workspace:

- **One continuous memory across every window**: Ordinary bots trap context inside isolated chat threads. In Meerkat, each member carries a single continuous identity across 1-on-1 DMs, multiple group channels, and Feishu Tasks—remembering prior decisions everywhere while keeping private DMs strictly private.
- **Reads the room—no constant `@mentions`, no spam**: Agents observe group conversations and tune their own attention. They step in when a discussion touches their work, drop a quick emoji reaction (`Get`, `OnIt`, `LGTM`) when a nod is enough, stay quiet when they have nothing useful to add, and DM you proactively when they need input.
- **Autonomous handoffs without human routing**: Drop a goal in the group without playing middleman. Agents create Feishu Tasks, split ownership, write code and tests in parallel, verify each other's changes, and `@mention` you once everything ships.
- **Real local execution across all your machines**: Agents run directly on your own hardware against local repositories and toolchains—and members across your MacBook, Mac mini, and remote workstations can collaborate side by side in the same group.
- **QR onboarding, 100% local & private**: No public webhooks, servers, or config files to wrangle. Scan a QR code with Feishu to get running in minutes while keeping code, credentials, and memory on your devices.

## Quick Start

1. **Install**: Download the installer or archive for macOS (`.dmg`), Windows (`-setup.exe` / `.zip`), or Linux (`.tar.gz`) from [Releases](https://github.com/what1f/meerkat/releases/latest) (or build from source below).
2. **Connect**: Launch Meerkat and scan the QR code with Feishu to create and link your bot (or connect via Telegram).
3. **Collaborate**: Pick an installed local agent, give them a name and role, then DM them directly or invite your team into a group chat and state what you want built.

## Build from Source

Prerequisites: Rust (`1.96.0`, managed via `rust-toolchain.toml`), [just](https://github.com/casey/just), [pnpm](https://pnpm.io/), and [Zig](https://ziglang.org/download/) (`0.17`, for building the built-in `meerkat-agent` binary on macOS and Linux). On Linux, also install `libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev`, and `librsvg2-dev`.

```bash
# macOS / Linux (with just)
just dev    # build local services + built-in agent and launch the desktop app
just check  # format, build, web lint, open-source & boundary checks
just test   # run hermetic module-level E2E scenarios
just app    # build release Meerkat.app on macOS
```

```powershell
# Windows (PowerShell)
pnpm -C apps/web install --frozen-lockfile; pnpm -C apps/web build
cargo build --locked --bin meerkatd --bin meerkat-machine --bin meerkat
cargo run --locked -p meerkat-desktop
```

## Supported Integrations

| Messaging Apps | Local Agents | Desktop & Machine Platforms |
| :--- | :--- | :--- |
| Feishu (飞书), Telegram | Claude Code, Codex, Pi | macOS (Apple Silicon / Intel), Windows (x64), Linux (x64 / ARM64) |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for local development setup, testing, and guidelines.

## License

[Apache 2.0 License](LICENSE)
