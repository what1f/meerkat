# Lankide

[English](README.md)

Lankide 是一款面向 macOS 的本地多 Agent 协作应用。它把 Agent Harness、消息平台、
持久化工作区和原生 GPUI 桌面端整合在同一个产品中。

当前支持：

- 飞书和 Telegram 消息平台；
- Pi、Codex 和兼容 Claude Code 的 Agent Harness；
- macOS 桌面端及当前用户范围内的 Runtime、Computer 服务；
- 供 Agent 在执行期间使用的 `lankide` 能力 CLI。

项目仍在持续开发，首个稳定版本之前配置和兼容性可能发生变化。

## 环境要求

- macOS；
- `rust-toolchain.toml` 固定的 Rust 工具链；
- [just](https://github.com/casey/just)。

本地开发不要求配置真实 Provider 账号或模型 CLI；默认验证不会访问真实账号或模型会话。

## 构建与验证

```bash
just check
just test
```

启动开发版本：

```bash
just dev
```

构建本地 macOS App：

```bash
just app
```

使用 `just package` 生成 DMG、ZIP 和 SHA-256 校验文件；使用 `just release-local`
完成检查、测试与交付包验证。DMG 打包还需要 [uv](https://docs.astral.sh/uv/)。
命令与验收范围见[打包流程](docs/packaging.md)。

真实集成测试必须显式启用，并使用隔离的测试账号。运行前请阅读 `.env.example` 和
[测试契约](docs/rust-workspace.md)。

## 工程结构

仓库只有一个 Cargo workspace。领域模块位于 `crates/`，Provider 和 Harness 适配器
分别位于对应顶层目录，交付入口位于 `apps/`。详见
[Workspace 架构](docs/rust-workspace.md)。

## 贡献与安全

提交 Pull Request 前请阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。安全问题请通过仓库的
私有 Security Advisory 报告，详见 [SECURITY.md](SECURITY.md)。

## 许可证

Lankide 使用 [MIT License](LICENSE)。字体、图标及其他第三方组件的许可见
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md)。
