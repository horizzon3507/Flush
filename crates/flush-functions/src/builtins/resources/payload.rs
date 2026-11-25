//! `payload.ts`: turns a `$` argument into a send payload. Text, one embed, a
//! list of embeds, or a message object with `content`, `embeds`, `tts`,
//! `stickerIds`, `flags` and `allowedMentions`.

use serde_json::{Map, Value};

use flush_core::{AllowedMentions, MessageSend};

use crate::call::ScriptCall;
use crate::errors::FnResult;
use crate::types::Definitions;
use crate::values::text;

use super::embeds::{assert_embed_count, embed_to_wire, fail, normalize_embed, EMBEDS_PER_MESSAGE};
use super::{arg_value, require_permission};

const MESSAGE_KEYS: &[&str] = &[
    "content",
    "embeds",
    "tts",
    "stickerIds",
    "flags",
    "allowedMentions",
];
const MENTION_PARSE: &[&str] = &["users", "roles", "everyone"];
const MESSAGE_LENGTH: usize = 2000;
const MAX_STICKERS: usize = 3;
const MAX_MENTION_IDS: usize = 100;

fn is_snowflake_text(s: &str) -> bool {
    !s.is_empty() && s.len() <= 20 && s.chars().all(|c| c.is_ascii_digit())
}

/// `snowflakes(value, max, label)`: array of numeric IDs (numbers allowed).
fn snowflakes(value: &Value, max: usize, label: &str) -> FnResult<Vec<String>> {
    let ids = match value {
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::Number(n) if n.as_f64().is_some_and(|v| v.fract() == 0.0) => {
                    Some(n.to_string())
                }
                Value::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect::<Option<Vec<_>>>(),
        _ => None,
    };
    match ids {
        Some(ids)
            if ids.len() <= max
                && ids
                    .iter()
                    .all(|id| is_snowflake_text(id) && id.parse::<u64>().is_ok()) =>
        {
            Ok(ids)
        }
        _ => fail(
            "MESSAGE_OPTION",
            format!("{label} must be an array of up to {max} numeric IDs."),
        ),
    }
}

fn embed_list(values: &[Value]) -> FnResult<Vec<Value>> {
    let embeds = values
        .iter()
        .map(normalize_embed)
        .collect::<FnResult<Vec<_>>>()?;
    assert_embed_count(embeds.len())?;
    Ok(embeds)
}

/// `payload(c, value)`: the send payload for `$reply`/`$sendMessage`/`$editMessage`.
pub async fn payload(c: &ScriptCall<'_>, value: &Value) -> FnResult<MessageSend> {
    if let Value::Object(map) = value {
        if map.keys().any(|k| MESSAGE_KEYS.contains(&k.as_str())) {
            return message_payload(c, map).await;
        }
        return Ok(MessageSend {
            embeds: vec![embed_to_wire(&normalize_embed(value)?).unwrap_or_default()],
            ..Default::default()
        });
    }
    if let Value::Array(items) = value {
        if !items.is_empty() && items.iter().all(|i| i.is_object()) {
            return Ok(MessageSend {
                embeds: embed_list(items)?
                    .iter()
                    .filter_map(embed_to_wire)
                    .collect(),
                ..Default::default()
            });
        }
    }
    let content = text(value);
    if content.is_empty() || content.chars().count() > MESSAGE_LENGTH {
        return fail(
            "MESSAGE_LENGTH",
            "The message must have 1 to 2000 characters.",
        );
    }
    Ok(MessageSend {
        content: Some(content),
        ..Default::default()
    })
}

async fn message_payload(c: &ScriptCall<'_>, map: &Map<String, Value>) -> FnResult<MessageSend> {
    for key in map.keys() {
        if !MESSAGE_KEYS.contains(&key.as_str()) {
            return fail("MESSAGE_KEY", format!("Unknown message field: {key}."));
        }
    }
    let mut send = MessageSend::default();
    let content = map
        .get("content")
        .filter(|v| !v.is_null())
        .map(text)
        .unwrap_or_default();
    if content.chars().count() > MESSAGE_LENGTH {
        return fail(
            "MESSAGE_LENGTH",
            "The message must have at most 2000 characters.",
        );
    }
    if !content.is_empty() {
        send.content = Some(content);
    }
    if let Some(embeds) = map.get("embeds").filter(|v| !v.is_null()) {
        let items: Vec<Value> = match embeds {
            Value::Array(list) => list.clone(),
            single => vec![single.clone()],
        };
        send.embeds = embed_list(&items)?
            .iter()
            .filter_map(embed_to_wire)
            .collect();
    }
    if send.content.is_none() && send.embeds.is_empty() {
        return fail("MESSAGE_LENGTH", "The message needs text or an embed.");
    }
    let options = message_options(c, map).await?;
    if let Some(v) = options.tts {
        send.tts = Some(v);
    }
    send.sticker_ids = options.sticker_ids;
    if let Some(v) = options.flags {
        send.flags = Some(v);
    }
    send.allowed_mentions = options.allowed_mentions;
    Ok(send)
}

/// `messageOptions`: tts (requires author `SendTtsMessages`), stickerIds (max
/// 3), flags (32-bit non-negative), allowedMentions (relaxing it requires
/// author `MentionEveryone`).
async fn message_options(c: &ScriptCall<'_>, map: &Map<String, Value>) -> FnResult<MessageSend> {
    let mut send = MessageSend::default();
    if let Some(tts) = map.get("tts") {
        let Value::Bool(v) = tts else {
            return fail("MESSAGE_OPTION", "tts must be true or false.");
        };
        if *v {
            require_permission(c, "SendTtsMessages", false, None).await?;
        }
        send.tts = Some(*v);
    }
    if let Some(v) = map.get("stickerIds") {
        send.sticker_ids = snowflakes(v, MAX_STICKERS, "stickerIds")?
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
    }
    if let Some(flags) = map.get("flags") {
        let valid = flags
            .as_f64()
            .is_some_and(|v| v.fract() == 0.0 && (0.0..=0x7FFFFFFF as f64).contains(&v));
        if !valid {
            return fail(
                "MESSAGE_OPTION",
                "flags must be a non-negative 32-bit integer.",
            );
        }
        send.flags = Some(flags.as_f64().unwrap_or(0.0) as u64);
    }
    if let Some(v) = map.get("allowedMentions") {
        send.allowed_mentions = Some(mention_policy(c, v).await?);
    }
    Ok(send)
}

async fn mention_policy(c: &ScriptCall<'_>, value: &Value) -> FnResult<AllowedMentions> {
    let Value::Object(map) = value else {
        return fail("MESSAGE_OPTION", "allowedMentions must be an object.");
    };
    for key in map.keys() {
        if !["parse", "users", "roles", "repliedUser"].contains(&key.as_str()) {
            return fail(
                "MESSAGE_OPTION",
                format!("Unknown allowedMentions field: {key}."),
            );
        }
    }
    let mut policy = AllowedMentions::default();
    if let Some(parse) = map.get("parse") {
        let ok = matches!(parse, Value::Array(items)
            if items.iter().all(|i| i.as_str().is_some_and(|s| MENTION_PARSE.contains(&s))));
        if !ok {
            return fail(
                "MESSAGE_OPTION",
                "parse only accepts users, roles and everyone.",
            );
        }
        let mut seen = Vec::new();
        for item in parse.as_array().unwrap() {
            let s = item.as_str().unwrap().to_string();
            if !seen.contains(&s) {
                seen.push(s);
            }
        }
        policy.parse = Some(seen);
    }
    for key in ["users", "roles"] {
        if let Some(v) = map.get(key) {
            let ids = snowflakes(v, MAX_MENTION_IDS, key)?;
            let parsed: Vec<_> = ids.iter().map(|s| s.parse().unwrap()).collect();
            if key == "users" {
                policy.users = Some(parsed);
            } else {
                policy.roles = Some(parsed);
            }
        }
    }
    if let Some(v) = map.get("repliedUser") {
        let Value::Bool(b) = v else {
            return fail("MESSAGE_OPTION", "repliedUser must be true or false.");
        };
        policy.replied_user = Some(*b);
    }
    let relaxes = policy.parse.as_ref().is_some_and(|p| !p.is_empty())
        || policy.users.as_ref().is_some_and(|u| !u.is_empty())
        || policy.roles.as_ref().is_some_and(|r| !r.is_empty())
        || policy.replied_user == Some(true);
    if relaxes {
        require_permission(c, "MentionEveryone", false, None).await?;
    }
    Ok(policy)
}

/// No-ping mention policy applied when the payload carries none
/// (`transport.ts` defaults).
pub fn default_mentions(send: &mut MessageSend) {
    if send.allowed_mentions.is_none() {
        send.allowed_mentions = Some(AllowedMentions::no_ping());
    }
}

pub fn register(d: &mut Definitions) {
    d.add(
        "embeds",
        "embeds",
        1,
        EMBEDS_PER_MESSAGE,
        "[embed;embed;...]",
        "Builds a list of up to 10 validated embeds, for $reply, $sendMessage or $editMessage.",
        |c| {
            Box::pin(async move {
                let mut values = Vec::new();
                for i in 0..c.arg_count() {
                    values.push(arg_value(c, i).await?);
                }
                Ok(Value::Array(embed_list(&values)?))
            })
        },
    );

    d.add(
        "addembed",
        "embeds",
        2,
        2,
        "[list;embed]",
        "Returns a new embed list with the embed appended, respecting the limit of 10.",
        |c| {
            Box::pin(async move {
                let current = arg_value(c, 0).await?;
                let mut list = match current {
                    Value::Array(items) => items,
                    single => vec![single],
                };
                list.push(arg_value(c, 1).await?);
                Ok(Value::Array(embed_list(&list)?))
            })
        },
    );

    d.add(
        "messageoptions",
        "messages",
        1,
        3,
        "[tts?;stickerIds?;flags?]",
        "Send options to merge with $jsonMerge and a payload with content or embeds. TTS requires SendTtsMessages; mentions, MentionEveryone.",
        |c| {
            Box::pin(async move {
                let mut map = Map::new();
                let tts = c.arg(0).await?;
                if !tts.trim().is_empty() {
                    map.insert("tts".into(), Value::Bool(tts.trim().eq_ignore_ascii_case("true")));
                }
                if c.arg_count() > 1 {
                    let raw = c.arg(1).await?;
                    if !raw.trim().is_empty() {
                        let ids: Vec<Value> = raw
                            .split(',')
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .map(|s| Value::String(s.to_string()))
                            .collect();
                        map.insert("stickerIds".into(), Value::Array(ids));
                    }
                }
                if c.arg_count() > 2 {
                    let raw = c.arg(2).await?;
                    if !raw.trim().is_empty() {
                        let n = raw.trim().parse::<f64>().unwrap_or(f64::NAN);
                        map.insert("flags".into(), serde_json::json!(n));
                    }
                }
                let send = message_options(c, &map).await?;
                let mut out = Map::new();
                if let Some(v) = send.tts {
                    out.insert("tts".into(), Value::Bool(v));
                }
                if !send.sticker_ids.is_empty() {
                    out.insert(
                        "stickerIds".into(),
                        Value::Array(send.sticker_ids.iter().map(|s| Value::String(s.to_string())).collect()),
                    );
                }
                if let Some(v) = send.flags {
                    out.insert("flags".into(), serde_json::json!(v));
                }
                if let Some(m) = &send.allowed_mentions {
                    let mut policy = Map::new();
                    if let Some(p) = &m.parse {
                        policy.insert("parse".into(), serde_json::json!(p));
                    }
                    if let Some(u) = &m.users {
                        policy.insert(
                            "users".into(),
                            Value::Array(u.iter().map(|s| Value::String(s.to_string())).collect()),
                        );
                    }
                    if let Some(r) = &m.roles {
                        policy.insert(
                            "roles".into(),
                            Value::Array(r.iter().map(|s| Value::String(s.to_string())).collect()),
                        );
                    }
                    if let Some(v) = m.replied_user {
                        policy.insert("repliedUser".into(), Value::Bool(v));
                    }
                    out.insert("allowedMentions".into(), Value::Object(policy));
                }
                Ok(Value::Object(out))
            })
        },
    );
}
