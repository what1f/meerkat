//! Fresh-session decisions and the context a fresh native session starts with.
//!
//! A native session that sat idle past its model's prompt-cache lifetime is
//! re-read at full price on the next run. Once it is also large, starting a
//! new session seeded from Runtime records is cheaper. The seed is built from
//! stored conversations, so it is the same for every Harness.

use std::collections::BTreeSet;

use meerkat_storage::{AttentionRecord, RecentConversationView, SnapshotView};

/// Context size above which a cold session is replaced instead of resumed.
/// The Harnesses compact at 128k by default, so a replaced session is one
/// that grew past half of that since its last compaction.
const RESET_TOKENS: u64 = 64_000;
const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 60 * MINUTE_MS;

/// Conversations considered: participation within this window.
pub(crate) const RECENT_WINDOW_MS: i64 = 24 * HOUR_MS;
pub(crate) const RECENT_CHAT_LIMIT: usize = 6;
pub(crate) const RECENT_MESSAGE_LIMIT: usize = 8;
/// Character budget for all conversation lines together (about 6k tokens).
const CONVERSATION_BUDGET_CHARS: usize = 9_000;
/// A single message longer than this is cut with an explicit marker.
const MESSAGE_MAX_CHARS: usize = 600;

/// How long a native session may stay idle before its cache is gone, by
/// model family.
fn idle_limit_ms(harness_id: &str, model: &str) -> i64 {
    let model = model.to_ascii_lowercase();
    if model.contains("deepseek") {
        24 * HOUR_MS
    } else if model.contains("gpt")
        || model.contains("codex")
        || model.starts_with('o') && model[1..].starts_with(|c: char| c.is_ascii_digit())
    {
        30 * MINUTE_MS
    } else if ["claude", "opus", "sonnet", "haiku"]
        .iter()
        .any(|family| model.contains(family))
    {
        HOUR_MS
    } else if model.is_empty() && harness_id == "codex" {
        30 * MINUTE_MS
    } else {
        HOUR_MS
    }
}

/// Agent environment variables that replace the defaults above.
const RESET_TOKENS_VARIABLE: &str = "MEERKAT_RESET_TOKENS";
const RESET_IDLE_VARIABLE: &str = "MEERKAT_RESET_IDLE_SECONDS";

fn variable(harness: &meerkat_protocol::HarnessConfig, name: &str) -> Option<i64> {
    harness
        .environment_variables
        .iter()
        .find(|variable| variable.name == name)
        .and_then(|variable| variable.value.trim().parse::<i64>().ok())
}

/// True when the next run should start a new native session.
pub(crate) fn should_start_fresh(
    harness: &meerkat_protocol::HarnessConfig,
    last_completed_at: i64,
    last_context_tokens: Option<u64>,
    now: i64,
) -> bool {
    let idle_ms = variable(harness, RESET_IDLE_VARIABLE)
        .map(|seconds| seconds.saturating_mul(1_000))
        .unwrap_or_else(|| idle_limit_ms(&harness.harness_id, &harness.model));
    let tokens = variable(harness, RESET_TOKENS_VARIABLE)
        .map(|tokens| tokens.max(0) as u64)
        .unwrap_or(RESET_TOKENS);
    now.saturating_sub(last_completed_at) >= idle_ms
        && last_context_tokens.is_some_and(|context| context > tokens)
}

/// Recent conversations as one XML-delimited section, or empty when there is
/// nothing to add beyond the unread list. Messages already shown in the
/// unread snapshot are left out; when over budget, whole oldest messages drop
/// first so every line that remains is complete.
/// First run after the Agent moved to another Machine.
pub(crate) const MOVED_NOTICE: &str = "你刚搬到了另一台电脑，记忆和工作文件已经带过来。之前记下的绝对路径、本机工具和登录状态可能不一样了，用到之前先核实。";

pub(crate) fn render_recovery(
    conversations: &[RecentConversationView],
    snapshot: &SnapshotView,
    now: i64,
) -> String {
    let shown = snapshot
        .chats
        .iter()
        .flat_map(|chat| {
            chat.recent_messages
                .iter()
                .chain(&chat.trigger_messages)
                .chain(chat.mentioned_message.as_ref())
                .map(|message| message.message_id)
        })
        .collect::<BTreeSet<_>>();
    // (chat index, occurred_at, line)
    let mut lines = Vec::new();
    for (index, conversation) in conversations.iter().enumerate() {
        for recent in &conversation.messages {
            if shown.contains(&recent.message.message_id) {
                continue;
            }
            let sender = if recent.own {
                if recent.delivered {
                    "你".to_owned()
                } else {
                    "你（未送达）".to_owned()
                }
            } else if recent.message.sender_name.trim().is_empty() {
                recent.message.sender_id.clone()
            } else {
                recent.message.sender_name.clone()
            };
            lines.push((
                index,
                recent.message.occurred_at,
                format!(
                    "- [message_id={}] {sender}: {}",
                    recent.message.message_id,
                    clip(&recent.message.content)
                ),
            ));
        }
    }
    let mut total = lines
        .iter()
        .map(|(_, _, line)| line.chars().count())
        .sum::<usize>();
    while total > CONVERSATION_BUDGET_CHARS && lines.len() > 1 {
        let oldest = lines
            .iter()
            .enumerate()
            .min_by_key(|(_, (_, at, _))| *at)
            .map(|(position, _)| position)
            .expect("non-empty");
        total -= lines.remove(oldest).2.chars().count();
    }
    if lines.is_empty() {
        return String::new();
    }
    let mut output = String::from(
        "<recent-conversations note=\"你最近参与的会话片段，更早内容用 chat_messages 读取\">",
    );
    for (index, conversation) in conversations.iter().enumerate() {
        let body = lines
            .iter()
            .filter(|(chat, _, _)| *chat == index)
            .map(|(_, _, line)| line.as_str())
            .collect::<Vec<_>>();
        if body.is_empty() {
            continue;
        }
        let chat = &conversation.chat;
        let kind = match chat.channel_id.as_str() {
            "group_chat" => "群聊",
            "private_chat" => "私聊",
            "task" => "任务",
            _ => chat.channel_name.as_str(),
        };
        let name = if chat.chat_name.trim().is_empty() {
            chat.provider_chat_id.as_str()
        } else {
            chat.chat_name.as_str()
        };
        output.push_str(&format!(
            "\n<chat kind=\"{kind}\" name=\"{}\" channel_id=\"{}\" chat_id=\"{}\"{} last-active=\"{}\">\n{}\n</chat>",
            attribute(name),
            chat.channel_id,
            attribute(&chat.provider_chat_id),
            conversation
                .attention
                .as_ref()
                .map(|attention| format!(" attention=\"{}\"", attention_name(attention)))
                .unwrap_or_default(),
            elapsed(now.saturating_sub(conversation.last_participation_at)),
            body.join("\n"),
        ));
    }
    output.push_str("\n</recent-conversations>");
    output
}

fn attention_name(attention: &AttentionRecord) -> String {
    match attention {
        AttentionRecord::Always => "follow".into(),
        AttentionRecord::Mention => "mention".into(),
        AttentionRecord::Mute => "mute".into(),
        AttentionRecord::Schedule { interval_minutes } => format!("schedule {interval_minutes}m"),
    }
}

fn elapsed(ms: i64) -> String {
    if ms < HOUR_MS {
        format!("{} 分钟前", (ms / MINUTE_MS).max(1))
    } else if ms < 24 * HOUR_MS {
        format!("{} 小时前", ms / HOUR_MS)
    } else {
        format!("{} 天前", ms / (24 * HOUR_MS))
    }
}

fn attribute(value: &str) -> String {
    value.replace('"', "'").replace(['\n', '\r'], " ")
}

fn clip(content: &str) -> String {
    let flat = content.trim();
    if flat.chars().count() <= MESSAGE_MAX_CHARS {
        return flat.to_owned();
    }
    let mut clipped = flat.chars().take(MESSAGE_MAX_CHARS).collect::<String>();
    clipped.push_str("…（已截断，完整内容用 chat_messages 读取）");
    clipped
}
