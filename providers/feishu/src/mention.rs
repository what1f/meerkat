//! Translation between Lankide's `@name[id]` marker and Feishu native @.

use lankide_core::{markdown_mention, markdown_mentions};
use std::collections::HashMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MentionTarget {
    pub agent_id: String,
    pub external_id: String,
    pub display_name: String,
}

impl MentionTarget {
    pub fn new(
        agent_id: impl Into<String>,
        external_id: impl Into<String>,
        display_name: impl Into<String>,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            external_id: external_id.into(),
            display_name: display_name.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderedMention {
    pub agent_id: String,
    pub external_id: String,
    pub display_name: String,
    pub marker: String,
}

/// Return the unique, ordered Agent IDs present in a Markdown string.
pub fn extract_agent_ids(content: &str) -> Vec<String> {
    let mut result = Vec::new();
    for mention in markdown_mentions(content) {
        if !result.iter().any(|existing| existing == mention.id) {
            result.push(mention.id.to_owned());
        }
    }
    result
}

/// Render markers using Feishu's native Card/Markdown `<at>` element.
///
/// Unknown markers are retained verbatim.  This is important when Runtime
/// receives a message from one Workspace but the current provider instance
/// does not own the mentioned Agent.
pub fn render_card_markdown(
    content: &str,
    targets: &[MentionTarget],
) -> (String, Vec<RenderedMention>) {
    let by_agent = targets
        .iter()
        .filter(|target| {
            !target.agent_id.trim().is_empty() && !target.external_id.trim().is_empty()
        })
        .map(|target| (target.agent_id.trim().to_owned(), target))
        .collect::<HashMap<_, _>>();
    replace_markers(content, |name, id, marker| {
        by_agent
            .get(id)
            .map(|target| RenderedMention {
                agent_id: target.agent_id.clone(),
                external_id: target.external_id.clone(),
                display_name: preferred_name(&target.display_name, name),
                marker: format!("<at id={}></at>", xml_escape(target.external_id.trim())),
            })
            .or_else(|| {
                Some(RenderedMention {
                    agent_id: id.to_owned(),
                    external_id: String::new(),
                    display_name: name.to_owned(),
                    marker: marker.to_owned(),
                })
            })
    })
}

/// Render markers using Feishu's text-message `<at user_id="...">name</at>`.
pub fn render_text(content: &str, targets: &[MentionTarget]) -> (String, Vec<RenderedMention>) {
    let by_agent = targets
        .iter()
        .filter(|target| {
            !target.agent_id.trim().is_empty() && !target.external_id.trim().is_empty()
        })
        .map(|target| (target.agent_id.trim().to_owned(), target))
        .collect::<HashMap<_, _>>();
    replace_markers(content, |name, id, marker| {
        by_agent
            .get(id)
            .map(|target| RenderedMention {
                agent_id: target.agent_id.clone(),
                external_id: target.external_id.clone(),
                display_name: preferred_name(&target.display_name, name),
                marker: format!(
                    "<at user_id=\"{}\">{}</at>",
                    xml_escape(target.external_id.trim()),
                    xml_escape(&preferred_name(&target.display_name, name))
                ),
            })
            .or_else(|| {
                Some(RenderedMention {
                    agent_id: id.to_owned(),
                    external_id: String::new(),
                    display_name: name.to_owned(),
                    marker: marker.to_owned(),
                })
            })
    })
}

fn replace_markers<F>(content: &str, mut replacement: F) -> (String, Vec<RenderedMention>)
where
    F: FnMut(&str, &str, &str) -> Option<RenderedMention>,
{
    let mut rendered = String::with_capacity(content.len());
    let mut mentions = Vec::new();
    let mut cursor = 0;
    for mention in markdown_mentions(content) {
        let start = mention.range.start;
        let end = mention.range.end;
        rendered.push_str(&content[cursor..start]);
        let marker = &content[start..end];
        if let Some(value) = replacement(mention.name, mention.id, marker) {
            rendered.push_str(&value.marker);
            if !mentions
                .iter()
                .any(|existing: &RenderedMention| existing.agent_id == value.agent_id)
            {
                mentions.push(value);
            }
        } else {
            rendered.push_str(marker);
        }
        cursor = end;
    }
    if cursor < content.len() {
        rendered.push_str(&content[cursor..]);
    }
    (rendered, mentions)
}

pub fn inbound_marker(display_name: &str, id: &str) -> Option<String> {
    markdown_mention(display_name, id)
}

fn preferred_name(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.to_owned()
    } else {
        value.to_owned()
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
