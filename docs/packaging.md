# Lankide macOS 打包

## 用户交付物

本地版交付一个 `Lankide.app`。DMG 提供 App 和 Applications 快捷方式，用户拖入应用程序目录后打开；ZIP 提供同一份 App。App 内包含管理界面、两个后台进程和一个 Agent 执行期 CLI：

```text
Lankide.app/Contents/
├── MacOS/Lankide
├── Library/LaunchAgents/com.lankide.runtime.plist
├── Library/LaunchAgents/com.lankide.computer.plist
├── Library/LaunchServices/lankide-runtime-daemon
├── Library/LaunchServices/lankide-computer-daemon
├── Resources/bin/lankide
├── Resources/AppIcon.icns
├── Resources/LICENSE
├── Resources/THIRD_PARTY_LICENSES.md
├── Resources/licenses/
└── Info.plist
```

用户只需打开 `Lankide.app`。macOS 通过当前用户的后台项目管理 Runtime 和 Computer；Desktop 确认它们可用后连接管理界面。关闭 Desktop 不会中断消息接收或正在进行的 Agent 工作，Dock 中也不会保留 Lankide 图标。用户可以随时在系统设置的登录项中允许或停用 Lankide 后台运行。

直接运行未打包的开发二进制时，Desktop 临时启动本次运行所需的两个进程，并在退出时清理；这一开发回退不会注册系统后台项目。

`lankide` 是提供给 Agent 的执行期能力 CLI，不是用户需要单独启动的第四个产品进程。

## Just 命令

```bash
just dev             # 构建本地服务并启动 Desktop
just check           # 格式、编译与 E2E 合同同步检查
just test            # 全部 hermetic 模块业务 E2E
just app             # 生成 target/release/Lankide.app
just package         # 生成 App、DMG、ZIP 和 SHA-256 文件
just verify-package  # 打包并验证交付物、签名、安装布局和隔离启动
just release-local   # check + test + verify-package，不访问真实账号
just run             # 打包后打开 App
just full-release    # 包含真实 Provider/Harness 的完整 Release Gate
just release         # Full Release Gate + 最终可分发包
```

打包逻辑位于 `scripts/package-macos`，`Justfile` 只提供稳定入口。开发包默认使用 ad-hoc 签名；设置 `LANKIDE_CODESIGN_IDENTITY` 后会使用指定身份依次签名内部可执行文件和整个 App。

打包需要 macOS、仓库固定的 Rust 工具链和 `just`；生成 DMG 还需要 `uv`。
DMG 使用固定版本 `dmgbuild==1.6.7`，安装窗口不含说明文案。
背景位于 `apps/desktop/assets/dmg/`，提供 640×400 和 1280×800 两种分辨率；
中间的箭头提示拖动方向，两侧留给 App 与 Applications 图标。
当前签名流程不包含 Apple notarization，ad-hoc 包用于本地验证。

## 流程与输出

```text
just package
  → 检查生产边界、记录源码、构建四个 release 二进制
  → 校验源码与二进制、检查架构、组装 App 和许可证
  → 签名内部可执行文件，再签名 App
  → 生成 ZIP 并核对 App 的内容与权限
  → 生成 DMG、只读挂载并核对 App、签名与安装布局
  → 保存 ZIP 和 DMG 的 SHA-256
```

默认输出到 `target/release/`：

```text
Lankide.app
Lankide-<version>-macos-<architecture>.dmg
Lankide-<version>-macos-<architecture>.zip
Lankide-<version>-macos-<architecture>.sha256
build-provenance.json
package-provenance.json
```

版本来自 `apps/desktop/Cargo.toml`，架构来自当前构建机，四个二进制必须匹配。
源码凭据包含打包脚本和许可证；签名后不再修改 App。
校验文件使用相对文件名，可在交付目录执行
`shasum -a 256 -c Lankide-<version>-macos-<architecture>.sha256`。

`CARGO_TARGET_DIR` 可指定构建目录；输出必须仍在仓库 `target/` 内。
完整 Release Gate 使用隔离构建目录，并通过 `LANKIDE_RELEASE_BIN_DIR`、
`LANKIDE_RELEASE_PROVENANCE`、`LANKIDE_PACKAGE_OUTPUT_DIR` 将已验收二进制交给打包脚本，避免重新构建。

`just app` 只生成 App。`just package` 不运行模块测试；交付前使用 `just release-local`。
`just full-release` 和 `just release` 会运行真实 Provider/Harness 和实际窗口验收，
只在用户明确授权、隔离账号及验收条件齐备时执行。
当前 GitHub CI 运行 `just check` 和 `just test`；尚未配置 tag 自动打包或 GitHub Release 发布。

## Package E2E

`scripts/verify-package` 不依赖页面操作。它从打包后的 App 主程序启动一个隔离的临时 Lankide Home，验证：

1. Bundle 中四个可执行文件位置正确；
2. App 与内部可执行文件签名有效；
3. ZIP 完整且没有 App 外置的 daemon 或启动脚本；DMG 中的 App 与同一份签名凭据一致，Applications 快捷方式和安装图标位置正确；两个交付物的 SHA-256 一致；
4. Bundle 包含并签署两个用户级 LaunchAgent 描述；
5. 隔离 smoke 路径只启动 Desktop 即可自动启动 Runtime 和 Computer，并在退出后清理临时 daemon、socket 和实例锁。

结果写入 `target/e2e-artifacts/package/<run_id>/result.json`。

隔离启动验证不注册用户的真实后台项目，也不访问真实 Provider/Harness。
它证明交付包可启动本地服务；实际窗口、系统后台注册及真实消息往返由完整 Release Gate 验收。

## 其他桌面平台

跨平台产品语义保持一致：Desktop 是可退出的管理界面，Runtime 和 Computer 由当前用户的系统后台机制托管，用户可以在系统设置中查看和停用。

- Windows 的发行包应使用 MSIX `windows.startupTask` 启动独立后台可执行文件。若提供非 MSIX 安装包，则使用当前用户的 Task Scheduler 登录触发任务并配置失败重启；不要把依赖用户配置、凭据和 Harness CLI 的 Computer 安装成 LocalSystem 服务。
- Linux 使用 `systemd --user` service，分别声明 Runtime 和 Computer 的启动顺序、失败重启和用户会话生命周期。默认随用户登录运行；只有产品明确需要用户退出登录后仍运行时，才由用户显式启用 linger。

Windows/Linux adapter 尚未加入当前 macOS 交付物；实现时复用同一后台生命周期接口，不改变 Desktop、Runtime 或 Computer 的业务接口。
