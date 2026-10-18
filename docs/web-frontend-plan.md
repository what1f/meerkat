# Web 前端与 Tauri 外壳方案

状态：Web 前端与 Tauri 外壳已落地，D1 清理完成；视觉验收由 C2、D2 独立执行。

本方案移除旧原生桌面框架，管理界面改为一套 Web 前端，由 Runtime 通过 HTTP 提供。
Desktop 改为 Tauri 外壳，只负责托管后台服务和打开窗口。业务数据全部走
HTTP JSON + WS，不经过 Tauri IPC。同一套前端也可以直接在本机浏览器中打开。

## 迁移前背景

以下描述迁移开始前的实现和限制，仅用于说明方案的起因；这些问题已由后续任务处理。

- 旧 Desktop 通过 `lankide-client` 用 HTTP 与 WS 访问 Runtime：业务操作是
  `POST /api/v1/<op>`，事件走 `GET /api/v1/events`。
- 管理 DTO 已使用 serde JSON，二进制数据用 base64，不依赖本地路径。
- 管理、Machine link 和资源下载共用手写的 hyper router，并且只允许绑定 loopback。

浏览器接入前存在以下限制：

- 事件 WS 用 `Authorization` 和 `x-lankide-subscription` 请求头传参，浏览器的 `WebSocket` 无法设置请求头。
- 管理 listener 默认绑定 `127.0.0.1:0`，端口每次都变，浏览器没有固定地址可用。
- 没有静态文件服务，没有 Host 和 Origin 校验。
- 枚举在 JSON 中是 i32，前端只能硬编码数字。
- Desktop 通过读取 `machine-credentials.json` 判断本机 Machine，目录接口里没有 `is_local`。
- Harness 列表和思考强度在 UI 中硬编码，没有使用 catalog。
- 工作区和 Agent 靠轮询刷新，只有 Machine 订阅了事件。

只有 Desktop 外壳需要本机能力，包括：用 SMAppService 或 launchctl 托管 `lankided`
和 `lankide-machine`、替换旧版本、窗口材质和标题栏。其余能力都有 Web 原生实现：

- 文件选择：`<input type=file>`
- 复制：`navigator.clipboard`
- 二维码：前端生成
- 外链：交给系统浏览器打开

## 目标拓扑

```text
   Tauri 外壳（Desktop）                    本机浏览器
 ┌───────────────────────────────┐             │
 │ 托管 lankided / lankide-machine│             │
 │ 启动页 → 导航到 Runtime URL   │             │
 │ WKWebView，仅开放窗口操作          │             │
 └──────────────┬────────────────┘             │
                │      HTTP JSON + WS（同源）    │
                ▼                               ▼
   lankided 127.0.0.1:7440
     /                 Web 前端静态资源（内嵌）
     /api/v1/*         管理 RPC
     /api/v1/events    事件 WS
     /resources/* /machines/pair /   Machine 相关（不变）
```

核心决定：**前端内嵌在 Runtime 中，以同源方式提供，不打进 Tauri 包。**

- Tauri 窗口和浏览器打开同一个 URL、同一份构建产物。
- 同源访问，不需要 CORS。
- UI 版本始终与 Runtime 版本一致。
- Tauri 包里只放一个静态启动页，用来显示 loader 和错误状态。

## 访问控制

Runtime 只在本机提供管理服务，这一版不做登录和 Session。

即便如此，有一类攻击不能放着不管：用户浏览器里打开的任意网页都可以向
`127.0.0.1:7440` 发请求。`CreateAgent` 的 `cli_path` 和 `environment_variables`
会在 Machine 上执行，因此不设防就等于任何网页都能在用户的电脑上执行命令。
下面三条规则可以挡住这类攻击，用户不会感知到：

- **Host 校验**：只接受 `127.0.0.1:<port>` 和 `localhost:<port>`。用来防 DNS rebinding，
  也就是外部域名解析到 127.0.0.1 后读取响应。
- **Origin 校验**：如果请求带了 `Origin`，它必须等于自身 origin。所有 `POST /api/v1/*`
  和事件 WS 握手都要检查，用来防 CSRF 和跨站 WebSocket 劫持。非浏览器客户端不带
  `Origin`，可以直接通过。
- **Content-Type 校验**：只接受 `Content-Type: application/json`。跨站请求因此必须先发预检，
  而 Runtime 不响应 CORS，预检会失败。

满足这三条之后，owner Bearer 就不再承担额外的防护：同一个 OS 用户的任何进程，包括 Agent
自己，本来就能读取同一用户的本地凭据。因此这一版**移除 owner 凭据**，
loopback 访问统一按上面三条规则处理。改动涉及 `apps/daemon`、`crates/protocol` 中的
`management.rs` 和 `resources.rs`、`crates/client/src/http.rs`，以及对应的 E2E。
代价是同一台 Mac 上的其他 OS 用户也能访问管理接口。

事件订阅参数改为放在 query 中：`/api/v1/events?workspace=…&after=…`。

局域网浏览器访问不在这一版范围内。局域网里的远程 Machine 继续使用现有的
7443 pinned TLS 入口，这条路径不受影响。一旦允许局域网浏览器访问，就必须加凭据和
TLS，否则同一 Wi‑Fi 下的任何人都能在这台电脑上执行命令。等真正需要时，再做一个
类似配对码的访问密钥，或者直接接入用户体系。

## 固定端口

`machine_link_listen` 的默认值改为 `127.0.0.1:7440`，这样浏览器可以收藏这个地址。
端口被占用时，Runtime 直接报错退出，不回退到随机端口。hermetic 测试继续使用端口 0。
`management.url` 仍然是 Desktop 和 Machine 发现 Runtime 的方式。

## 事件与刷新

事件仍然只是失效提示，不携带业务数据，现有协议可以原样复用。前端的做法如下：

- 每个 `ManagementEventKind` 对应一组 TanStack Query 的 query key，收到事件后对这组 key 调用
  `invalidateQueries`。
- 连接建立和重连后，失效全部 query。这样可以弥补协议没有回放、sequence 只在进程内
  有效的问题，Runtime 不需要新增回放机制。
- 工作区和 Agent 改为事件驱动，不再轮询。Feishu 二维码登录和 onboarding 仍然按
  `poll_interval_seconds` 轮询，因为它们的状态在外部系统，不在 Runtime。

## 协议调整

项目尚未发布，直接按单一基线修改，不做兼容层。

1. 管理 DTO 的枚举改为 serde snake_case 字符串；`ManagementEvent.kind` 等 i32 字段改为
   枚举类型。Machine link 帧不变。
2. `MachineCatalogItem` 增加 `is_local` 字段。
3. 用 `ts-rs` 从 `lankide-protocol` 的管理 DTO 生成 TypeScript 类型，放在 dev feature 下，
   生成结果提交到 `apps/web/src/lib/api/generated/`。`just check` 检查生成结果是否与源码一致。
4. Harness、思考强度、模型选项都从 catalog 读取。

## 前端

### 技术栈

页面范围：成员、团队、设置、Machine 与配对、十来个对话框、onboarding，后续再加一个
IM 式对话界面。选用 React，原因如下：

- 代码主要由 Agent 编写，而项目不写单元测试。框架越熟悉，越少出现能跑但行为不对的代码。
- IM 对话需要的生态都是现成的：消息列表、富文本输入、Markdown 渲染。
- Select、Menu、Popover 有成熟的 headless 组件可用。

为了不把栈搞复杂，依赖限定如下（版本为 2026-10-18 的 npm latest，落地时再确认一次）：

| 用途 | 选择 |
|---|---|
| 框架 | `react` / `react-dom` 19.3，开启 React Compiler（`babel-plugin-react-compiler` 1.0） |
| 构建 | `vite` 8，`@vitejs/plugin-react` 6，TypeScript |
| 样式 | CSS Modules（Vite 内置），token 用 CSS 变量 |
| 服务端状态 | `@tanstack/react-query` 5.104（latest 5.104.3，2026-10-15 发布） |
| 键盘交互复杂的控件 | `@base-ui/react` 1.9，只用于 Select、Menu、Popover；对话框用原生 `<dialog>` |
| 二维码 | `qrcode` |
| 图标 | `lucide-react`，只在 `design/icons.tsx` 中使用 |

不引入以下依赖：

- 路由库：页面切换用一个 page 状态加 `location.hash`，以页面状态维护导航。
- 全局状态库：服务端数据放在 TanStack Query；界面状态放在组件内部，必要时用 Context。
- 动效库：用 CSS transition 加 `@starting-style`。

### 组件与目录

组件清单、props、目录结构、分层依赖和 lint 规则见
[web-frontend-components.md](web-frontend-components.md)。所有实现任务以那份文档为准，
不照搬 `docs/design/system.html` 的 DOM。分层是 `app → features → shell → scene → design`，
`lib/` 只承载数据、i18n、主题和存储。

其他约定：

- 代码放在 `apps/web/`，用 pnpm 管理，Cargo workspace 保持不变。
- i18n：类型化的中英文字典，每个 feature 一个 `messages.ts`，由 `tsc` 检查键名。
  语言和主题存在 `localStorage`，主题默认跟随 `prefers-color-scheme`。
- 边界检查：ESLint 和 stylelint 承担界面边界检查，由 `just check` 执行。
- 测试选择器：统一用 kebab-case 的 `data-testid`，清单见组件文档。
- 静态资源：reaction、图标、头像、figures、字体统一放在 `apps/web/src/assets/`，由 Vite 打包，
  再内嵌进 Runtime。

### 在 Tauri 外壳中

前端只通过 User-Agent 后缀 `LankideDesktop/<version>` 判断自己是否运行在外壳里，
不读 `window.__TAURI__`（`withGlobalTauri: false`）。在外壳中：

- 侧栏顶部为红绿灯留出安全区，并标记为 `data-tauri-drag-region`。
- 背景透明，露出窗口的 vibrancy 材质。

## 服务端静态资源

- 新建 `crates/web-assets`。release 构建用 `rust-embed` 内嵌 `apps/web/dist`，
  debug 构建从磁盘读取。release 构建时如果缺少 dist，直接报错；`cargo check` 不受影响。
- `apps/daemon` 把它作为 fallback handler 注入 router，`lankide-protocol` 不依赖前端产物。
  - 所有非 API 路径回落到 `index.html`。
  - 带 hash 的资源设置长期缓存，`index.html` 设置 `no-cache`。
- 开发时，`just web-dev` 启动 Vite，把 `/api` 和 WS 代理到本地 Runtime。
  浏览器看到的仍然是同源。

## Tauri 外壳（apps/desktop）

- 使用 Tauri v2，作为普通 cargo bin 构建（`tauri-build` + `custom-protocol`），不引入
  tauri-cli 的打包功能。`scripts/package-macos`、签名、DMG、provenance 和 `verify-package`
  继续使用，只替换主程序二进制。
- `background_services.rs` 和 `backend.rs` 不依赖界面框架，保留在 `apps/desktop` 中。
  `--local-bootstrap-smoke` 保留。
- 启动流程：
  1. 窗口加载内嵌的 `boot.html`，显示 meerkat loader。
  2. 后台线程执行现有的 `start_services`。
  3. 成功后，把窗口导航到 `management.url`。
  4. 失败时，导航到 `boot.html#<error-code>`，由启动页显示本地化的错误状态，
     例如“需要在登录项中允许后台运行”。
- 窗口：1240×820，最小 960×660，`titleBarStyle: Overlay`，红绿灯位置 (16,17)，
  vibrancy 由 `window-vibrancy` 实现。
- 启动页仅授予窗口拖拽权限。启动成功后，仅向发现的 Runtime origin、main 窗口动态授予
  拖拽与主题设置权限；不开放业务 IPC。每个外壳进程只授权一个 Runtime origin；重试发现不同
  origin 时停止自有服务并重启外壳，避免动态授权累加。启动页仍通过 URL hash 接收状态。
- 外链：拦截导航，非 Runtime origin 的链接交给系统浏览器打开（`tauri-plugin-opener`）。
- `Info.plist` 增加 `NSAllowsLocalNetworking`，允许 WKWebView 加载 loopback 的 http 地址。
- 菜单：保留“退出 Lankide”，新增“在浏览器中打开”。

## 测试与验收

只写业务 E2E，不写单元测试。

- **Web 业务 E2E**：Playwright（headless Chromium）连接 `tests/system` 拉起的真实
  `lankided` 和 `lankide-machine`，Harness 和 IM 使用现有 test-support。
  它替代旧桌面交互 E2E，以及 `machine_pairing_e2e` 中涉及 UI 的部分。
  场景沿用现有合同：
  - 创建工作区，完成 onboarding
  - 创建和编辑 Agent，暂停和恢复 Agent
  - 上传头像
  - 配对远程 Machine，重命名和移除 Machine
  - 切换语言和主题
  - 事件驱动的界面刷新
- **访问控制 E2E**：放在 system rig 中，验证以下请求被拒绝，同时非浏览器客户端照常可用：
  - 错误 Host
  - 跨站 Origin
  - 非 JSON 的 POST
  - 跨站 WS 握手
- **外壳 E2E**：继续使用 `bootstrap_e2e.py`。smoke 结果额外记录窗口最终导航到的 URL。
- **视觉验收**：标准按 strict visual acceptance。
  - 浏览器：交给 haiku sub agent，用 ego-lite 完成。
  - Tauri 窗口：通过 Herdr 交给 Codex 做 Computer Use。
- `tests/e2e-modules.tsv`：页面合同使用 `web`，`desktop` 一行改为外壳验收。

## 移除清单（D1 已执行）

- 删除旧桌面组件、预览程序、追踪 shim 和旧 UI 素材；不保留兼容层。
- 删除旧框架 workspace 依赖和 Zed patch，更新 Cargo.lock。
- Desktop 保留 Tauri、后台服务、启动页、App 图标和 DMG 素材。
  Objective-C 依赖仍用于 WKWebView 配置、系统语言和后台项目注册。
- 管理面不使用 owner 凭据，只使用 loopback、Host、Origin 和 JSON 校验。
- `just check` 通过 `just web-check` 执行前端 lint；移除旧桌面边界脚本。
- 开源检查不再包含旧框架许可证例外；reaction 检查指向 `apps/web/src/assets/reactions`。
- 文档和 E2E 合同使用 Web 页面与 Tauri 外壳边界；窗口证据入口为 `apps/desktop/window-e2e.sh`。

## 任务

所有任务都在 `refactor/web-frontend` 分支上推进。每个任务使用独立的 git worktree（`../lankide-wt/<任务>`），
分支名为 `web/<任务>`。所有 agent 都在 Herdr pane 中运行，我负责跑 gate、做最终验收和合并。

| 工作 | 执行者 |
|---|---|
| 协议、服务端、数据层、rig、外壳、组件接口和交互行为（A*、B0、B2、B4、C1、D1） | Codex（gpt-6.1-sol，medium） |
| design 和 scene 的外观（B1） | 设计会话 |
| 页面（B3） | Claude。不交给 Codex |
| 界面文案（各 feature 的 `messages.ts`） | 实现者起草，再请 agy 中的 gemini-3.8-flash（low）润色 |
| 代码 review | Codex，按下面的清单进行 |
| 窗口验收（C2） | Codex Computer Use（gpt-6.1-sol，low） |
| 浏览器视觉验收（D2） | haiku sub agent，配合 ego-lite |
| 调研 | Codex（low） |

### 每个任务的规则

1. **只写业务 E2E。** 不写单元测试，不在 `src/` 下加 `#[cfg(test)]`，前端不写组件测试。
   新增测试都是通过公开 API、daemon 二进制、system rig 或 Playwright 进入的业务场景。
2. **只 mock 外部边界。** 允许 mock 的只有 Harness 进程（scripted Harness）、IM（mock IM 服务）、
   网络和时钟。
3. **范围。** 只改任务列出的 crate 和目录。前端只能新增 [组件文档](web-frontend-components.md)
   里列出的文件和组件；需要新组件时停下来报告，由我先改文档。
4. **不搬原型。** `system.html` 只用来参考视觉和交互，不复制 DOM、字符串模板和全局事件处理。
5. **没有兼容层。** 项目尚未发布，协议和存储都按单一基线直接修改。
6. **Gate。** 每个任务结束时，`just check` 和 `just test` 都要通过。前端任务还要通过
   `just web-check`（lint、stylelint、typecheck、build）。任务改了哪个模块，就要同步更新
   `tests/e2e-modules.tsv` 中对应的场景描述。
7. **报告。** 最后列出：改动的文件、新增或修改的场景及运行每个场景的命令，以及没做完的部分。

### Review

实现 pane 完成后，另开一个 Codex pane 做 review。review pane 拿到三样东西：任务原文、
`git diff refactor/web-frontend...web/<任务>` 和下面的清单。review 结果交回实现 pane 修复，
修完由我验收后合并。

通用清单：

- 是否只改了任务范围内的文件，是否新增了规格之外的组件、依赖或数据路径
- 有没有单元测试、test hook，或者生产代码判断自己是否在测试中运行
- 有没有兼容旧格式的代码
- 有没有过度设计和过度防御：用不到的抽象、配置项和泛型，以及不可能发生的错误分支
- 每个验收场景是否都有对应的 E2E，是否真的会因为行为错误而失败

前端追加：

- props 是否与组件文档逐字一致，`design/` 和 `scene/` 是否接受了 `className` 或 `style`
- 有没有从原型整段搬运的 JSX：单个文件超过 200 行、大段静态标记、内联 SVG 路径（图标和插画必须走 `assets/`）
- import 方向是否符合分层表，feature 之间有没有互相引用
- 用户可见文案是否全部经过 `t()`，中英文是否齐全
- 文案规则：不加多余说明，不暴露内部逻辑，状态用状态点而不是文字，不用 `·` 拼接，不随处显示数量
- `data-testid` 是否与组件文档的清单一致
- features 中是否出现订阅外部资源的 `useEffect`，轮询和 WS 是否都放在 `lib/` 中

### 协议与服务端

#### A1：管理 DTO 的枚举和类型生成

范围：`crates/protocol`、`crates/client`、`crates/runtime`、`apps/daemon`、`apps/desktop`（只修改编译所需部分）、
`tests/system`、`justfile`、`scripts/`。

- 管理 DTO 中的枚举改为 serde snake_case 字符串，`ManagementEvent.kind`、onboarding 的 `status`
  等 i32 字段改为枚举类型。Machine link 帧不变。
- `MachineCatalogItem` 增加 `is_local`。
- 用 `ts-rs` 生成 TypeScript 类型，放到 `apps/web/src/lib/api/generated/`。生成命令写成
  `just web-types`；`just check` 检查生成结果是否最新。

验收（system rig，原始 HTTP）：

- A1.1 `list_agents`、`get_machine_catalog`、`list_workspaces` 返回的状态字段都是字符串，
  客户端按字符串写回后，行为保持不变。
- A1.2 本机 Machine 的 `is_local` 为 true；配对进来的远程 Machine 为 false。
- A1.3 修改一个 DTO 字段但不重新生成类型时，`just check` 失败。

#### A2：本机访问控制、固定端口和 query 订阅

范围：`crates/protocol`、`crates/client`、`apps/daemon`、`apps/desktop`（只修改编译所需部分）、`tests/system`、
`crates/protocol/tests`、`crates/client/tests`。

- 按「访问控制」一节移除 owner 凭据，加上 Host、Origin、Content-Type 三项校验。
- 事件 WS 的订阅参数改为放在 query 中。
- 默认监听 `127.0.0.1:7440`；端口被占用时报错退出，hermetic 测试继续使用端口 0。

验收（system rig）：

- A2.1 不带凭据的 `lankide` CLI 和 `lankide-client` 能正常完成创建工作区、创建 Agent 和订阅事件。
- A2.2 以下请求都被拒绝，并且不产生任何副作用：Host 为 `evil.test:<port>`；`Origin: https://evil.test` 的 POST；
  `text/plain` 的 POST；跨站 Origin 的 WS 握手。
- A2.3 同源 Origin 的 POST 和 WS 握手可以通过；通过 `?workspace=…&after=…` 订阅后，能收到该工作区的事件。
- A2.4 7440 被占用时，`lankided` 以非零状态退出，错误信息中包含端口号。

#### A3：内嵌前端

依赖 B0。范围：新建 `crates/web-assets`、`apps/daemon`、`Cargo.toml`、`justfile`、`scripts/build-release`、
`tests/system`。

按「服务端静态资源」一节实现。`just web-dev` 启动 Vite，并把 `/api` 和 WS 代理到本地 Runtime。

验收（system rig）：

- A3.1 `GET /` 和任意非 API 路径（如 `/members`）都返回 `index.html`，并带 `no-cache`；
  带 hash 的资源带 `immutable` 长缓存；`/api/v1/*` 不受 fallback 影响。
- A3.2 缺少 `apps/web/dist` 时，release 构建失败，并提示先构建前端。

#### A4：新设计需要的接口

依赖 A1、A2 合并。契约依据 Codex 的调研（报告见 `/tmp/lankide-web/a4-research-report.md`，内容未入库），
并按「最小可用」做了收窄。成员顺序和团队墙的摆放存在 localStorage，不进 API。
三个子任务都会改 `crates/protocol` 和 `crates/runtime`。A4a 和 A4b 可以并行；A4c 依赖 A4a 引入的启动参数，排在 A4a 之后。

**A4a 成员字段与模型目录**（`crates/protocol`、`crates/storage`、`crates/runtime`、`crates/machine-core`、
`crates/harness`、`harnesses/*`、`tests/system`）

- **形象**：
  - `Agent` 和 `CreateAgentRequest` 增加 `figure: String`，`UpdateAgentRequest` 增加 `figure: Option<String>`。创建时为空则使用默认形象。
  - 内置形象随前端 assets 打包。
  - 自定义形象包：`import_figure_pack { workspace, bytes }` 返回 `FigurePack { id, name, author, scale, frames: { work, sleep, carry, stand } }`，
    `list_figure_packs { workspace }` 返回 `FigurePack[]`。每一帧都是现有的 `ResourceRef`，存入现有的 workspace 资源系统。
  - 包格式见 `docs/design/design-system.md`。拒绝缺帧、路径穿越和超限的包；导入失败不留下任何残余。导入成功后发 workspace 的 catalog 事件。
- **启动参数**：
  - `Agent`、`CreateAgentRequest` 增加 `launch_arguments: Vec<String>`，`UpdateAgentRequest` 增加 `Option<Vec<String>>`。
  - 一路传到 link 和原生的 `HarnessConfig`。adapter 把它们作为独立的 argv 追加，不经过 shell，也不做冲突校验。
  - 启动参数纳入配置指纹，修改后会话失效。
- **实时在忙**：
  - `Agent` 增加 `activity: idle | busy`。busy 表示有处于 assigning、running 或 recovering 状态的执行。
  - 进入和退出 busy 时都发 `AgentUpdated`，必须在长任务开始时就发出。`ListAgents` 批量计算，不逐个查询。
- **按模型的思考档位**：沿用现有的 `ModelCatalogItem.supported_thinking_levels`，把链路接通。
  - Machine 探测后调用 `factory.models`，写入 `HarnessInfo.model_catalog`。
  - Runtime 实现 `get_model_catalog`。
  - `thinking_level` 允许为空：模型没有档位时隐藏拉杆。

**A4b Provider 能力**（`crates/provider`、`providers/feishu`、`providers/telegram`、`crates/runtime`、`crates/storage`、
`crates/protocol`、`tests/system`）

- **管理失效信号**：Provider contract 增加一个窄接口，“这个 workspace 的管理数据变了”，由 Runtime 发出 `WorkspaceUpdated`。人类成员和任务清单都使用它。
- **人类成员工卡**：统一 `list_human_members` / `update_human_position` 管理接口，与 Provider 无关。
  - 每次新建飞书 App 使用一次性内存扫码会话；创建者身份来自 App 自己的 applications/me，姓名头像来自官方通讯录，失败走空资料兜底。
  - 每个创建者入团，只有首个人类成员成为 lead，默认岗位 Team Leader；后续成员岗位默认空，岗位编辑不会改变身份。
  - Telegram 沿用首次配对的负责人规则，资料写入同一人类成员结构。
  - 小卡、大卡无背面和翻转；姓名头像只读，只开放岗位输入，不展示介绍、状态点或引擎模型。
- **任务清单（只做飞书）**：
  - 新增 `feishu.workspace.tasks` interface，只提供读取操作：
    - `list` 返回 `{ enabled, name?, items: [{ id, text, completed, assignee?: { agent_id?, name?, avatar? } }] }`
  - 复用现有的 task client 和 tasklist_guid。启用和停用沿用 `provider_config_json.task_system_enabled`；停用时不删除远程数据。
  - 外部任务回调通过管理失效信号刷新界面。
- **第一条回复**：
  - 新增 `get_agent_first_reply { agent }` 返回 `{ state: waiting | replied }`。
  - replied 的条件是：成员创建之后，在与当前 lead 的私聊中，成功发出了一条非空回复。
  - 满足条件的发送提交之后，发 `AgentUpdated`。

**A4c AI 测试**（`crates/protocol`、`crates/runtime`、`crates/machine-core`、`crates/harness`、`harnesses/*`、`tests/system`）

- 新增管理 op `test_harness { machine_id, harness_id, model, thinking_level, cli_path?, environment_variables, launch_arguments }`，
  返回 `{ state: passed | failed | timed_out, reply?, error_code? }`。
- 这是一个同步请求，服务端总共最多 30 秒。
- Machine link 增加定向的测试帧。Machine 用同一个 factory 在临时隔离目录中跑一次短问候：不带业务工具，不创建成员，不发 IM，结束或超时时清理进程组。
- 结果不持久化，也不发事件。

验收场景写在每个子任务的说明里，全部放在 system rig 中，使用 scripted Harness 和 mock IM。

### 前端

#### B0：`apps/web` 骨架和全部组件接口

范围：`apps/web/`、`justfile`、`scripts/`、`.gitignore`。

- 依赖、Vite、TS、ESLint 和 stylelint 按「技术栈」和组件文档配置，所有 lint 规则都要启用。
- 按组件文档建好完整目录树。`design/`、`scene/`、`shell/` 中的每个组件都要写出**最终的 props 类型**，
  再加一个只有语义结构、不带视觉样式的最小实现，从 index 导出。B1 在这个基础上补外观，B3 直接基于这些接口开发。
- `design/` 控件的交互行为在 B0 中完成：键盘操作、`InlineText` 的提交和撤销、`Combo` 的过滤、toast 计时、
  `Command` 的复制、`Dialog` 的打开和关闭、Base UI 控件的接入。`scene/drag/` 的 hook 只写出签名和空实现，由 B1 完成。
- `lib/` 中只建 `i18n/`、`storage.ts`、`theme.ts`，其余模块属于 B2。`lib/api/generated/` 由 A1 生成。
- `app/`：Provider 组合，以及 hash 路由的空页面。`lib/i18n` 的骨架和 `common.ts`。
- `dev/DesignPreview` 的框架，每组组件先放一个空的 section。
- 把素材迁移到 `src/assets/`。`apps/desktop/assets` 中原有的素材暂不删除，留到 D1 处理。
- 新增 `just web-check`（lint、stylelint、typecheck、build），并接入 `just check`。
- 新增 `apps/web/lint-fixtures/` 和 `scripts/verify-web-lint`：每条规则准备一个反例，确认 lint 会失败。

验收：

- B0.1 `just web-check` 通过；`scripts/verify-web-lint` 中的每个反例都会被对应规则拦下。
- B0.2 `pnpm -C apps/web dev` 后，可以打开 `#/design`；生产构建产物中不包含 `dev/` 的代码。

#### B1：design 和 scene 的外观（由设计会话完成）

依赖 B0。范围：`apps/web/src/design/`、`apps/web/src/scene/`、`apps/web/src/dev/`、`apps/web/src/assets/`。

- 不修改 props。确实需要修改时，先提出来，由我更新组件文档。
- `tokens.css` 从 `system.html` 整理而来。每个组件的全部状态都要在 `#/design` 中预览，并和原型并排对照验收。

#### B2：数据层和外壳

依赖 A1、A2、B0。范围：`apps/web/src/lib/`、`apps/web/src/shell/`（只做行为部分，不改外观）、`apps/web/src/app/`。

- 按组件文档实现 `lib/` 的全部模块。每个资源提供 query 和 mutation hook；`keys.ts` 维护事件 kind 到
  query key 的映射，覆盖 `ManagementEventKind` 的每个值。
- `events.ts`：建立连接后，以及每次重连后，都失效全部 query。
- `agentStatus()`，把成员状态映射到状态点和工位屏幕。
- `AppShell` 和 `Sidebar` 接入导航和团队切换，`useSidebarLayout` 负责持久化。在 Tauri 外壳中运行时，留出红绿灯安全区。

#### B4：Playwright rig

依赖 A2、A3。范围：`tests/system`（新增一个可执行 rig）、`apps/web/e2e/`、`apps/web/playwright.config.ts`、
`justfile`、`scripts/verify-all`、`tests/e2e-modules.tsv`。

- rig 拉起真实的 `lankided` 和 `lankide-machine`，使用 scripted Harness 和 mock IM，向 Playwright 输出 base URL，
  并提供驱动 mock IM 的控制接口，例如“某人在群里 @ 了某个成员”。
- `just web-e2e` 跑全部 spec，并接入 `just test`。

验收：

- B4.1 打开首页，侧栏的三个导航可以切换；通过 rig 创建工作区后，界面在不刷新页面的情况下出现这个工作区（验证事件驱动）。

#### B3：页面

都依赖 B0、B2、B4，用到新字段的部分还依赖 A4。由 Claude 实现，Codex review。四个任务可以并行，各自只改 `features/<名称>/`、
`app/routes.ts` 里自己那一行，以及 `e2e/<名称>.spec.ts`。

- **B3a 成员**：成员板、你自己的工卡、成员详情页（操控台、四列选择器、表情、头像、形象、高级抽屉）、删除。
  - 场景：创建成员并保存；修改名字和模型后刷新页面，修改仍然保留；暂停后恢复；上传头像；删除时需要确认；
    成员开始工作时，状态点和工位都会变化。
- **B3b 团队和设置**：团队墙（名字、规约、任务清单、上下班翻牌，以及摆放位置的持久化）、设置页。
  - 场景：修改规约后保存；下班后成员停止响应，上班后恢复；切换语言和主题后刷新页面，设置仍然保留。
- **B3c 工区**：工区列表、工位、拎起成员换到另一个工区、添加工区（配对命令和连接状态）、重命名、移除。
  - 场景：粘贴配对命令后启动第二台 Machine，新工区出现，状态点变为在线；把成员拖到新工区后，
    它下一次运行就在新 Machine 上；重命名；移除。
- **B3d 引导**：五个步骤、Feishu 二维码的各个状态、Telegram Token、跳过规则。
  - 场景：mock IM 依次完成扫码、选择 AI、创建第一个成员；成员第一次回复后，引导结束并进入成员板。

### 外壳与收尾

#### C1：Tauri 外壳

依赖 A2，导航到真实 UI 需要 A3。范围：`apps/desktop`、`Cargo.toml`、`scripts/package-macos`、`scripts/verify-package`。

按「Tauri 外壳」一节实现。旧桌面主程序直接替换，不保留两套。C1 在第 2 波完成开发，但要等 B3 完成后，与 D1 一起合并。
`boot.html` 由 C1 先实现功能结构，视觉由设计会话补充。

验收：

- C1.1 `--local-bootstrap-smoke` 通过，记录下的最终 URL 等于 `management.url`。
- C1.2 `just verify-package` 通过：签名、DMG 和 provenance 都正常，`Info.plist` 中包含 `NSAllowsLocalNetworking`。
- C1.3 在页面中打开外链时，由系统浏览器打开，窗口本身不跳转。

#### C2：窗口验收

依赖 C1、B3。通过 Herdr 交给 Codex 做 Computer Use：红绿灯的位置、vibrancy、拖拽区域、
启动 loader、错误页、“在浏览器中打开”菜单。

#### D1：移除旧原生桌面框架

依赖 B3、C1。按「移除清单」执行。`tests/e2e-modules.tsv`、`verify-*` 脚本和文档同步更新。
验收标准：`just check`、`just test`、`just verify-package` 全部通过，移除全部旧框架依赖（设计归档文档按任务要求保留）。

#### D2：浏览器视觉验收

依赖 B1、B3。交给 haiku sub agent，用 ego-lite 逐页、逐个状态和 `system.html` 对照，
浅色和深色都要检查，标准按 strict visual acceptance。

### 派发顺序

| 波次 | 任务 | 说明 |
|---|---|---|
| 1 | A1、A2、B0 | B0 不碰 Cargo 代码；A1 和 A2 都改 `crates/protocol`，A1 先合并，A2 再 rebase 到它上面 |
| 2 | A3、A4、B1、B2、B4、C1 | B1 交给设计会话，其余交给 Codex |
| 3 | B3a、B3b、B3c、B3d | B1 合并后，各页面直接获得新外观，不需要改代码 |
| 4 | D1，然后 C2、D2 | |

D1 完成后，只保留 Web 前端和 Tauri 外壳，不设共存期。
