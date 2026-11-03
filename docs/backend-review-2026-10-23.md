# 非 UI 业务代码审查记录

- 日期：2026-10-23
- 分支：`main`
- 基准提交：`e5a99d98af55e7b29f52b80e301b6262cf72f82c`
- 审查对象：当前工作区的业务代码，包含 Runtime、Machine、Harness/插件、Provider、Storage、协议与鉴权、CLI、桌面后台服务，以及 Web 目录中的非 UI 调用逻辑。
- 不包含：UI 展示、样式、测试代码。
- 状态：记录问题，尚未实施修复。审查没有修改业务代码，也没有改动原有未提交内容。

代码位置使用仓库相对路径和审查时的行号。后续修改可能使行号变化，可按函数名定位。

## 结果与验证范围

本轮确认 11 个问题：5 项 P1、6 项 P2。P1 建议优先处理；P2 应安排修复。

| 编号 | 优先级 | 问题 | 验证方式 | 状态 |
| --- | --- | --- | --- | --- |
| R01 | P1 | 本地 MCP 工具接口缺少来源和鉴权校验 | 隔离 HTTP 请求，确认进入工具调用队列 | 待修复 |
| R02 | P1 | 第二次运行覆盖 Agent 自有凭据 | 临时目录中连续调用配置刷新逻辑 | 待修复 |
| R03 | P1 | 同名 Telegram Bot 被匹配到错误的 Agent | 调用生产匹配函数，核对凭据写入链路 | 待修复 |
| R04 | P1 | 自定义 Harness 阻止整个 Machine 启动 | 最小 JS 插件调用生产注册 API | 待修复 |
| R05 | P1 | Telegram 团队的成员新增流程必然缺少注册配置 | 核对提交逻辑与 Provider 创建分支 | 待修复 |
| R06 | P2 | 一个 Agent 的轮询确认掉其他 Agent 的注册事件 | 生产 onboarding API 对接隔离更新队列 | 待修复 |
| R07 | P2 | 工作区暂停后，已发送成功的消息仍返回失败 | 临时数据库与隔离 Provider 调用 Runtime API | 待修复 |
| R08 | P2 | Provider 凭据更新后，运行中的客户端继续使用旧值 | 核对管理 API、生命周期与客户端缓存 | 待修复 |
| R09 | P2 | 附件 ID 前缀冲突生成错误路径 | 调用原始资源链接替换函数 | 待修复 |
| R10 | P2 | 多段消息撤回只删除第一段 | 临时数据库验证撤回目标，核对 Provider 链路 | 待修复 |
| R11 | P2 | Claude Code 收不到工具失败的原因 | 隔离 MCP 请求验证返回内容 | 待修复 |

共 9 项完成了针对性的隔离验证，R05、R08 通过调用链确认。隔离验证使用临时数据库、临时目录、最小插件或本地 fixture；部分验证直接编译引用原始生产源码。没有读取真实账号凭据，也没有调用真实 Provider 或 Harness CLI。

相关生产库构建通过：

```sh
cargo build --locked -p meerkat-machine-core -p meerkat-storage -p meerkat-daemon-runtime
cargo build --locked -p meerkat-provider-telegram
cargo build --locked -p meerkat-runtime
```

实际构建通过工作区现有的 `scripts/with-dev-cache` 包装执行。没有审查或运行仓库测试，没有执行 `just check`、`just test`，也没有执行真实浏览器或外部服务验收。构建通过只证明相关生产库能够编译。

## R01：本地 MCP 工具接口缺少来源和鉴权校验

**位置：** `crates/harness/src/plugin_mcp.rs:107–114`，`handle_connection`；关联 `read_http_body`、`harnesses/claude-code/plugin.js`。

**触发条件：** Claude Code 的本地 MCP 服务正在运行，调用方能够访问其监听端口。

服务监听于本机随机端口，但请求处理不校验 `Origin`、`Host`，也不要求会话凭证。收到合法的 `tools/call` JSON 后，请求直接进入当前 Harness 的工具调用队列。

**影响：** 非预期调用方可以借用当前 Agent 的工具权限。只绑定 `127.0.0.1` 不能替代请求来源和会话校验。

**验证：** 隔离请求携带外部 `Origin`、外部 `Host` 和 `Content-Type: text/plain`，仍成功触发无副作用的 fixture 工具。此验证确认服务端接受并分发请求，未进行真实浏览器攻击复现。

**修复方向：** 在工具调度前校验请求来源、Host 和请求格式，并为本地会话绑定不可猜测的凭证。不要把随机端口当作鉴权机制。[MCP 传输规范](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports#security-warning)要求校验 Origin，并拒绝非法来源。

## R02：第二次运行会覆盖 Agent 自有凭据

**位置：** `crates/harness/src/config.rs:109–118,140–141`，`refresh_config_overlay`。

**触发条件：** 全局来源目录和 Agent 目标目录均有凭据文件，目标文件尚未被记录为继承文件。

第一次刷新遇到目标目录已有独立 `auth.json` 或 `.credentials.json` 时，会保留文件。但外层仍因来源文件存在，将文件名写入 `.meerkat-inherited.json`。第二次刷新因此把目标文件视为继承文件，删除它并链接或复制全局凭据。

**影响：** Agent 的独立登录状态被覆盖，可能切换到另一个账号或失去原有访问能力。

**验证：** 使用普通字符串作为凭据 fixture，连续调用两次原始配置刷新逻辑：

```text
run=1, credential=AGENT_FIXTURE, manifest=["auth.json"]
run=2, credential=GLOBAL_FIXTURE, manifest=["auth.json"]
```

**修复方向：** 只有实际继承的文件才能写入继承清单。保留已有本地凭据时，也必须保留其本地所有权。

## R03：同名 Telegram Bot 会被绑定到错误的 Agent

**位置：** `providers/telegram/src/client.rs:111–118`，`ManagedBotRegistration::matches`；关联 `providers/telegram/src/onboarding.rs:268–278,333–350`。

**触发条件：** Manager Bot 的队列中存在另一个 Bot 的注册事件，两个 Bot 的显示名称相同，但用户名不同。

匹配条件为 `username_matches || name_matches`。因此，只要显示名称相同，就会接受不属于本次注册目标的 Bot。onboarding 随后获取该 Bot 的凭据，并写入当前 Agent 的 secret 和 provider state。

**影响：** 当前 Agent 可能绑定到其他 Bot。尚未确认的旧事件也能触发错误绑定。

**验证：** 目标用户名为 `review_a_bot`，事件中的用户名为 `review_b_bot`，显示名称均为 `Developer`，生产函数返回 `matches=true`。

**修复方向：** 使用能关联本次注册的稳定身份判断归属。不能仅凭可重复的显示名称自动绑定凭据；若用户改变了注册用户名，需要明确的重新关联流程。

## R04：安装自定义 Harness 会阻止整个 Machine 启动

**位置：** `crates/machine-core/src/host/mod.rs:244–249`，`Machine::register_harness`。

**关联位置：**

- `apps/machine/src/lib.rs:294–359`：插件加载器接受新增 Harness ID。
- `apps/machine/src/lib.rs:243–246`：注册错误直接终止启动。
- `crates/machine-core/src/host/assignment.rs:554–558`：执行配置还有同样的内置 ID 限制。

**触发条件：** 安装清单有效、入口有效、使用新 Harness ID 的插件。

加载器可以构造并加入自定义插件，但注册函数仍只接受 `STANDARD_HARNESS_IDS` 中的三个内置 ID。注册失败会从启动入口传播出去，整个 Machine 无法正常启动。

**影响：** 新插件无法接入，且同一 Machine 上已有 Agent 也无法执行。

**验证：** 最小 JS 插件成功创建 `JsHarnessFactory`，注册时返回 `InvalidHarnessRegistration("review_custom")`。验证没有启动原生 CLI。

**修复方向：** 根据实际注册表验证 Harness，而不是固定 ID 列表。注册与 assignment 两处需要一致处理。

## R05：Telegram 团队无法通过成员新增流程创建 Agent

**位置：** `apps/web/src/features/members/useMemberDraft.ts:148–165`。

**关联位置：** `apps/web/src/features/members/creation.ts:14`；`providers/telegram/src/adapter.rs:619–650`。

**触发条件：** 已有 Telegram 团队通过成员新增流程创建新 Agent。

`onboards` 仅对 Feishu 返回真。因此，Telegram 创建请求同时提交空的 `provider_agent_config_json` 和空凭据。Telegram Provider 在缺少 `bot_token` 时，仅允许配置中的 `mode` 为 `new`；当前请求不满足条件，直接失败。

**影响：** 首次引导可以创建 Telegram 成员，但常规成员新增流程不能继续创建。该问题位于请求构造与注册流程，属于非 UI 业务逻辑。

**验证：** 静态核对了成员新增请求、`onboards` 条件和 Telegram Provider 的凭据检查分支。

**修复方向：** 与首次引导保持一致，提交 `mode=new` 和目标 Bot 用户名，并完成 Telegram onboarding。

## R06：一个 Agent 的轮询会丢弃其他 Agent 的注册事件

**位置：** `providers/telegram/src/onboarding.rs:258–273,360–369`。

**触发条件：** 多个 Agent 的注册流程使用同一个 Manager Bot，且轮询时间交错。

各 Agent 分别保存 offset，但读取的是同一个 Manager Bot 的更新队列。A 收到属于 B 的事件后，会因不匹配而跳过；`next_offset` 却按整个批次推进。A 下一次轮询便确认了 B 的事件，B 随后无法再取得它。

**影响：** 注册事件丢失，另一个 Agent 持续等待注册完成。

**验证：** 生产 onboarding API 对接遵循 Telegram 确认规则的本地 fixture：

```text
A: getUpdates(offset=0)   -> 收到 B 的 update_id=100，保存 offset=101
A: getUpdates(offset=101) -> 确认并移除 update_id=100
B: getUpdates(offset=0)   -> 无事件，状态仍为 Verifying
```

**修复方向：** 统一消费 Manager Bot 队列，先持久化并分发每个注册事件，再推进共享 offset。[Telegram getUpdates 文档](https://core.telegram.org/bots/api#getupdates)明确说明，较大的 offset 会确认此前的更新。

## R07：工作区暂停后，已发送成功的消息仍返回失败

**位置：** `crates/runtime/src/provider_ops.rs:54–100`，`Runtime::send_message`。

**关联位置：** `crates/runtime/src/lifecycle.rs:116–138`；`crates/runtime/src/coordination.rs:59–65`；`crates/runtime/src/capability.rs:534–565`。

**触发条件：** 工作区暂停时仍有活跃执行，之后该执行向已有会话发送消息。

暂停逻辑保留活跃执行使用的 Provider。发送成功并落库后，`send_message` 继续调用要求工作区处于 active 状态的 `coordinate_event_inner`。该调用返回 `WorkspacePaused`，使已成功的发送被当作调用失败。网关随后还会尝试将已确认的发送状态改为失败，引发状态冲突。

**影响：** 外部聊天已经收到消息，Agent 却得到错误结果，可能误判发送状态并再次发送。

**验证：** 使用临时数据库和隔离 Provider 调用 Runtime 公共 API：

```text
provider_send_count=1
result=Err(WorkspacePaused(...))
stored_state=sent
```

**修复方向：** 已确认的外部发送结果必须保留。暂停期间的内部事件协调应明确处理，不应通过后续协调错误推翻发送结果。

## R08：更新 Provider 凭据后，运行中的客户端继续使用旧值

**位置：** `crates/runtime/src/management/lifecycle.rs:394–398,447–455`，`AgentLifecycle::update`。

**关联位置：** `crates/runtime/src/lifecycle.rs` 的 `configure_agent`；Telegram、Feishu Provider 的 `create_agent`、`update_agent` 和客户端缓存。

**触发条件：** 工作区正在运行，通过管理 API 修改 Agent 的 Provider secret，例如 `bot_token` 或 `app_secret`。

只修改 secret 不会进入 `provider_profile_changed` 分支。后续操作仅更新数据库和 AgentRuntimeConfig，没有重建 Provider 客户端或接收任务。已有客户端仍持有创建时的凭据。

**影响：** 管理 API 返回更新成功后，收发消息仍使用旧凭据。旧凭据已被撤销时，保存新值不能立即恢复服务；需要重启工作区。

**验证：** 静态核对了 secret 更新入口、生命周期分支、`configure_agent` 的作用，以及 Provider 客户端创建和使用路径。

**修复方向：** 将凭据变化纳入 Provider 生命周期更新，安全地替换对应客户端和接收任务，并明确更新失败时的状态。

## R09：附件 ID 前缀冲突会生成错误路径

**位置：** `crates/machine-core/src/resources.rs:216–223`，`replace_resource_links`。

**触发条件：** 一次替换中包含具有前缀关系的资源 ID，例如 1 和 10，且先处理较短 ID。

函数逐项执行全局字符串替换，没有检查 ID 边界。处理 `resource://1` 时，也会替换 `resource://10` 的前缀；后续处理 ID 10 时，原始链接已经不存在。

**影响：** Agent 收到不存在或错误的附件路径。初始消息、追加输入和工具结果中的资源链接均可能受影响。

**验证：** 输入 `[one](resource://1) [ten](resource://10)`，分别映射到两个独立文件后，第二个链接变成 `/tmp/one.png0`。

**修复方向：** 解析完整资源链接，再按精确 ID 替换。`resource://` 与 `attachment://` 应使用相同的边界规则。

## R10：多段消息撤回只删除第一段

**位置：** `crates/storage/src/message.rs:835–854`，`recall_target`；`crates/runtime/src/capability.rs:947–975`，`invoke_message_recall`。

**关联位置：** `providers/feishu/src/adapter.rs:489–550`；`providers/telegram/src/adapter.rs:465–521`。

**触发条件：** 一条逻辑消息包含文字和附件，或包含多个附件，Provider 将其发送为多个平台消息。

`recall_target` 只读取主 `provider_message_id`。Runtime 只调用一次撤回，成功后便报告整条消息撤回成功。Feishu 已保存的 `message_provider_parts` 未参与撤回；Telegram 发送时只保留第一条回执，其他 ID 没有返回给 Runtime。

**影响：** 文字被撤回后，附件仍留在聊天中，撤回结果与实际内容不一致。

**验证：** 在临时数据库保存 `text-id`、`file-id` 两个分段后，`recall_target` 只返回 `text-id`；同时核对 Runtime 和两个 Provider 的撤回调用链。

**修复方向：** Provider 返回全部分段 ID，Storage 保留完整映射，Runtime 逐段撤回并处理部分失败，不能在仅删除主消息后报告全部成功。

## R11：Claude Code 收不到工具失败的原因

**位置：** `crates/harness/src/plugin_mcp.rs:159–164`。

**关联位置：** `crates/machine-core/src/host/control.rs:316–326`；`crates/runtime/src/capability.rs` 的 `capability_error`。

**触发条件：** 工具返回失败，错误详情位于 `ToolResultRequest.error`，正文和结构化内容为空。

Machine 已将错误转换成 `ToolResultRequest.error`，但 MCP 响应只使用正文、结构化内容和 `isError`，忽略了错误的 `code`、`message`、`retryable`。Runtime 的 `capability_error` 正是通过独立 error 字段返回详情。

**影响：** Claude Code 只能看到空错误，无法了解失败原因或判断是否应重试。

**验证：** 隔离工具返回带有错误码、说明和重试标记的失败结果，MCP 响应仅包含：

```json
{
  "content": [{ "type": "text", "text": "" }],
  "isError": true,
  "structuredContent": {}
}
```

**修复方向：** 将错误信息显式映射到 MCP 工具结果，保留足以让调用方决定下一步的信息。

## 已核对但未列为缺陷的行为

鉴权或配置失败导致 Agent 暂停后，原失败批次不会因恢复 Agent 而重新入队。该行为已隔离复现，但 `docs/remote-machine.md:198–207` 明确规定停止重试时将批次标记为失败并发送通知。因此，本轮将其视为既定策略，不计入上述 11 项，也不建议在修复时顺带改变。
