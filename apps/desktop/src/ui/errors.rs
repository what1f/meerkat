use super::*;

pub(super) fn friendly_error(error: &ClientError, language: DesktopLanguage) -> String {
    match error {
        ClientError::DuplicateWorkspaceId(_) => language
            .text(
                "这个 Workspace ID 已经被使用。",
                "This Workspace ID is already in use.",
            )
            .into(),
        ClientError::DuplicateAgentId(_) => language
            .text(
                "这个 Agent ID 已经被使用。",
                "This Agent ID is already in use.",
            )
            .into(),
        ClientError::InvalidAgentId => language
            .text(
                "Agent ID 只能使用小写字母、数字和下划线。",
                "Agent IDs may contain lowercase letters, digits and underscores only.",
            )
            .into(),
        ClientError::WorkspaceNotFound(_) => language
            .text(
                "找不到这个工作区，请刷新后重试。",
                "Workspace not found. Refresh to try again.",
            )
            .into(),
        ClientError::AgentNotFound(_) => language
            .text(
                "找不到这位 Agent，请刷新后重试。",
                "Agent not found. Refresh to try again.",
            )
            .into(),
        ClientError::Validation(message) => format!(
            "{} {message}",
            language.text("请检查填写内容：", "Check the entered values:")
        ),
        ClientError::Provider(_) => language
            .text(
                "暂时无法完成这个操作，请稍后重试。",
                "The chat service could not complete this action. Try again later.",
            )
            .into(),
        ClientError::Unavailable(_) | ClientError::Transport(_) | ClientError::StreamEnded => {
            language
                .text(
                    "暂时无法连接 Lankide，请稍后重试。",
                    "Cannot connect to Lankide. Try again shortly.",
                )
                .into()
        }
        ClientError::ComputerHasAssignedAgents => language
            .text(
                "请先将这台电脑上的 Agent 移到其他电脑。",
                "Move this computer’s Agents to another computer first.",
            )
            .into(),
        ClientError::LocalComputerCannotBeRemoved => language
            .text("无法移除本机。", "This computer cannot be removed.")
            .into(),
        ClientError::ComputerNameInvalid => language
            .text(
                "请输入 1–80 个字符的名称。",
                "Enter a name with 1–80 characters.",
            )
            .into(),
        ClientError::ComputerNotRegistered
        | ClientError::ComputerRevoked
        | ClientError::HarnessUnavailableOnComputer
        | ClientError::Protocol(_) => language
            .text(
                "暂时无法完成操作，请稍后重试。",
                "Could not complete this action. Try again shortly.",
            )
            .into(),
    }
}
