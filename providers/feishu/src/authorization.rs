use crate::client::Domain;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flate2::{Compression, write::GzEncoder};
use serde::Serialize;
use std::io::Write;
use url::Url;

pub const STANDARD_BOT_TENANT_SCOPES: [&str; 26] = [
    "application:application:patch",
    "application:application:self_manage",
    "application:bot.basic_info:read",
    "cardkit:card:read",
    "cardkit:card:write",
    "contact:user.base:readonly",
    "im:chat.members:bot_access",
    "im:chat:read",
    "im:message.group_msg",
    "im:message.group_msg.include_bot:read",
    "im:message.group_at_msg.include_bot:readonly",
    "im:message.group_at_msg:readonly",
    "im:message.p2p_msg:readonly",
    "im:message.reactions:read",
    "im:message.reactions:write_only",
    "im:message:readonly",
    "im:message:recall",
    "im:message:send_as_bot",
    "im:resource",
    "task:comment",
    "task:comment:read",
    "task:comment:write",
    "task:task:read",
    "task:task:write",
    "task:tasklist:read",
    "task:tasklist:write",
];

pub const STANDARD_BOT_EVENTS: [&str; 5] = [
    "im.message.receive_v1",
    "im.message.reaction.created_v1",
    "im.message.reaction.deleted_v1",
    "im.chat.updated_v1",
    "task.task.comment.updated_v1",
];

#[derive(Serialize)]
struct Addons<'a> {
    scopes: Scopes<'a>,
    events: Events<'a>,
}

#[derive(Serialize)]
struct Scopes<'a> {
    tenant: &'a [&'a str],
    user: &'a [&'a str],
}

#[derive(Serialize)]
struct Events<'a> {
    items: EventItems<'a>,
}

#[derive(Serialize)]
struct EventItems<'a> {
    tenant: &'a [&'a str],
    user: [String; 0],
}

pub fn authorization_url(domain: &Domain, app_id: &str) -> Result<String, String> {
    let app_id = app_id.trim();
    if app_id.is_empty() {
        return Err("missing App ID".into());
    }
    let host = domain.base_url();
    let raw = serde_json::to_vec(&Addons {
        scopes: Scopes {
            tenant: &STANDARD_BOT_TENANT_SCOPES,
            user: &[],
        },
        events: Events {
            items: EventItems {
                tenant: &STANDARD_BOT_EVENTS,
                user: [],
            },
        },
    })
    .map_err(|_| "could not encode authorization request")?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&raw)
        .map_err(|_| "could not compress authorization request")?;
    let addons = URL_SAFE_NO_PAD.encode(
        encoder
            .finish()
            .map_err(|_| "could not compress authorization request")?,
    );
    let mut url = Url::parse(&format!("{host}/page/launcher"))
        .map_err(|_| "could not build authorization link")?;
    url.query_pairs_mut()
        .append_pair("clientID", app_id)
        .append_pair("addons", &addons);
    Ok(url.into())
}

pub fn contains_standard_capabilities(scopes: &[String], events: &[String]) -> bool {
    let (missing_scopes, missing_events) = standard_capability_gaps(scopes, events);
    missing_scopes.is_empty() && missing_events.is_empty()
}

pub(crate) fn standard_capability_gaps(
    scopes: &[String],
    events: &[String],
) -> (Vec<String>, Vec<String>) {
    let missing_scopes = STANDARD_BOT_TENANT_SCOPES
        .iter()
        .filter(|required| !scopes.iter().any(|actual| actual == **required))
        .map(|value| (*value).to_owned())
        .collect();
    let missing_events = STANDARD_BOT_EVENTS
        .iter()
        .filter(|required| !events.iter().any(|actual| actual == **required))
        .map(|value| (*value).to_owned())
        .collect();
    (missing_scopes, missing_events)
}
