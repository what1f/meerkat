//! Telegram mention and Markdown-to-HTML rendering.

use crate::dto::{MessageEntity, User};
use lankide_core::markdown_mentions;
use std::collections::HashMap;
use std::fmt::Write;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MentionTarget {
    pub agent_id: String,
    pub external_user_id: Option<i64>,
    pub username: Option<String>,
    pub display_name: String,
}

impl MentionTarget {
    pub fn new(
        agent_id: impl Into<String>,
        external_user_id: Option<i64>,
        username: Option<String>,
        display_name: impl Into<String>,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            external_user_id,
            username,
            display_name: display_name.into(),
        }
    }
}

pub fn extract_agent_ids(content: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for mention in markdown_mentions(content) {
        if !result.iter().any(|existing| existing == mention.id) {
            result.push(mention.id.to_owned());
        }
    }
    result
}

/// Render Lankide Markdown to Telegram HTML while replacing agent markers
/// with native `tg://user?id=` links when the target has a numeric user ID.
pub fn render_html_with_mentions(markdown: &str, targets: &[MentionTarget]) -> String {
    let by_id = targets
        .iter()
        .filter(|target| !target.agent_id.trim().is_empty())
        .map(|target| (target.agent_id.trim().to_owned(), target))
        .collect::<HashMap<_, _>>();
    let mut source = String::with_capacity(markdown.len());
    let mut replacements = Vec::new();
    let mut cursor = 0;
    for mention in markdown_mentions(markdown) {
        let start = mention.range.start;
        let end = mention.range.end;
        source.push_str(&markdown[cursor..start]);
        if let Some(target) = by_id.get(mention.id) {
            let sentinel = format!("\u{E000}{:x}\u{E001}", replacements.len());
            source.push_str(&sentinel);
            replacements.push((sentinel, target_html(target, mention.name)));
        } else {
            source.push_str(&markdown[start..end]);
        }
        cursor = end;
    }
    if cursor < markdown.len() {
        source.push_str(&markdown[cursor..]);
    }
    let mut rendered = render_markdown(&source);
    for (sentinel, replacement) in replacements {
        rendered = rendered.replace(&sentinel, &replacement);
    }
    rendered
}

pub fn render_markdown(markdown: &str) -> String {
    let lines = markdown
        .replace("\r\n", "\n")
        .split('\n')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut result = String::new();
    let mut code = String::new();
    let mut in_fence = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if in_fence {
                if !result.is_empty() && !result.ends_with('\n') {
                    result.push('\n');
                }
                result.push_str("<pre>");
                result.push_str(&escape_html(code.trim_end_matches('\n')));
                result.push_str("</pre>");
                code.clear();
                in_fence = false;
            } else {
                in_fence = true;
            }
            continue;
        }
        if in_fence {
            code.push_str(&line);
            code.push('\n');
            continue;
        }
        if !result.is_empty() {
            result.push('\n');
        }
        if let Some(level) = heading_level(&line) {
            result.push_str("<b>");
            result.push_str(&render_inline(line.trim()[level..].trim()));
            result.push_str("</b>");
            continue;
        }
        if let Some(quote) = line.trim_start().strip_prefix("> ") {
            result.push_str("<blockquote>");
            result.push_str(&render_inline(quote));
            result.push_str("</blockquote>");
            continue;
        }
        result.push_str(&render_inline(&line));
    }
    if in_fence {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str("<pre>");
        result.push_str(&escape_html(code.trim_end_matches('\n')));
        result.push_str("</pre>");
    }
    result.trim().to_owned()
}

pub fn mentioned_agent_ids(
    text: &str,
    entities: &[MessageEntity],
    bot: &User,
    by_external_id: &HashMap<i64, String>,
) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for entity in entities {
        let external_id = match entity.entity_type.as_str() {
            "text_mention" => entity
                .user
                .as_ref()
                .filter(|user| user.is_bot)
                .map(|user| user.id),
            "mention" => {
                let username_value = utf16_slice(text, entity.offset, entity.length);
                let username = username_value.trim_start_matches('@');
                bot.username
                    .eq_ignore_ascii_case(username)
                    .then_some(bot.id)
            }
            _ => None,
        };
        if let Some(external_id) = external_id
            .and_then(|id| by_external_id.get(&id))
            .filter(|id| !result.contains(id))
        {
            result.push(external_id.to_owned());
        }
    }
    result
}

pub fn utf16_slice(value: &str, offset: usize, length: usize) -> String {
    let units = value.encode_utf16().collect::<Vec<_>>();
    if length == 0 || offset >= units.len() {
        return String::new();
    }
    let end = offset.saturating_add(length).min(units.len());
    String::from_utf16_lossy(&units[offset..end])
}

fn target_html(target: &MentionTarget, fallback: &str) -> String {
    let name = if target.display_name.trim().is_empty() {
        fallback
    } else {
        target.display_name.trim()
    };
    if let Some(user_id) = target.external_user_id {
        format!(
            "<a href=\"tg://user?id={user_id}\">{}</a>",
            escape_html(name)
        )
    } else if let Some(username) = target
        .username
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        format!("@{}", escape_html(username.trim_start_matches('@')))
    } else {
        escape_html(name)
    }
}

fn render_inline(value: &str) -> String {
    let mut result = String::new();
    let mut index = 0;
    while index < value.len() {
        let bytes = value.as_bytes();
        if bytes[index] == b'\\' && index + 1 < value.len() {
            let next = value[index + 1..].chars().next().unwrap_or_default();
            result.push_str(&escape_html(&next.to_string()));
            index += 1 + next.len_utf8();
            continue;
        }
        if bytes[index] == b'`' {
            if let Some(end) = value[index + 1..].find('`') {
                result.push_str("<code>");
                result.push_str(&escape_html(&value[index + 1..index + 1 + end]));
                result.push_str("</code>");
                index += end + 2;
                continue;
            }
        }
        let markers = [
            ("**", "<b>", "</b>"),
            ("__", "<b>", "</b>"),
            ("~~", "<s>", "</s>"),
            ("*", "<i>", "</i>"),
        ];
        let mut handled = false;
        for (marker, open, close) in markers {
            if value[index..].starts_with(marker)
                && (marker != "*" || emphasis_boundary(value, index))
            {
                if let Some(end) = value[index + marker.len()..].find(marker) {
                    if end > 0 {
                        result.push_str(open);
                        result.push_str(&render_inline(
                            &value[index + marker.len()..index + marker.len() + end],
                        ));
                        result.push_str(close);
                        index += marker.len() + end + marker.len();
                        handled = true;
                        break;
                    }
                }
            }
        }
        if handled {
            continue;
        }
        if bytes[index] == b'[' {
            if let Some(label_end) = value[index + 1..].find("](") {
                let label_end = index + 1 + label_end;
                let destination_start = label_end + 2;
                if let Some(dest_offset) = value[destination_start..].find(')') {
                    let destination_end = destination_start + dest_offset;
                    let destination = value[destination_start..destination_end].trim();
                    if let Some(scheme) = destination.split(':').next() {
                        if matches!(scheme, "http" | "https" | "tg" | "mailto") {
                            let _ = write!(
                                result,
                                "<a href=\"{}\">{}</a>",
                                escape_html(destination),
                                render_inline(&value[index + 1..label_end])
                            );
                            index = destination_end + 1;
                            continue;
                        }
                    }
                }
            }
        }
        let character = value[index..].chars().next().unwrap_or_default();
        result.push_str(&escape_html(&character.to_string()));
        index += character.len_utf8();
    }
    result
}

fn heading_level(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    let mut level = 0;
    while level < trimmed.len() && level < 6 && trimmed.as_bytes()[level] == b'#' {
        level += 1;
    }
    (level > 0 && level < trimmed.len() && trimmed.as_bytes()[level] == b' ').then_some(level)
}

fn emphasis_boundary(value: &str, index: usize) -> bool {
    index == 0
        || value[..index]
            .chars()
            .next_back()
            .is_some_and(|character| character.is_whitespace() || character.is_ascii_punctuation())
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
