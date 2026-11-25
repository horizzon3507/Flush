//! `resource-extras.ts`: dm/forward/search, attachments, member/guild/thread
//! edits, invites and expression clones.

use serde_json::{json, Map, Value};

use flush_core::{AllowedMentions, MessageSend};

use crate::call::ScriptCall;
use crate::errors::{FnError, FnResult, ScriptFailure};
use crate::types::Definitions;

use super::{arg_value, number_arg, permissions, required, snowflake, user_id};

fn channel_json(ch: &flush_core::Channel) -> Value {
    serde_json::to_value(ch).unwrap_or(Value::Null)
}

fn fail<T>(code: &'static str, message: impl Into<String>) -> FnResult<T> {
    Err(FnError::new(code, message).into())
}

fn object<'a>(value: &'a Value, keys: &[&str]) -> FnResult<&'a Map<String, Value>> {
    let Value::Object(map) = value else {
        return fail("OPTIONS", "Options must be an object.");
    };
    if map.keys().any(|k| !keys.contains(&k.as_str())) {
        return fail("OPTIONS", "Unknown options property.");
    }
    Ok(map)
}

const BASE64_RE: &str = r"^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$";

fn base64_decode(raw: &str) -> FnResult<Vec<u8>> {
    let valid = regex::Regex::new(BASE64_RE).expect("regex").is_match(raw);
    if raw.is_empty() || !valid {
        return fail(
            "ATTACHMENT_BASE64",
            "Enter valid base64, without a data: prefix.",
        );
    }
    let mut table = [0u8; 256];
    for (i, b) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
        .iter()
        .enumerate()
    {
        table[*b as usize] = i as u8;
    }
    let clean: Vec<u8> = raw.bytes().filter(|b| *b != b'=').collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut acc = 0u32;
        for (i, b) in chunk.iter().enumerate() {
            acc |= (table[*b as usize] as u32) << (18 - 6 * i);
        }
        out.push((acc >> 16) as u8);
        if chunk.len() > 2 {
            out.push((acc >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(acc as u8);
        }
    }
    Ok(out)
}

/// Validated attachment spec: `{url|base64, name, description?, spoiler?}`.
struct Attachment {
    name: String,
    description: Option<String>,
    spoiler: bool,
    url: Option<String>,
    data: Option<Vec<u8>>,
}

fn attachment(value: &Value) -> FnResult<Attachment> {
    let file = object(value, &["url", "base64", "name", "description", "spoiler"])?;
    let name = match file.get("name") {
        Some(Value::String(s)) if !s.trim().is_empty() && s.chars().count() <= 255 => s.clone(),
        _ => {
            return fail(
                "ATTACHMENT_NAME",
                "An attachment name must have 1 to 255 characters.",
            )
        }
    };
    let description = match file.get("description") {
        Some(Value::String(s)) => Some(s.clone()),
        None => None,
        _ => return fail("ATTACHMENT_DESCRIPTION", "The description must be text."),
    };
    let spoiler = match file.get("spoiler") {
        Some(Value::Bool(b)) => *b,
        None => false,
        _ => return fail("ATTACHMENT", "spoiler must be a boolean."),
    };
    let has_url = file.get("url").is_some();
    let has_b64 = file.get("base64").is_some();
    if has_url == has_b64 {
        return fail("ATTACHMENT", "Enter only url or base64 per attachment.");
    }
    if let Some(Value::String(b64)) = file.get("base64") {
        return Ok(Attachment {
            name,
            description,
            spoiler,
            url: None,
            data: Some(base64_decode(b64)?),
        });
    }
    match file.get("url") {
        Some(Value::String(u))
            if u.len() >= 8 && u[..8].eq_ignore_ascii_case("https://")
                || u.len() >= 7 && u[..7].eq_ignore_ascii_case("http://") =>
        {
            Ok(Attachment {
                name,
                description,
                spoiler,
                url: Some(u.clone()),
                data: None,
            })
        }
        _ => fail("ATTACHMENT_URL", "The URL must be HTTP/HTTPS."),
    }
}

/// `messageSearchInput` from `domain/message-query.ts`.
pub fn message_search_input(raw: &Value) -> FnResult<Map<String, Value>> {
    const SEARCH_KEYS: &[&str] = &[
        "content",
        "contents",
        "exactPhrases",
        "guildID",
        "channelID",
        "channelIDs",
        "excludeChannelIDs",
        "authorID",
        "authorIDs",
        "excludeAuthorIDs",
        "authorType",
        "excludeAuthorType",
        "mentions",
        "excludeMentions",
        "mentionEveryone",
        "pinned",
        "includeNsfw",
        "has",
        "excludeHas",
        "minID",
        "maxID",
        "sortBy",
        "sortOrder",
        "limit",
        "page",
        "cursor",
    ];
    let input = object_named(raw, SEARCH_KEYS)?;
    let mut result = Map::new();
    for (key, value) in input {
        if value.is_null() {
            continue;
        }
        match key.as_str() {
            k @ ("guildID" | "channelID" | "authorID" | "minID" | "maxID") => {
                result.insert(k.into(), Value::String(search_id(value, k)?));
            }
            k @ ("mentionEveryone" | "pinned" | "includeNsfw") => {
                let Value::Bool(b) = value else {
                    return fail("MESSAGE_OPTIONS", format!("{k} must be a boolean."));
                };
                result.insert(k.into(), Value::Bool(*b));
            }
            k @ ("limit" | "page") => {
                let (min, max) = if k == "limit" { (1, 100) } else { (0, 9999) };
                result.insert(k.into(), json!(search_integer(value, k, min, max)?));
            }
            k @ ("content" | "sortBy" | "sortOrder") => {
                let max = if k == "content" { 2000 } else { 100 };
                let v = search_text(value, k, max)?;
                if k == "sortOrder" && v != "asc" && v != "desc" {
                    return fail("MESSAGE_OPTIONS", "sortOrder must be asc or desc.");
                }
                result.insert(k.into(), Value::String(v));
            }
            k => {
                let id_keys = [
                    "channelIDs",
                    "excludeChannelIDs",
                    "authorIDs",
                    "excludeAuthorIDs",
                    "mentions",
                    "excludeMentions",
                ];
                let is_id = id_keys.contains(&k);
                let max_items = if is_id {
                    100
                } else if k == "cursor" {
                    10
                } else {
                    20
                };
                let min_items = if k == "cursor" { 0 } else { 1 };
                let Value::Array(items) = value else {
                    return fail(
                        "MESSAGE_OPTIONS",
                        format!("{k} must be an array of {min_items} to {max_items} items."),
                    );
                };
                if items.len() < min_items || items.len() > max_items {
                    return fail(
                        "MESSAGE_OPTIONS",
                        format!("{k} must be an array of {min_items} to {max_items} items."),
                    );
                }
                let mut out = Vec::new();
                for item in items {
                    if is_id {
                        out.push(Value::String(search_id(item, k)?));
                    } else {
                        let max = if ["contents", "exactPhrases", "cursor"].contains(&k) {
                            2000
                        } else {
                            100
                        };
                        let t = search_text(item, k, max)?;
                        if (k == "authorType" || k == "excludeAuthorType")
                            && !["user", "bot", "webhook"].contains(&t.as_str())
                        {
                            return fail(
                                "MESSAGE_OPTIONS",
                                format!("{k} accepts user, bot or webhook."),
                            );
                        }
                        out.push(Value::String(t));
                    }
                }
                result.insert(k.into(), Value::Array(out));
            }
        }
    }
    if result.contains_key("authorID") && result.contains_key("authorIDs") {
        return fail("MESSAGE_OPTIONS", "Use only authorID or authorIDs.");
    }
    if result.contains_key("page") && result.contains_key("cursor") {
        return fail("MESSAGE_CURSOR", "Use only page or cursor in the search.");
    }
    if let (Some(Value::String(min)), Some(Value::String(max))) =
        (result.get("minID"), result.get("maxID"))
    {
        if min.parse::<u64>().unwrap_or(0) > max.parse::<u64>().unwrap_or(u64::MAX) {
            return fail("MESSAGE_OPTIONS", "minID cannot be greater than maxID.");
        }
    }
    if serde_json::to_string(&result).map(|s| s.len()).unwrap_or(0) > 32_768 {
        return fail("MESSAGE_OPTIONS", "Search options exceed 32 KiB.");
    }
    Ok(result)
}

fn object_named<'a>(value: &'a Value, keys: &[&str]) -> FnResult<&'a Map<String, Value>> {
    let Value::Object(map) = value else {
        return fail("MESSAGE_OPTIONS", "Message options must be a plain object.");
    };
    if map.keys().any(|k| !keys.contains(&k.as_str())) {
        return fail("MESSAGE_OPTIONS", "Unknown message options property.");
    }
    Ok(map)
}

fn search_id(value: &Value, key: &str) -> FnResult<String> {
    match value {
        Value::String(s)
            if !s.is_empty() && s.len() <= 20 && s.chars().all(|c| c.is_ascii_digit()) =>
        {
            Ok(s.clone())
        }
        _ => fail(
            "MESSAGE_OPTIONS",
            format!("{key} must be a numeric ID of up to 20 digits."),
        ),
    }
}

fn search_text(value: &Value, key: &str, max: usize) -> FnResult<String> {
    match value {
        Value::String(s) if !s.trim().is_empty() && s.chars().count() <= max => Ok(s.clone()),
        _ => fail(
            "MESSAGE_OPTIONS",
            format!("{key} must be text of 1 to {max} characters."),
        ),
    }
}

fn search_integer(value: &Value, key: &str, min: i64, max: i64) -> FnResult<i64> {
    match value.as_f64() {
        Some(v) if v.fract() == 0.0 && v >= min as f64 && v <= max as f64 => Ok(v as i64),
        _ => fail(
            "MESSAGE_OPTIONS",
            format!("{key} must be an integer between {min} and {max}."),
        ),
    }
}

/// Maps `MessageSearchInput` to the REST body (`infrastructure/fluxer/messages.ts`).
fn search_body(input: &Map<String, Value>) -> Value {
    let mut body = Map::new();
    let copy = |k: &str, wire: &str, body: &mut Map<String, Value>| {
        if let Some(v) = input.get(k) {
            body.insert(wire.into(), v.clone());
        }
    };
    copy("content", "content", &mut body);
    if let Some(v) = input.get("guildID") {
        body.insert("contextGuildId".into(), v.clone());
    }
    if let Some(v) = input.get("channelID") {
        body.insert("contextChannelId".into(), v.clone());
        if !input.contains_key("channelIDs") {
            body.insert("channelIds".into(), json!([v]));
        }
    }
    if let Some(v) = input.get("channelIDs") {
        body.insert("channelIds".into(), v.clone());
    }
    if let Some(v) = input.get("authorID") {
        if !input.contains_key("authorIDs") {
            body.insert("authorIds".into(), json!([v]));
        }
    }
    if let Some(v) = input.get("authorIDs") {
        body.insert("authorIds".into(), v.clone());
    }
    let limit = input
        .get("limit")
        .and_then(|v| v.as_i64())
        .map(|v| v.clamp(25, 100))
        .unwrap_or(25);
    body.insert("hitsPerPage".into(), json!(limit.max(25)));
    copy("cursor", "cursor", &mut body);
    for (k, wire) in [
        ("contents", "contents"),
        ("exactPhrases", "exactPhrases"),
        ("excludeChannelIDs", "excludeChannelIds"),
        ("excludeAuthorIDs", "excludeAuthorIds"),
        ("authorType", "authorType"),
        ("excludeAuthorType", "excludeAuthorType"),
        ("mentions", "mentions"),
        ("excludeMentions", "excludeMentions"),
        ("mentionEveryone", "mentionEveryone"),
        ("pinned", "pinned"),
        ("includeNsfw", "includeNsfw"),
        ("has", "has"),
        ("excludeHas", "excludeHas"),
        ("minID", "minId"),
        ("maxID", "maxId"),
        ("sortBy", "sortBy"),
        ("sortOrder", "sortOrder"),
        ("page", "page"),
    ] {
        copy(k, wire, &mut body);
    }
    Value::Object(body)
}

async fn run_search(c: &ScriptCall<'_>, input: Map<String, Value>) -> FnResult<Value> {
    let body = search_body(&input);
    let rest = c.ctx.rest.clone();
    let result = c
        .wait(async move {
            rest.search_messages(body)
                .await
                .map_err(ScriptFailure::from)
        })
        .await?;
    if result.get("indexing").is_some() {
        return Ok(json!({ "indexing": true, "messages": [], "total": 0, "cursor": Value::Null }));
    }
    Ok(json!({
        "indexing": false,
        "messages": result.get("messages").cloned().unwrap_or(json!([])),
        "total": result.get("total").cloned().unwrap_or(json!(0)),
        "cursor": result.get("cursor").cloned().unwrap_or(Value::Null),
    }))
}

async fn send_files(
    c: &ScriptCall<'_>,
    channel: flush_core::Snowflake,
    files: Vec<Attachment>,
    content: Option<String>,
) -> FnResult<Value> {
    if files.is_empty() || files.len() > 10 {
        return fail("ATTACHMENTS", "Send an array of 1 to 10 attachments.");
    }
    let total: usize = files
        .iter()
        .filter_map(|f| f.data.as_ref().map(|d| d.len()))
        .sum();
    if total > 10_000_000 {
        return fail(
            "ATTACHMENTS",
            "Send 1 to 10 attachments, with up to 10 MB of bytes in total.",
        );
    }
    let mut uploads = Vec::new();
    for file in files {
        let name = if file.spoiler && !file.name.starts_with("SPOILER_") {
            format!("SPOILER_{}", file.name)
        } else {
            file.name
        };
        let data = match file.data {
            Some(d) => d,
            None => {
                let url = file.url.unwrap_or_default();
                let mut response = c
                    .wait(async move {
                        surf::get(&url)
                            .await
                            .map_err(|e| FnError::new("ATTACHMENT_URL", e.to_string()).into())
                    })
                    .await?;
                c.wait(async move {
                    response
                        .body_bytes()
                        .await
                        .map_err(|e| FnError::new("ATTACHMENT_URL", e.to_string()).into())
                })
                .await?
            }
        };
        let mut upload = flush_rest::FileUpload::new(name, data);
        if let Some(d) = file.description {
            upload = upload.description(d);
        }
        uploads.push(upload);
    }
    let send = MessageSend {
        content,
        allowed_mentions: Some(AllowedMentions::no_ping()),
        ..Default::default()
    };
    let rest = c.ctx.rest.clone();
    let sent = c
        .wait(async move {
            rest.send_message_with_files(channel, &send, &uploads)
                .await
                .map_err(ScriptFailure::from)
        })
        .await?;
    Ok(Value::String(sent.id.to_string()))
}

pub fn register(d: &mut Definitions) {
    d.add(
        "dm",
        "messages",
        2,
        2,
        "[user;content]",
        "Sends a direct message and returns the ID; does not mention users automatically.",
        |c| {
            Box::pin(async move {
                let user = user_id(&c.arg(0).await?)?;
                let content = required(c, 1, "Content").await?;
                let rest = c.ctx.rest.clone();
                let channel = c
                    .wait(async move { rest.create_dm(user).await.map_err(ScriptFailure::from) })
                    .await?;
                let send = MessageSend {
                    content: Some(content),
                    allowed_mentions: Some(AllowedMentions::no_ping()),
                    ..Default::default()
                };
                let rest = c.ctx.rest.clone();
                let sent = c
                    .wait(async move {
                        rest.send_message(channel.id, &send)
                            .await
                            .map_err(ScriptFailure::from)
                    })
                    .await?;
                Ok(Value::String(sent.id.to_string()))
            })
        },
    );

    d.add(
        "forward",
        "messages",
        3,
        3,
        "[target;sourceChannel;message]",
        "Forwards a message to another channel and returns the new ID.",
        |c| {
            Box::pin(async move {
                let target = snowflake(&required(c, 0, "Target").await?)?;
                let source = snowflake(&required(c, 1, "Source channel").await?)?;
                let message = snowflake(&required(c, 2, "Message").await?)?;
                let body = json!({
                    "forward": {
                        "channelId": source.to_string(),
                        "messageId": message.to_string(),
                    },
                    "allowed_mentions": { "parse": [], "replied_user": false },
                });
                let rest = c.ctx.rest.clone();
                let sent: Value = c
                    .wait(async move {
                        rest.raw(
                            "POST",
                            &format!("/channels/{target}/messages"),
                            Some(body),
                            None,
                            None,
                        )
                        .await
                        .map_err(ScriptFailure::from)
                    })
                    .await?;
                Ok(Value::String(
                    sent.get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                ))
            })
        },
    );

    d.add(
        "searchmessages",
        "messages",
        1,
        5,
        "[text;channel?;limit?;cursor?;author?]",
        "Searches messages; returns indexing, messages, total and cursor. Cursor is the array returned by the previous page.",
        |c| {
            Box::pin(async move {
                let content = required(c, 0, "Text").await?;
                let mut input = Map::new();
                input.insert("content".into(), Value::String(content));
                if c.arg_count() > 1 {
                    let channel = c.arg(1).await?.trim().to_string();
                    if !channel.is_empty() {
                        input.insert("channelID".into(), Value::String(channel));
                    }
                }
                let limit = number_arg(c, 2, 25, 1, 100).await?;
                input.insert("limit".into(), json!(limit));
                let guild = c.ctx.guild_id();
                if guild.get() != 0 {
                    input.insert("guildID".into(), Value::String(guild.to_string()));
                }
                if c.arg_count() > 3 {
                    if let Value::Array(items) = arg_value(c, 3).await? {
                        if items.iter().all(|i| i.is_string()) {
                            input.insert("cursor".into(), Value::Array(items));
                        } else {
                            return fail("MESSAGE_CURSOR", "The search cursor must be an array of texts.");
                        }
                    }
                }
                if c.arg_count() > 4 {
                    let author = c.arg(4).await?.trim().to_string();
                    if !author.is_empty() {
                        input.insert("authorID".into(), Value::String(user_id(&author)?.to_string()));
                    }
                }
                let input = message_search_input(&Value::Object(input))?;
                run_search(c, input).await
            })
        },
    );

    d.add(
        "searchmessagesadvanced",
        "messages",
        1,
        1,
        "[options]",
        "Searches by content, ID, author, mention, attachment, date and order filters; text is optional. Up to 100 results, options of up to 32 KiB; page or cursor are exclusive.",
        |c| {
            Box::pin(async move {
                let input = message_search_input(&arg_value(c, 0).await?)?;
                if input.contains_key("guildID") {
                    return fail("MESSAGE_OPTIONS", "guildID is set by the command's context.");
                }
                let mut input = input;
                let guild = c.ctx.guild_id();
                if guild.get() != 0 {
                    input.insert("guildID".into(), Value::String(guild.to_string()));
                } else if !input.contains_key("channelID") {
                    input.insert(
                        "channelID".into(),
                        Value::String(c.ctx.channel_id().to_string()),
                    );
                }
                run_search(c, input).await
            })
        },
    );

    d.add(
        "filebase64",
        "messages",
        2,
        4,
        "[base64;name;description?;spoiler?]",
        "Builds a base64 attachment for $sendAttachments; spoiler accepts true/false, without sending.",
        |c| {
            Box::pin(async move {
                let spoiler = if c.arg_count() > 3 {
                    let s = c.arg(3).await?;
                    if s != "true" && s != "false" {
                        return fail("ATTACHMENT", "spoiler accepts true or false.");
                    }
                    Some(Value::Bool(s == "true"))
                } else {
                    None
                };
                let mut file = Map::new();
                file.insert("base64".into(), Value::String(c.arg(0).await?));
                file.insert("name".into(), Value::String(c.arg(1).await?));
                if c.arg_count() > 2 {
                    file.insert("description".into(), Value::String(c.arg(2).await?));
                }
                if let Some(s) = spoiler {
                    file.insert("spoiler".into(), s);
                }
                let _ = attachment(&Value::Object(file.clone()))?;
                Ok(Value::Object(file))
            })
        },
    );

    d.add(
        "sendfilebase64",
        "messages",
        3,
        4,
        "[channel;base64;name;content?]",
        "Decodes base64 and sends a binary attachment, returning the ID.",
        |c| {
            Box::pin(async move {
                let channel = snowflake(&required(c, 0, "Channel").await?)?;
                let data = base64_decode(&c.arg(1).await?)?;
                let name = required(c, 2, "Name").await?;
                let content = if c.arg_count() > 3 {
                    Some(c.arg(3).await?)
                } else {
                    None
                };
                let file = Attachment {
                    name,
                    description: None,
                    spoiler: false,
                    url: None,
                    data: Some(data),
                };
                send_files(c, channel, vec![file], content).await
            })
        },
    );

    d.add(
        "sendattachments",
        "messages",
        2,
        3,
        "[channel;array;content?]",
        "Sends 1 to 10 URL/base64 attachments in the same message, with up to 10 MB of bytes.",
        |c| {
            Box::pin(async move {
                let channel = snowflake(&required(c, 0, "Channel").await?)?;
                let Value::Array(items) = arg_value(c, 1).await? else {
                    return fail("ATTACHMENTS", "Send an array of 1 to 10 attachments.");
                };
                if items.is_empty() || items.len() > 10 {
                    return fail("ATTACHMENTS", "Send an array of 1 to 10 attachments.");
                }
                let files = items.iter().map(attachment).collect::<FnResult<Vec<_>>>()?;
                let content = if c.arg_count() > 2 {
                    Some(c.arg(2).await?)
                } else {
                    None
                };
                send_files(c, channel, files, content).await
            })
        },
    );

    d.add(
        "searchmembers",
        "users",
        1,
        3,
        "[name;limit?;offset?]",
        "Searches members by name; returns members, total, hasMore and indexing.",
        |c| {
            Box::pin(async move {
                let name = required(c, 0, "Name").await?;
                let limit = number_arg(c, 1, 25, 1, 100).await? as u32;
                let offset = number_arg(c, 2, 0, 0, i64::MAX).await? as u32;
                let guild = c.ctx.require_guild()?;
                let search = flush_rest::MemberSearch {
                    query: Some(name),
                    limit: Some(limit),
                    offset: Some(offset),
                    ..Default::default()
                };
                let rest = c.ctx.rest.clone();
                c.wait(async move {
                    rest.search_members(guild, &search)
                        .await
                        .map_err(ScriptFailure::from)
                })
                .await
            })
        },
    );

    d.add(
        "nickname",
        "users",
        3,
        3,
        "[user;nickname;reason]",
        "Changes a nickname with permissions and hierarchy; empty removes it. Returns the member.",
        |c| {
            Box::pin(async move {
                let guild = permissions(c, "ManageNicknames").await?;
                let target = user_id(&c.arg(0).await?)?;
                let nick = c.arg(1).await?;
                let reason = c.arg(2).await?;
                let rest = c.ctx.rest.clone();
                let member = c
                    .wait(async move {
                        rest.member(guild, target)
                            .await
                            .map_err(ScriptFailure::from)
                    })
                    .await?;
                // Hierarchy: author and bot must outrank the target (owner exempt).
                let guild_obj = match &c.ctx.guild {
                    Some(g) if g.id == guild => Some(g.clone()),
                    _ => None,
                };
                if let Some(g) = guild_obj {
                    if g.owner_id != target {
                        let top = g.highest_role_position(&member);
                        for who in [c.ctx.author_id(), c.ctx.bot_id.unwrap_or_default()] {
                            if who.get() == 0 || g.owner_id == who {
                                continue;
                            }
                            let member_of = if who == c.ctx.author_id() {
                                c.ctx.member.clone()
                            } else {
                                c.ctx.bot_member.clone()
                            };
                            let theirs =
                                member_of.map(|m| g.highest_role_position(&m)).unwrap_or(0);
                            if theirs <= top {
                                return Err(crate::errors::GuardError::new(
                                    "Hierarchy prevents the nickname change.",
                                    crate::errors::GuardReason::Hierarchy,
                                )
                                .into());
                            }
                        }
                    }
                }
                let patch = flush_rest::MemberPatch {
                    nick: Some(if nick.is_empty() { None } else { Some(nick) }),
                    ..Default::default()
                };
                let reason = if reason.is_empty() {
                    None
                } else {
                    Some(reason.as_str())
                };
                let rest = c.ctx.rest.clone();
                let updated = c
                    .wait(async move {
                        rest.patch_member(guild, target, &patch, reason)
                            .await
                            .map_err(ScriptFailure::from)
                    })
                    .await?;
                Ok(serde_json::to_value(updated).unwrap_or(Value::Null))
            })
        },
    );

    d.add(
        "editguild",
        "users",
        1,
        1,
        "[options]",
        "Edits name, icon, banner, system/AFK channel and AFK timeout; requires ManageGuild from author and bot.",
        |c| {
            Box::pin(async move {
                let guild = permissions(c, "ManageGuild").await?;
                let value = arg_value(c, 0).await?;
                let map = object(
                    &value,
                    &["name", "icon", "banner", "systemChannelID", "afkChannelID", "afkTimeout"],
                )?;
                let mut patch = flush_rest::GuildPatch::default();
                for (key, value) in map {
                    match key.as_str() {
                        "afkTimeout" => {
                            let Some(v) = value.as_i64() else {
                                return fail("GUILD_OPTIONS", "afkTimeout must be an integer from 0.");
                            };
                            if v < 0 {
                                return fail("GUILD_OPTIONS", "afkTimeout must be an integer from 0.");
                            }
                            patch.afk_timeout = Some(v as u32);
                        }
                        "name" => {
                            let Some(v) = value.as_str() else {
                                return fail("GUILD_OPTIONS", "name must be text.");
                            };
                            if v.trim().is_empty() || v.chars().count() > 100 {
                                return fail("GUILD_OPTIONS", "The name must have 1 to 100 characters.");
                            }
                            patch.name = Some(v.to_string());
                        }
                        key => {
                            let opt = match value {
                                Value::Null => Some(None),
                                Value::String(s) => Some(Some(s.clone())),
                                _ => {
                                    return fail("GUILD_OPTIONS", format!("{key} must be text or null."));
                                }
                            };
                            match key {
                                "icon" => patch.icon = opt,
                                "banner" => patch.banner = opt,
                                "systemChannelID" => {
                                    patch.system_channel_id =
                                        opt.map(|v| v.and_then(|s| s.parse().ok()));
                                }
                                "afkChannelID" => {
                                    patch.afk_channel_id =
                                        opt.map(|v| v.and_then(|s| s.parse().ok()));
                                }
                                _ => {}
                            }
                        }
                    }
                }
                let rest = c.ctx.rest.clone();
                let updated = c
                    .wait(async move {
                        rest.edit_guild(guild, &patch, None)
                            .await
                            .map_err(ScriptFailure::from)
                    })
                    .await?;
                Ok(serde_json::to_value(updated).unwrap_or(Value::Null))
            })
        },
    );

    d.add(
        "editthread",
        "threads",
        2,
        2,
        "[thread;options]",
        "Edits name, archive, lock, duration, slowmode and tags; requires ManageChannels.",
        |c| {
            Box::pin(async move {
                permissions(c, "ManageChannels").await?;
                let thread = snowflake(&required(c, 0, "Thread").await?)?;
                let value = arg_value(c, 1).await?;
                let map = object(
                    &value,
                    &[
                        "name",
                        "archived",
                        "locked",
                        "autoArchiveDuration",
                        "slowmode",
                        "tags",
                        "reason",
                    ],
                )?;
                let mut body = Map::new();
                let mut audit = None;
                for (key, value) in map {
                    match key.as_str() {
                        k @ ("archived" | "locked") => {
                            let Value::Bool(b) = value else {
                                return fail("THREAD_OPTIONS", format!("{k} must be a boolean."));
                            };
                            body.insert(k.into(), Value::Bool(*b));
                        }
                        "name" => {
                            let Value::String(s) = value else {
                                return fail("THREAD_OPTIONS", "name must be text.");
                            };
                            if s.trim().is_empty() || s.chars().count() > 100 {
                                return fail(
                                    "THREAD_OPTIONS",
                                    "The name must have 1 to 100 characters.",
                                );
                            }
                            body.insert("name".into(), Value::String(s.clone()));
                        }
                        "reason" => {
                            let Value::String(s) = value else {
                                return fail("THREAD_OPTIONS", "reason must be text.");
                            };
                            audit = Some(s.clone());
                        }
                        "autoArchiveDuration" => {
                            let Some(v) = value.as_i64() else {
                                return fail("THREAD_OPTIONS", "Invalid archive duration.");
                            };
                            if ![60, 1440, 4320, 10080].contains(&v) {
                                return fail("THREAD_OPTIONS", "Invalid archive duration.");
                            }
                            body.insert("auto_archive_duration".into(), json!(v));
                        }
                        "slowmode" => {
                            let Some(v) = value.as_i64() else {
                                return fail(
                                    "THREAD_OPTIONS",
                                    "Slowmode must be between 0 and 21600.",
                                );
                            };
                            if !(0..=21600).contains(&v) {
                                return fail(
                                    "THREAD_OPTIONS",
                                    "Slowmode must be between 0 and 21600.",
                                );
                            }
                            body.insert("rate_limit_per_user".into(), json!(v));
                        }
                        "tags" => {
                            let Value::Array(tags) = value else {
                                return fail(
                                    "THREAD_OPTIONS",
                                    "Tags must be an array of up to 5 IDs.",
                                );
                            };
                            if tags.len() > 5 || tags.iter().any(|t| !t.is_string()) {
                                return fail(
                                    "THREAD_OPTIONS",
                                    "Tags must be an array of up to 5 IDs.",
                                );
                            }
                            body.insert("applied_tags".into(), value.clone());
                        }
                        _ => {}
                    }
                }
                let rest = c.ctx.rest.clone();
                let updated = c
                    .wait(async move {
                        rest.patch_channel(thread, Value::Object(body), audit.as_deref())
                            .await
                            .map_err(ScriptFailure::from)
                    })
                    .await?;
                Ok(serde_json::to_value(updated).unwrap_or(Value::Null))
            })
        },
    );

    for (name, join) in [("jointhread", true), ("leavethread", false)] {
        let desc = if join {
            "Makes the bot join the thread."
        } else {
            "Makes the bot leave the thread."
        };
        d.add(name, "threads", 1, 1, "[thread]", desc, move |c| {
            Box::pin(async move {
                c.ctx.require_guild()?;
                let thread = snowflake(&required(c, 0, "Thread").await?)?;
                let rest = c.ctx.rest.clone();
                c.wait(async move {
                    if join {
                        rest.join_thread(thread).await.map_err(ScriptFailure::from)
                    } else {
                        rest.leave_thread(thread).await.map_err(ScriptFailure::from)
                    }
                })
                .await?;
                Ok(Value::String(String::new()))
            })
        });
    }

    d.add(
        "searchthreads",
        "threads",
        1,
        4,
        "[channel;name?;limit?;offset?]",
        "Searches threads; returns threads, total, hasMore, indexing and retryAfter.",
        |c| {
            Box::pin(async move {
                c.ctx.require_guild()?;
                let channel = snowflake(&required(c, 0, "Channel").await?)?;
                let search = flush_rest::ThreadSearch {
                    name: if c.arg_count() > 1 {
                        Some(c.arg(1).await?)
                    } else {
                        None
                    },
                    limit: Some(number_arg(c, 2, 25, 1, 25).await? as u32),
                    offset: Some(number_arg(c, 3, 0, 0, 9975).await? as u32),
                    ..Default::default()
                };
                let rest = c.ctx.rest.clone();
                let result = c
                    .wait(async move {
                        rest.search_threads(channel, &search)
                            .await
                            .map_err(ScriptFailure::from)
                    })
                    .await?;
                Ok(json!({
                    "threads": result.threads.iter().map(channel_json).collect::<Vec<_>>(),
                    "members": result.members.iter().map(|m| serde_json::to_value(m).unwrap_or(Value::Null)).collect::<Vec<_>>(),
                    "hasMore": result.has_more,
                }))
            })
        },
    );

    d.add(
        "invites",
        "invites",
        0,
        0,
        "",
        "Lists the guild's invites; requires ManageGuild from author and bot.",
        |c| {
            Box::pin(async move {
                let guild = permissions(c, "ManageGuild").await?;
                let rest = c.ctx.rest.clone();
                let list = c
                    .wait(
                        async move { rest.guild_invites(guild).await.map_err(ScriptFailure::from) },
                    )
                    .await?;
                Ok(json!(list
                    .iter()
                    .map(|i| {
                        json!({
                            "code": i.code, "uses": i.uses, "maxUses": i.max_uses,
                            "maxAge": i.max_age, "temporary": i.temporary,
                            "createdAt": i.created_at, "expiresAt": i.expires_at,
                        })
                    })
                    .collect::<Vec<_>>()))
            })
        },
    );

    d.add(
        "vanity",
        "invites",
        0,
        0,
        "",
        "Gets the code and uses of the guild's vanity invite.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                let rest = c.ctx.rest.clone();
                let vanity = c
                    .wait(async move { rest.vanity_url(guild).await.map_err(ScriptFailure::from) })
                    .await?;
                Ok(json!({ "code": vanity.code, "uses": vanity.uses }))
            })
        },
    );

    d.add(
        "setvanity",
        "invites",
        1,
        1,
        "[code]",
        "Sets the vanity invite; empty removes it. Requires ManageGuild.",
        |c| {
            Box::pin(async move {
                let guild = permissions(c, "ManageGuild").await?;
                let code = c.arg(0).await?.trim().to_string();
                if !code.is_empty() {
                    let valid = regex::Regex::new(r"^[a-zA-Z0-9_-]{2,32}$")
                        .expect("regex")
                        .is_match(&code);
                    if !valid {
                        return fail(
                            "VANITY",
                            "The code must have 2 to 32 letters, digits, _ or -.",
                        );
                    }
                }
                let rest = c.ctx.rest.clone();
                let result = c
                    .wait(async move {
                        rest.set_vanity_url(
                            guild,
                            if code.is_empty() {
                                None
                            } else {
                                Some(code.as_str())
                            },
                            None,
                        )
                        .await
                        .map_err(ScriptFailure::from)
                    })
                    .await?;
                Ok(json!({ "code": result.code, "uses": result.uses }))
            })
        },
    );

    for name in ["cloneemoji", "clonesticker"] {
        d.add(
            name,
            "expressions",
            1,
            1,
            "[sourceId]",
            "Clones an expression from another guild into the current one; requires ManageExpressions.",
            move |c| {
                Box::pin(async move {
                    let guild = permissions(c, "ManageExpressions").await?;
                    let source = snowflake(&required(c, 0, "Source ID").await?)?;
                    let rest = c.ctx.rest.clone();
                    let result = c
                        .wait(async move {
                            let value = if name == "cloneemoji" {
                                serde_json::to_value(
                                    rest.clone_emoji(guild, source, None)
                                        .await
                                        .map_err(ScriptFailure::from)?,
                                )
                            } else {
                                serde_json::to_value(
                                    rest.clone_sticker(guild, source, None)
                                        .await
                                        .map_err(ScriptFailure::from)?,
                                )
                            };
                            Ok::<_, ScriptFailure>(value.unwrap_or(Value::Null))
                        })
                        .await?;
                    Ok(result)
                })
            },
        );
    }
}
