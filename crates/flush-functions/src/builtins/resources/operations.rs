//! `resource-operations.ts`: instance info, cache ops (deferred), gateway
//! requests (deferred), collectors, webhooks and bulk message fetches.

use serde_json::{json, Value};

use flush_core::Snowflake;

use crate::call::ScriptCall;
use crate::errors::{codes, FnError, FnResult, ScriptFailure};
use crate::types::Definitions;

use super::{
    arg_value, batch, bool_value, collector_channel, guild_channel, integer, invalid,
    native_unavailable, operator, options, snowflake,
};

fn redact_application(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    map.remove("client_secret");
    Value::Object(map)
}

fn message_info(m: &flush_core::Message, channel: Snowflake) -> Value {
    json!({
        "id": m.id.to_string(),
        "channelID": channel.to_string(),
        "guildID": m.guild_id.map(|g| g.to_string()).unwrap_or_default(),
        "content": m.content,
        "authorID": m.author.id.to_string(),
        "authorName": m.author.username,
        "authorBot": m.author.bot,
        "mentions": m.mentions.iter().map(|u| u.id.to_string()).collect::<Vec<_>>(),
    })
}

async fn webhook_message(
    c: &ScriptCall<'_>,
    thread_index: usize,
) -> FnResult<(Snowflake, Snowflake, String, Option<Snowflake>)> {
    c.ctx.require_guild()?;
    let id = snowflake(&c.arg(0).await?)?;
    let message = snowflake(&c.arg(1).await?)?;
    let thread = if c.arg_count() > thread_index {
        Some(snowflake(&c.arg(thread_index).await?)?)
    } else {
        None
    };
    let rest = c.ctx.rest.clone();
    let webhook = c
        .wait(async move { rest.webhook(id).await.map_err(ScriptFailure::from) })
        .await?;
    let Some(token) = webhook.token.clone() else {
        return invalid("The webhook has no available token.");
    };
    if let Some(thread_id) = thread {
        let channel = guild_channel(c, thread_id, "ManageWebhooks").await?;
        if channel.parent_id != webhook.channel_id {
            return invalid("The thread must belong to the webhook's channel.");
        }
    }
    Ok((id, message, token, thread))
}

pub fn register(d: &mut Definitions) {
    d.add(
        "instanceinfo",
        "instance",
        0,
        0,
        "",
        "Returns the configured instance's discovery document; bot owners only.",
        |c| {
            Box::pin(async move {
                operator(c)?;
                let base = c.ctx.rest.api_base();
                let origin = base
                    .trim_end_matches('/')
                    .trim_end_matches("/v1")
                    .to_string();
                let url = format!("{origin}/.well-known/fluxer");
                let doc: Value = c
                    .wait(async move {
                        surf::get(&url)
                            .recv_json()
                            .await
                            .map_err(|e| FnError::new("API_ERROR", format!("{e}")).into())
                    })
                    .await?;
                Ok(instance_info(&doc))
            })
        },
    );

    d.add(
        "gatewayinfo",
        "instance",
        0,
        0,
        "",
        "Returns URL, shards and session limits of the gateway; bot owners only.",
        |c| {
            Box::pin(async move {
                operator(c)?;
                let rest = c.ctx.rest.clone();
                let info = c
                    .wait(async move {
                        rest.raw("GET", "/gateway/bot", None, None, None)
                            .await
                            .map_err(ScriptFailure::from)
                    })
                    .await?;
                let limit = info
                    .get("session_start_limit")
                    .cloned()
                    .unwrap_or(Value::Null);
                Ok(json!({
                    "url": info.get("url"),
                    "shards": info.get("shards"),
                    "sessionStartLimit": match &limit {
                        Value::Object(l) => json!({
                            "total": l.get("total"),
                            "remaining": l.get("remaining"),
                            "resetAfter": l.get("reset_after"),
                            "maxConcurrency": l.get("max_concurrency"),
                        }),
                        _ => Value::Null,
                    },
                }))
            })
        },
    );

    d.add(
        "applicationinfo",
        "instance",
        0,
        0,
        "",
        "Returns the authenticated application without clientSecret; bot owners only.",
        |c| {
            Box::pin(async move {
                operator(c)?;
                let rest = c.ctx.rest.clone();
                let app = c
                    .wait(async move { rest.application().await.map_err(ScriptFailure::from) })
                    .await?;
                Ok(redact_application(
                    serde_json::to_value(app).unwrap_or(Value::Null),
                ))
            })
        },
    );

    d.add(
        "oauthapplications",
        "instance",
        0,
        0,
        "",
        "Lists OAuth applications without clientSecret; bot owners only.",
        |c| {
            Box::pin(async move {
                operator(c)?;
                let rest = c.ctx.rest.clone();
                let app = c
                    .wait(
                        async move { rest.oauth2_application().await.map_err(ScriptFailure::from) },
                    )
                    .await?;
                Ok(json!([redact_application(
                    serde_json::to_value(app).unwrap_or(Value::Null)
                )]))
            })
        },
    );

    d.add(
        "emojimetadata",
        "instance",
        1,
        1,
        "[emoji]",
        "Returns emoji metadata by ID.",
        |c| {
            Box::pin(async move {
                let id = snowflake(&c.arg(0).await?)?;
                let rest = c.ctx.rest.clone();
                let emoji = c
                    .wait(async move { rest.emoji_metadata(id).await.map_err(ScriptFailure::from) })
                    .await?;
                Ok(serde_json::to_value(emoji).unwrap_or(Value::Null))
            })
        },
    );

    d.add(
        "stickermetadata",
        "instance",
        1,
        1,
        "[sticker]",
        "Returns sticker metadata by ID.",
        |c| {
            Box::pin(async move {
                let id = snowflake(&c.arg(0).await?)?;
                let rest = c.ctx.rest.clone();
                let sticker = c
                    .wait(
                        async move { rest.sticker_metadata(id).await.map_err(ScriptFailure::from) },
                    )
                    .await?;
                Ok(serde_json::to_value(sticker).unwrap_or(Value::Null))
            })
        },
    );

    d.add(
        "resolveemoji",
        "instance",
        1,
        1,
        "[emoji]",
        "Resolves an emoji by text, ID or mention in the current guild (when available).",
        |c| {
            Box::pin(async move {
                let emoji = c.arg(0).await?.trim().to_string();
                if emoji.is_empty() || emoji.chars().count() > 128 {
                    return invalid("Enter an emoji.");
                }
                // `<a?:name:id>` or a bare ID resolves to a custom emoji lookup.
                let inner = emoji
                    .strip_prefix('<')
                    .and_then(|s| s.strip_suffix('>'))
                    .map(|s| s.trim_start_matches('a').trim_start_matches(':'))
                    .unwrap_or(&emoji);
                let (name_hint, id_hint) = match inner.rsplit_once(':') {
                    Some((n, i)) if i.chars().all(|c| c.is_ascii_digit()) => {
                        (n.to_string(), Some(i.to_string()))
                    }
                    _ if inner.chars().all(|c| c.is_ascii_digit()) => {
                        (inner.to_string(), Some(inner.to_string()))
                    }
                    _ => (inner.to_string(), None),
                };
                let guild = c.ctx.guild_id();
                if guild.get() == 0 {
                    return Ok(Value::Null);
                }
                let rest = c.ctx.rest.clone();
                let emojis = c
                    .wait(
                        async move { rest.guild_emojis(guild).await.map_err(ScriptFailure::from) },
                    )
                    .await?;
                let found = emojis.iter().find(|e| {
                    id_hint.as_deref() == Some(e.id.to_string().as_str())
                        || e.name == name_hint
                        || e.name == emoji
                });
                Ok(match found {
                    Some(e) => serde_json::to_value(e).unwrap_or(Value::Null),
                    None => Value::Null,
                })
            })
        },
    );

    for name in [
        "cachestats",
        "sweepcache",
        "cascadecache",
        "evictcachedchannel",
    ] {
        let (min, max) = match name {
            "sweepcache" => (1, 3),
            "cascadecache" => (2, 2),
            "evictcachedchannel" => (1, 1),
            _ => (0, 0),
        };
        d.add(
            name,
            "cache",
            min,
            max,
            "",
            "Local cache operation; bot owners only.",
            move |c| {
                Box::pin(async move {
                    operator(c)?;
                    native_unavailable("The local cache")
                })
            },
        );
    }

    d.add(
        "selfuser",
        "users",
        0,
        0,
        "",
        "Fetches normalized data of the authenticated user; bot owners only.",
        |c| {
            Box::pin(async move {
                operator(c)?;
                let rest = c.ctx.rest.clone();
                let user = c
                    .wait(async move { rest.current_user().await.map_err(ScriptFailure::from) })
                    .await?;
                Ok(serde_json::to_value(user).unwrap_or(Value::Null))
            })
        },
    );

    d.add(
        "selfguilds",
        "users",
        0,
        1,
        "[counts?]",
        "Lists the bot's guilds, optionally with counts; bot owners only.",
        |c| {
            Box::pin(async move {
                operator(c)?;
                let counts = if c.arg_count() > 0 {
                    bool_value(&arg_value(c, 0).await?)?
                } else {
                    false
                };
                let rest = c.ctx.rest.clone();
                let guilds = c
                    .wait(async move {
                        rest.current_user_guilds(counts)
                            .await
                            .map_err(ScriptFailure::from)
                    })
                    .await?;
                Ok(serde_json::to_value(guilds).unwrap_or(Value::Null))
            })
        },
    );

    d.add(
        "selfleaveguild",
        "users",
        0,
        0,
        "",
        "Removes the bot from the current guild without sudo credentials; requires the bot owner and the guild owner.",
        |c| {
            Box::pin(async move {
                operator(c)?;
                let guild = c.ctx.require_guild()?;
                let rest = c.ctx.rest.clone();
                c.wait(async move { rest.leave_guild(guild).await.map_err(ScriptFailure::from) })
                    .await?;
                Ok(Value::String(String::new()))
            })
        },
    );

    d.add(
        "deletemymessages",
        "users",
        0,
        0,
        "",
        "Asks to delete all of the bot's messages in the current guild; requires the bot owner and ManageMessages from author and bot.",
        |c| {
            Box::pin(async move {
                operator(c)?;
                let guild = c.ctx.require_guild()?;
                super::both(c, "ManageMessages", None).await?;
                let rest = c.ctx.rest.clone();
                c.wait(async move {
                    rest.bulk_delete_my_messages_in_guild(guild, None)
                        .await
                        .map_err(ScriptFailure::from)
                })
                .await?;
                Ok(Value::String(String::new()))
            })
        },
    );

    for (name, min, max) in [
        ("fetchgatewaymembers", 0, 1),
        ("requestgatewaymembers", 0, 1),
        ("requestguildcounts", 0, 0),
        ("requestchannelcounts", 1, 1),
        ("requestforumunreads", 2, 2),
        ("subscribethreads", 1, 2),
    ] {
        d.add(
            name,
            "gateway",
            min,
            max,
            "",
            "Gateway request operation.",
            move |_c| Box::pin(async move { native_unavailable("Gateway request operations") }),
        );
    }

    d.add(
        "collectmessages",
        "collectors",
        1,
        4,
        "[timeMs;max?;user?;contains?]",
        "Collects up to 20 messages in the current channel within 30000 ms; only normalized data and declarative filters.",
        |c| {
            Box::pin(async move {
                let time_ms = integer(&arg_value(c, 0).await?, 1, 30_000)? as u64;
                let max = if c.arg_count() > 1 {
                    integer(&arg_value(c, 1).await?, 1, 20)? as usize
                } else {
                    5
                };
                let user = if c.arg_count() > 2 {
                    let v = c.arg(2).await?;
                    if v.is_empty() {
                        None
                    } else {
                        Some(snowflake(&v)?)
                    }
                } else {
                    None
                };
                let contains = if c.arg_count() > 3 {
                    let v = c.arg(3).await?;
                    if v.chars().count() > 200 {
                        return invalid("Enter text of up to 200 characters.");
                    }
                    Some(v)
                } else {
                    None
                };
                let channel = collector_channel(c).await?;
                let Some(collectors) = &c.ctx.collectors else {
                    return native_unavailable("Message collectors");
                };
                let collectors = collectors.clone();
                let result = c
                    .wait(async move {
                        let filter = move |m: &flush_core::Message| {
                            m.channel_id == channel
                                && user.is_none_or(|u| m.author.id == u)
                                && contains
                                    .as_ref()
                                    .is_none_or(|t| m.content.contains(t.as_str()))
                        };
                        Ok::<_, ScriptFailure>(
                            collectors
                                .messages(filter)
                                .collect(max, std::time::Duration::from_millis(time_ms))
                                .await,
                        )
                    })
                    .await?;
                let mut out = Vec::new();
                for m in result.iter().take(max) {
                    out.push(message_info(m, channel));
                    }
                Ok(Value::Array(out))
            })
        },
    );

    d.add(
        "collectreactions",
        "collectors",
        2,
        5,
        "[message;timeMs;max?;user?;emojis?]",
        "Collects up to 20 reactions in the current channel within 30000 ms; emojis is an optional array of up to 10 texts.",
        |c| {
            Box::pin(async move {
                let message = snowflake(&c.arg(0).await?)?;
                let time_ms = integer(&arg_value(c, 1).await?, 1, 30_000)? as u64;
                let max = if c.arg_count() > 2 {
                    integer(&arg_value(c, 2).await?, 1, 20)? as usize
                } else {
                    5
                };
                let user = if c.arg_count() > 3 {
                    let v = c.arg(3).await?;
                    if v.is_empty() {
                        None
                    } else {
                        Some(snowflake(&v)?)
                    }
                } else {
                    None
                };
                let emojis: Option<Vec<String>> = if c.arg_count() > 4 {
                    let value = arg_value(c, 4).await?;
                    let items = batch(&value, 10)?;
                    let mut list = Vec::new();
                    for item in items {
                        let Some(s) = item.as_str() else {
                            return invalid("Enter valid emoji texts.");
                        };
                        if s.trim().is_empty() || s.len() > 128 {
                            return invalid("Emoji required.");
                        }
                        list.push(s.trim().to_string());
                    }
                    Some(list)
                } else {
                    None
                };
                let channel = collector_channel(c).await?;
                let Some(collectors) = &c.ctx.collectors else {
                    return native_unavailable("Reaction collectors");
                };
                let collectors = collectors.clone();
                let result = c
                    .wait(async move {
                        let filter = move |r: &flush_framework::CollectedReaction| {
                            r.channel_id == channel
                                && r.message_id == message
                                && user.is_none_or(|u| r.user_id == u)
                                && emojis
                                    .as_ref()
                                    .is_none_or(|list| list.iter().any(|e| e == &r.emoji.name || e == &r.emoji.escaped()))
                        };
                        Ok::<_, ScriptFailure>(
                            collectors
                                .reactions(filter)
                                .collect(max, std::time::Duration::from_millis(time_ms))
                                .await,
                        )
                    })
                    .await?;
                let mut out = Vec::new();
                for r in result.iter().take(max) {
                    out.push(json!({
                        "userID": r.user_id.to_string(),
                        "emoji": r.emoji,
                        "messageID": r.message_id.to_string(),
                    }));
                    }
                Ok(Value::Array(out))
            })
        },
    );

    d.add(
        "fetchwebhookmessage",
        "webhooks",
        2,
        3,
        "[webhook;message;thread?]",
        "Fetches a webhook message with ManageWebhooks from author and bot; the script never sees the token.",
        |c| {
            Box::pin(async move {
                let (id, message, token, thread) = webhook_message(c, 2).await?;
                let rest = c.ctx.rest.clone();
                let result = c
                    .wait(async move {
                        rest.raw(
                            "GET",
                            &format!(
                                "/webhooks/{id}/{token}/messages/{message}{}",
                                thread.map(|t| format!("?thread_id={t}")).unwrap_or_default()
                            ),
                            None,
                            None,
                            None,
                        )
                        .await
                        .map_err(ScriptFailure::from)
                    })
                    .await?;
                Ok(result)
            })
        },
    );

    d.add(
        "editwebhookmessage",
        "webhooks",
        3,
        4,
        "[webhook;message;text;thread?]",
        "Edits up to 2000 characters of a webhook message, without automatic mentions; requires ManageWebhooks from author and bot.",
        |c| {
            Box::pin(async move {
                let content = c.arg(2).await?;
                if content.chars().count() > 2000 {
                    return invalid("Enter text of up to 2000 characters.");
                }
                let (id, message, token, _thread) = webhook_message(c, 3).await?;
                let rest = c.ctx.rest.clone();
                let body = flush_rest::EditMessageBody {
                    content: Some(content),
                    ..Default::default()
                };
                let result = c
                    .wait(async move {
                        rest.edit_webhook_message(id, &token, message, &body)
                            .await
                            .map_err(ScriptFailure::from)
                    })
                    .await?;
                Ok(serde_json::to_value(result).unwrap_or(Value::Null))
            })
        },
    );

    d.add(
        "deletewebhookmessage",
        "webhooks",
        2,
        3,
        "[webhook;message;thread?]",
        "Deletes a webhook message with ManageWebhooks from author and bot, without passing the token to the script.",
        |c| {
            Box::pin(async move {
                let (id, message, token, _thread) = webhook_message(c, 2).await?;
                let rest = c.ctx.rest.clone();
                c.wait(async move {
                    rest.delete_webhook_message(id, &token, message)
                        .await
                        .map_err(ScriptFailure::from)
                })
                .await?;
                Ok(Value::String(String::new()))
            })
        },
    );

    d.add(
        "bulkfetchmessages",
        "messages",
        1,
        1,
        "[windows]",
        "Fetches normalized messages in up to 10 visible channels (up to 100 messages total); each window has channelID, limit and one optional cursor.",
        |c| {
            Box::pin(async move {
                c.ctx.require_guild()?;
                let value = arg_value(c, 0).await?;
                let items = batch(&value, 10)?;
                let mut requests = Vec::new();
                for item in items {
                    let map = options(item, &["channelID", "limit", "after", "before", "around"])?;
                    let cursors: Vec<&str> = ["after", "before", "around"]
                        .iter()
                        .copied()
                        .filter(|k| map.contains_key(*k))
                        .collect();
                    if cursors.len() > 1 {
                        return invalid("Use only one cursor per channel.");
                    }
                    let channel = snowflake(
                        map.get("channelID")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default(),
                    )?;
                    let limit = integer(map.get("limit").unwrap_or(&Value::Null), 1, 100)? as u32;
                    let cursor = cursors
                        .first()
                        .map(|k| snowflake(map.get(*k).and_then(|v| v.as_str()).unwrap_or_default()))
                        .transpose()?;
                    requests.push((channel, limit, cursors.first().copied().unwrap_or(""), cursor));
                }
                if requests.iter().map(|(_, l, _, _)| l).sum::<u32>() > 100 {
                    return invalid("The total limit is 100 messages.");
                }
                for (channel, _, _, _) in &requests {
                    guild_channel(c, *channel, "ViewChannel").await?;
                    super::both(c, "ReadMessageHistory", None).await?;
                }
                let rest = c.ctx.rest.clone();
                let bulk: Vec<flush_rest::BulkMessageRequest> = requests
                    .iter()
                    .map(|(channel, limit, kind, cursor)| flush_rest::BulkMessageRequest {
                        channel_id: *channel,
                        limit: *limit,
                        before: if *kind == "before" { *cursor } else { None },
                        after: if *kind == "after" { *cursor } else { None },
                        around: if *kind == "around" { *cursor } else { None },
                    })
                    .collect();
                let pages = c
                    .wait(async move {
                        rest.bulk_fetch_messages(&bulk)
                            .await
                            .map_err(ScriptFailure::from)
                    })
                    .await?;
                let mut out = Vec::new();
                for page in &pages {
                    for m in &page.messages {
                        out.push(message_info(m, page.channel_id));
                    }
                    }
                Ok(Value::Array(out))
            })
        },
    );

    d.add(
        "cachedmessage",
        "messages",
        1,
        1,
        "[message]",
        "Reads a message from the current channel's cache, without fetching it from Fluxer; returns null when absent.",
        |c| {
            Box::pin(async move {
                let _ = snowflake(&c.arg(0).await?)?;
                let _ = collector_channel(c).await?;
                Err(FnError::new(
                    codes::NATIVE_UNAVAILABLE,
                    "The messages resource does not implement the message cache.",
                )
                .into())
            })
        },
    );

    d.add(
        "channelslowmode",
        "channels",
        1,
        1,
        "[channel]",
        "Reads the slowmode of a visible channel in the current guild.",
        |c| {
            Box::pin(async move {
                let id = snowflake(&c.arg(0).await?)?;
                guild_channel(c, id, "ViewChannel").await?;
                let rest = c.ctx.rest.clone();
                c.wait(async move { rest.slowmode_state(id).await.map_err(ScriptFailure::from) })
                    .await
            })
        },
    );

    d.add(
        "channelrtcregions",
        "channels",
        1,
        1,
        "[channel]",
        "Reads the RTC regions of a visible channel in the current guild.",
        |c| {
            Box::pin(async move {
                let id = snowflake(&c.arg(0).await?)?;
                guild_channel(c, id, "ViewChannel").await?;
                let rest = c.ctx.rest.clone();
                c.wait(async move {
                    Ok::<Value, ScriptFailure>(json!(rest
                        .rtc_regions(id)
                        .await
                        .map_err(ScriptFailure::from)?))
                })
                .await
            })
        },
    );

    d.add(
        "sendtyping",
        "channels",
        0,
        1,
        "[channel?]",
        "Sends a typing indicator in the current guild; requires SendMessages from author and bot.",
        |c| {
            Box::pin(async move {
                c.ctx.require_guild()?;
                let channel = if c.arg_count() > 0 {
                    snowflake(&c.arg(0).await?)?
                } else {
                    c.ctx.channel_id()
                };
                let rest = c.ctx.rest.clone();
                c.wait(async move { rest.typing(channel).await.map_err(ScriptFailure::from) })
                    .await?;
                Ok(Value::String(String::new()))
            })
        },
    );
}

/// `instanceInfo(discovery)` from yuzuha `resources/instance.ts`: maps the
/// `.well-known/fluxer` document to the camelCase shape scripts expect.
fn instance_info(doc: &Value) -> Value {
    let ep = doc.get("endpoints").cloned().unwrap_or(Value::Null);
    let endpoints = match &ep {
        Value::Object(e) => json!({
            "api": e.get("api"), "apiClient": e.get("api_client"),
            "apiPublic": e.get("api_public"), "gateway": e.get("gateway"),
            "media": e.get("media"), "staticCDN": e.get("static_cdn"),
            "marketing": e.get("marketing"), "admin": e.get("admin"),
            "invite": e.get("invite"), "gift": e.get("gift"),
            "webapp": e.get("webapp"),
        }),
        _ => Value::Null,
    };
    let features = match doc.get("features") {
        Some(Value::Object(f)) => json!({
            "voiceEnabled": f.get("voice_enabled"), "stripeEnabled": f.get("stripe_enabled"),
            "selfHosted": f.get("self_hosted"),
            "presignedAttachmentUploads": f.get("presigned_attachment_uploads"),
            "emailsEnabled": f.get("emails_enabled"),
        }),
        _ => Value::Null,
    };
    let community = doc.get("community").cloned().unwrap_or(Value::Null);
    let services = match doc.get("services") {
        Some(Value::Object(v)) => json!({
            "gif": v.get("gif_enabled"), "youtube": v.get("youtube_enabled"),
            "bluesky": v.get("bluesky_enabled"),
        }),
        _ => Value::Null,
    };
    json!({
        "apiCodeVersion": doc.get("api_code_version"),
        "codename": doc.get("codename"),
        "endpoints": endpoints,
        "features": features,
        "registrationMode": doc.pointer("/registration/mode"),
        "ssoEnabled": doc.pointer("/sso/enabled"),
        "ssoEnforced": doc.pointer("/sso/enforced"),
        "singleCommunity": community.get("single_community"),
        "singleCommunityGuildID": community.get("single_community_guild_id"),
        "directMessagesDisabled": community.get("direct_messages_disabled"),
        "services": services,
    })
}
