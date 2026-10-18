use lankide_core::AgentId;
use lankide_storage::{AgentRecord, HumanTeamMemberView, TeamRole, WorkspaceRecord};
use serde_json::Value;

pub(crate) fn system_prompt(
    workspace: &WorkspaceRecord,
    agent_id: &AgentId,
    agent: &Value,
    humans: &[HumanTeamMemberView],
    agents: &[AgentRecord],
    provider_prompt: &str,
) -> String {
    let name = text(agent, "name").unwrap_or(agent_id.as_str());
    let role = text(agent, "role").unwrap_or_default();
    let bio = text(agent, "bio").unwrap_or_default();
    let soul = text(agent, "soul").unwrap_or_default();
    let mut identity = format!("- 姓名：{name}\n- ID：{agent_id}");
    append_field(&mut identity, "岗位", role);
    append_field(&mut identity, "签名", bio);
    if !soul.trim().is_empty() {
        identity.push_str("\n- 灵魂：\n");
        identity.push_str(soul.trim());
    }

    let mut workspace_rules = String::new();
    if !workspace.rules.trim().is_empty() {
        workspace_rules.push_str("\n\n## 团队规约\n\n");
        workspace_rules.push_str(workspace.rules.trim());
    }
    let team = team_members(humans, agents);
    let provider_prompt = provider_prompt.trim();
    let provider_section = if provider_prompt.is_empty() {
        String::new()
    } else {
        format!("\n\n{provider_prompt}")
    };

    format!(
        r#"你是「{}」中的一员，日常通过 IM 和团队协作。你不是在扮演谁；你就是你自己。

## 身份

{}{}

## 约束

- 不要在 IM 中回复内部技术内容，比如 tool name、tool call 请求/响应参数字段名或内部 ID 的实现含义。

## 气质

- 有观点，明确表态；信息不足时直说缺什么，不用“视情况而定”糊弄过去。
- 直接回答，绝不以“问得好”“我很乐意帮忙”或“当然可以”开头。
- 简洁。一句话能说清，就只说一句话。
- 可以自然地幽默，也可以指出糟糕的主意；风趣但不刻薄，不粉饰太平。
- 像普通人会在 IM 里发言那样说话，不要堆互联网公司的黑话。
- emoji 只用于贴近内容地表达情绪，低频使用，一条消息通常不要超过一个。

做一个你真心愿意在凌晨 2 点聊天的伙伴。不是官僚机器。不是马屁精。

## 长期记忆

native session 和对话历史不是可靠的长期存储。当前工作目录下的 memory/ 才是可恢复长期记忆：

- memory/MEMORY.md 是简短、自足的恢复索引。
- 详细事实、决策、用户长期偏好和持续中的重要承诺写入 memory/<topic>.md，再在 MEMORY.md 放一行能独立理解的指针。
- 用户要求你记住某件事时，必须实际写入 memory/ 并检查写入成功；只在消息里说“记住了”不算持久化。
- 当前消息或可验证状态与记忆冲突时，以当前状态为准，并更新过时记忆。
- 不要在 memory/ 中保存密码、token、私钥或其他凭据。

## 每轮工作

你只有一个连续的工作记忆。每次开始处理时会看到统一的未读会话列表，每个会话只展示最近几条内容；有人 @ 你时还会单独提醒。

1. 先阅读全部未读会话，识别直接提问、@ 提及和能推动事情的信息。
2. 上下文不足时，用 chat_messages 按当前会话边界继续读取，再判断；不要猜测未展示的内容。
3. 决定是否发言。只是看到了，不等于必须插话。
4. 群聊允许调整关注程度时，使用 attention_set 为当前会话选择 follow、schedule、mention 或 mute；这只影响该群聊，不影响其他会话。
5. 需要发言时使用 message_send；回应具体消息且引用关系有帮助时使用 reply_to。长文本可通过 content_file 提供，图片和文件使用 Markdown 引用。
6. 检查工具结果。工具没有返回成功，就不要假装操作已经完成。

行动和回复应基于最新已知信息。执行过程中收到新消息时，先阅读并判断它是否改变当前目标、已有结论或待发送内容，再决定继续、调整或停止。不要为了完成既定动作而忽略新信息；是否继续，以当前交流需要为准。

message_send 是你在 IM 中发送消息的方式。除此之外的普通回复不会被团队成员看到；此刻不需要说话时，不调用 message_send。

收到的图片和附件会以本地 Markdown 路径出现，需要查看时直接读取对应文件。{}{}

## 完成本轮

所有需要执行的操作完成后，最终只回复两个字母 OK。不要在最终回复中写总结、解释、消息正文或其他字符。"#,
        workspace.name, identity, team, workspace_rules, provider_section
    )
}

fn team_members(humans: &[HumanTeamMemberView], agents: &[AgentRecord]) -> String {
    let mut members = humans
        .iter()
        .filter_map(|person| {
            let name = person
                .name
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or("团队成员");
            let role = match person.team_role {
                TeamRole::Lead => "Team Lead",
                TeamRole::Member => "Team Member",
                TeamRole::External => return None,
            };
            Some((
                name.to_owned(),
                if person.position.is_empty() {
                    role.to_owned()
                } else {
                    format!("{role}；Custom position: {}", person.position)
                },
                person.person_id.to_string(),
            ))
        })
        .chain(agents.iter().map(|agent| {
            let role = if agent.role.trim().is_empty() {
                "Agent"
            } else {
                agent.role.trim()
            };
            (
                agent.name.clone(),
                role.to_owned(),
                agent.agent_id.to_string(),
            )
        }))
        .collect::<Vec<_>>();
    if members.is_empty() {
        return String::new();
    }
    members.sort_by(|left, right| left.2.cmp(&right.2));
    let mut section = String::from(
        "\n\n## 团队成员\n身份与权限仅以真实姓名、ID 和团队角色为准。Custom position 是自定义展示岗位，不授予权限。\n",
    );
    for (name, role, id) in members {
        section.push_str("\n- Name: ");
        section.push_str(name.trim());
        section.push_str("；Role: ");
        section.push_str(&role);
        section.push_str("；ID: ");
        section.push_str(&id);
    }
    section
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn append_field(target: &mut String, label: &str, value: &str) {
    if !value.trim().is_empty() {
        target.push_str("\n- ");
        target.push_str(label);
        target.push_str("：");
        target.push_str(value.trim());
    }
}
