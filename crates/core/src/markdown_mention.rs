use std::ops::Range;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownMention<'a> {
    pub name: &'a str,
    pub id: &'a str,
    pub range: Range<usize>,
}

pub fn markdown_mentions(content: &str) -> Vec<MarkdownMention<'_>> {
    let mut mentions = Vec::new();
    let mut cursor = 0;
    while let Some(relative_at) = content[cursor..].find('@') {
        let start = cursor + relative_at;
        let Some(relative_open) = content[start + 1..].find('[') else {
            break;
        };
        let open = start + 1 + relative_open;
        let name = &content[start + 1..open];
        if name.is_empty()
            || name
                .chars()
                .any(|character| matches!(character, '@' | ']' | '\n' | '\r'))
        {
            cursor = start + 1;
            continue;
        }
        let Some(relative_close) = content[open + 1..].find(']') else {
            break;
        };
        let close = open + 1 + relative_close;
        let id = &content[open + 1..close];
        if !valid_id(id) {
            cursor = start + 1;
            continue;
        }
        mentions.push(MarkdownMention {
            name,
            id,
            range: start..close + 1,
        });
        cursor = close + 1;
    }
    mentions
}

pub fn markdown_mention(name: &str, id: &str) -> Option<String> {
    let name = name.trim();
    let id = id.trim();
    if name.is_empty()
        || name
            .chars()
            .any(|character| matches!(character, '@' | '[' | ']' | '\n' | '\r'))
        || !valid_id(id)
    {
        return None;
    }
    Some(format!("@{name}[{id}]"))
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}
