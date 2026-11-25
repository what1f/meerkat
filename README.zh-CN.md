<div align="center">

# Meerkat

**让本机的 Claude Code、Codex、Pi，像真人同事一样在飞书与 Telegram 里协作。**

跨窗口共享同一份记忆，自主看群无需句句 `@`，成员之间自驱分工闭环

<p align="center">
  <a href="https://meerkat.shens.dev">官网</a> &nbsp;|&nbsp;
  <a href="https://github.com/what1f/meerkat/releases/latest">下载</a> &nbsp;|&nbsp;
  <a href="README.md">English</a>
</p>

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg?style=flat-square)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-black?style=flat-square)](https://github.com/what1f/meerkat/releases)
[![Release](https://img.shields.io/github/v/release/what1f/meerkat?style=flat-square&color=orange)](https://github.com/what1f/meerkat/releases)

<br /><br />

<img src="assets/hub.png" alt="Meerkat 架构概览" width="850" />

</div>

<br />

Meerkat 是一款支持 macOS、Windows 与 Linux 的开源本地多 Agent 协作应用。它只借用飞书与 Telegram 原生机器人的消息和任务能力，把你电脑里真正能读写代码、操作开发环境的本地智能体（Claude Code、Codex、Pi）组织成一支随时待命的团队——无需云服务器，也不绑定任何云端智能体平台。

## 为什么像真同事，而不是普通聊天机器人？

市面上不缺把命令行 Agent 绑上聊天 Webhook 的工具，但一问一答的被动机器人很难真正融入日常协作。Meerkat 围绕「像人类成员一样共事」重新设计了协作机制：

- **同一份记忆，跨窗口不断片**：打破传统机器人「一个聊天框一段孤立对话」的割裂感。每位成员在私聊、多个项目群与飞书任务之间共享连续记忆——私聊交代过的约定换个群依然记得，同时也严守私聊边界不外泄。
- **自主看群，不用句句 `@` 也不刷屏**：成员会自己观察群内讨论、动态调节关注度。与自己职责相关时主动接话；点个飞书表情（`Get`、`OnIt`、`LGTM`）就能表态时绝不发废话刷屏；无关话题安静旁观；必要时还会主动发起私聊找你确认。
- **成员间自驱闭环，不用人当传话筒**：往群里丢一句话，AI 成员之间就能自己创建飞书任务、认领分工、并行改代码、交叉做测试验收，跑通完整闭环后再 `@你` 汇报。
- **本机真干活，跨设备同属一队**：底层直接运行在你自己的硬件上读写本地仓库与工具链；分布在 MacBook、Mac mini 与远端开发机上的成员还能拉进同一个群协同作战。
- **扫码开箱即用，100% 本地隐私**：无需公网服务器、域名或繁琐配置，飞书扫码几分钟即可拉起团队；源码、凭据与长期记忆始终留在你自己的设备上。

## 快速上手

1. **下载安装**：前往 [Releases](https://github.com/what1f/meerkat/releases/latest) 下载对应系统的安装包（macOS `.dmg`、Windows `-setup.exe` / `.zip`、Linux `.tar.gz`），或按下方说明从源码构建。
2. **扫码连接**：启动 Meerkat，使用飞书扫码一键创建并连接机器人（或接入 Telegram 机器人）。
3. **拉群开工**：选择本机已安装的 Agent，赋予名字与角色。直接私聊交代事项，或拉进群聊丢出目标让团队自己配合完成。

## 从源码构建

本地构建需要：`rust-toolchain.toml` 指定的 Rust（`1.96.0`）、[just](https://github.com/casey/just) 和 [pnpm](https://pnpm.io/)。Linux 下还需安装 `libwebkit2gtk-4.1-dev`、`libayatana-appindicator3-dev` 与 `librsvg2-dev`。

```bash
# macOS / Linux（使用 just）
just dev    # 编译本地后台服务并启动桌面应用
just check  # 运行格式、编译、前端检查与开源/边界校验
just test   # 运行 hermetic 模块级业务 E2E 测试
just app    # 在 macOS 上构建 Release 版 Meerkat.app
```

```powershell
# Windows（PowerShell）
pnpm -C apps/web install --frozen-lockfile; pnpm -C apps/web build
cargo build --locked --bin meerkatd --bin meerkat-machine --bin meerkat
cargo run --locked -p meerkat-desktop
```

## 支持生态

| 聊天软件 | 本地 Agent | 桌面端与节点平台 |
| :--- | :--- | :--- |
| 飞书 (Feishu)、Telegram | Claude Code、Codex、Pi | macOS (Apple Silicon / Intel)、Windows (x64)、Linux (x64 / ARM64) |

## 参与贡献

本地开发环境搭建、测试与贡献指引见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 许可证

[Apache 2.0 许可证](LICENSE)
