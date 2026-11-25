//! yuzuha `native-messages.ts` + `entities-messages.ts` + the message
//! builtins of `entities-channels.ts` (`acknowledgePins`, `clearReadState`,
//! `createDM`).

use serde_json::{json, Value};

use flush_core::message::{AllowedMentions, AttachmentRef, MessageSend};
use flush_core::Permissions;
use flush_rest::uploads::FileUpload;

use crate::errors::codes;
use crate::types::Definitions;

use super::common::{
    arg_value, channel_id, context_channel, entity_property, err, found, integer, opt,
    reaction_emoji, require_perms, rest, serialize, snowflake, user_id, Res,
};
use super::info::{channel_info, message_info, MESSAGE};

pub fn register(d: &mut Definitions) {
    d.add(
        "preloadMessages",
        "messages",
        1,
        1,
        "[canais]",
        "Fetches the last message of up to 100 channels; takes an array of IDs and returns an object per channel.",
        |c| {
            Box::pin(async move {
                let value = arg_value(c, 0).await?;
                let Some(list) = value.as_array() else {
                    return Err(err(codes::INVALID_ARGUMENTS, "Expected an array of channel IDs."));
                };
                if list.is_empty() || list.len() > 100 {
                    return Err(err(codes::INVALID_ARGUMENTS, "Expected 1 to 100 channel IDs."));
                }
                let mut ids = Vec::new();
                for item in list {
                    let raw = item.as_str().unwrap_or_default();
                    ids.push(snowflake(raw, "channel")?);
                }
                let result = rest(c, c.ctx.rest.preload_messages(&ids)).await?;
                Ok(serialize(&result))
            })
        },
    );

    d.add(
        "removeReaction",
        "messages",
        2,
        4,
        "[mensagem;emoji;usuário?;canal?]",
        "Removes the bot's or a user's reaction from the message.",
        |c| {
            Box::pin(async move {
                let message = snowflake(&c.arg(0).await?, "message")?;
                let emoji = super::common::reaction_emoji_value(&c.arg(1).await?);
                let user = opt_arg(c, 2).await?.map(|s| user_id(&s)).transpose()?;
                let channel = opt_arg(c, 3)
                    .await?
                    .map(|s| channel_id(&s))
                    .transpose()?
                    .unwrap_or(c.ctx.channel_id());
                match user {
                    Some(user) => {
                        rest(
                            c,
                            c.ctx
                                .rest
                                .remove_reaction(channel, message, &emoji, user, None),
                        )
                        .await?
                    }
                    None => {
                        rest(c, c.ctx.rest.remove_own_reaction(channel, message, &emoji)).await?
                    }
                }
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "clearReactions",
        "messages",
        1,
        3,
        "[mensagem;emoji?;canal?]",
        "Removes all reactions or only the ones of an emoji.",
        |c| {
            Box::pin(async move {
                let message = snowflake(&c.arg(0).await?, "message")?;
                let emoji = opt_arg(c, 1).await?.map(|s| reaction_emoji(&s));
                let channel = opt_arg(c, 2)
                    .await?
                    .map(|s| channel_id(&s))
                    .transpose()?
                    .unwrap_or(c.ctx.channel_id());
                match emoji {
                    Some(emoji) => {
                        rest(c, c.ctx.rest.clear_reaction(channel, message, &emoji)).await?
                    }
                    None => rest(c, c.ctx.rest.clear_reactions(channel, message, None)).await?,
                }
                Ok(Value::Null)
            })
        },
    );

    for name in ["reactionUsers", "reactionUsersPage"] {
        let paged = name == "reactionUsersPage";
        let desc = if paged {
            "Returns users with IDs, hasMore and nextAfter; limit of 1 to 100 and user cursor."
        } else {
            "Lists IDs of users who reacted; limit of 1 to 100 and user cursor."
        };
        d.add(
            name,
            "messages",
            2,
            5,
            "[mensagem;emoji;limite?;após?;canal?]",
            desc,
            move |c| {
                Box::pin(async move {
                    let message = snowflake(&c.arg(0).await?, "message")?;
                    let emoji = super::common::reaction_emoji_value(&c.arg(1).await?);
                    let n = match opt_arg(c, 2).await? {
                        Some(s) => {
                            let n = integer(&s)?;
                            if !(1..=100).contains(&n) {
                                return Err(err(
                                    codes::FUNCTION_ERROR,
                                    format!("Invalid limit: {n}."),
                                ));
                            }
                            n as u32
                        }
                        None => 100,
                    };
                    let after = opt_arg(c, 3)
                        .await?
                        .map(|s| snowflake(&s, "user"))
                        .transpose()?;
                    let channel = opt_arg(c, 4)
                        .await?
                        .map(|s| channel_id(&s))
                        .transpose()?
                        .unwrap_or(c.ctx.channel_id());
                    if paged {
                        return rest(
                            c,
                            c.ctx.rest.reaction_users_page(
                                channel,
                                message,
                                &emoji.escaped(),
                                n,
                                after,
                            ),
                        )
                        .await;
                    }
                    let users =
                        rest(c, c.ctx.rest.reaction_users(channel, message, &emoji, n)).await?;
                    Ok(Value::Array(
                        users
                            .iter()
                            .map(|u| Value::String(u.id.to_string()))
                            .collect(),
                    ))
                })
            },
        );
    }

    d.add(
        "fetchMessage",
        "messages",
        2,
        3,
        "[canal;mensagem;propriedade?]",
        "Fetches a message and returns the object or a property.",
        |c| {
            Box::pin(async move {
                let channel = channel_id(&c.arg(0).await?)?;
                let message_id = snowflake(&c.arg(1).await?, "message")?;
                let message = match found(c, c.ctx.rest.message(channel, message_id)).await? {
                    Some(message) => message,
                    None => return Ok(Value::Null),
                };
                let info = message_info(&message);
                if c.arg_count() < 3 {
                    return Ok(info);
                }
                let raw = c.arg(2).await?;
                entity_property(&info, raw.trim(), MESSAGE)
            })
        },
    );

    d.add(
        "messages",
        "messages",
        1,
        5,
        "[canal;limite?;before?;after?;around?]",
        "Lists messages, newest to oldest; accepts a single cursor per call.",
        |c| {
            Box::pin(async move {
                let channel = channel_id(&c.arg(0).await?)?;
                let n = match opt_arg(c, 1).await? {
                    Some(s) => integer(&s)?,
                    None => 50,
                };
                let before = opt_arg(c, 2).await?;
                let after = opt_arg(c, 3).await?;
                let around = opt_arg(c, 4).await?;
                let count = [&before, &after, &around]
                    .iter()
                    .filter(|c| c.is_some())
                    .count();
                if count > 1 {
                    return Err(err(
                        codes::INVALID_ARGUMENTS,
                        "Use a single cursor (before, after or around).",
                    ));
                }
                let cursor = if let Some(b) = &before {
                    Some(("before", snowflake(b, "message")?))
                } else if let Some(a) = &after {
                    Some(("after", snowflake(a, "message")?))
                } else {
                    around
                        .map(|a| snowflake(&a, "message"))
                        .transpose()?
                        .map(|id| ("around", id))
                };
                let (before, after, around) = match cursor {
                    Some(("before", id)) => (Some(id), None, None),
                    Some(("after", id)) => (None, Some(id), None),
                    Some(("around", id)) => (None, None, Some(id)),
                    _ => (None, None, None),
                };
                let messages = rest(
                    c,
                    c.ctx
                        .rest
                        .list_messages_cursor(channel, n as u32, before, after, around),
                )
                .await?;
                Ok(super::common::camelize(
                    &serde_json::json!({ "messages": messages }),
                ))
            })
        },
    );

    d.add(
        "pin",
        "messages",
        1,
        2,
        "[mensagem;canal?]",
        "Pins a message in the channel.",
        |c| {
            Box::pin(async move {
                let message = snowflake(&c.arg(0).await?, "message")?;
                let channel = opt_arg(c, 1)
                    .await?
                    .map(|s| channel_id(&s))
                    .transpose()?
                    .unwrap_or(c.ctx.channel_id());
                rest(c, c.ctx.rest.pin_message(channel, message, None)).await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "unpin",
        "messages",
        1,
        2,
        "[mensagem;canal?]",
        "Unpins a message in the channel.",
        |c| {
            Box::pin(async move {
                let message = snowflake(&c.arg(0).await?, "message")?;
                let channel = opt_arg(c, 1)
                    .await?
                    .map(|s| channel_id(&s))
                    .transpose()?
                    .unwrap_or(c.ctx.channel_id());
                rest(c, c.ctx.rest.unpin_message(channel, message, None)).await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "pinnedMessages",
        "messages",
        0,
        2,
        "[canal?;limite?]",
        "Lists pinned messages, newest to oldest.",
        |c| {
            Box::pin(async move {
                let channel = opt_arg(c, 0)
                    .await?
                    .map(|s| channel_id(&s))
                    .transpose()?
                    .unwrap_or(c.ctx.channel_id());
                let n = opt_arg(c, 1).await?.map(|s| integer(&s)).transpose()?;
                let mut messages = rest(c, c.ctx.rest.pinned_messages(channel)).await?;
                if let Some(n) = n {
                    messages.truncate(n as usize);
                }
                Ok(super::common::camelize(
                    &serde_json::json!({ "messages": messages }),
                ))
            })
        },
    );

    d.add(
        "pinnedMessagesPage",
        "messages",
        0,
        3,
        "[canal?;limite?;before?]",
        "Pages messages by pin date; returns messages, pinnedAt, hasMore and nextBefore. ISO cursor of the previous page.",
        |c| {
            Box::pin(async move {
                let channel = opt_arg(c, 0)
                    .await?
                    .map(|s| channel_id(&s))
                    .transpose()?
                    .unwrap_or(c.ctx.channel_id());
                let n = match opt_arg(c, 1).await? {
                    Some(s) => integer(&s)?,
                    None => 50,
                };
                let before = opt_arg(c, 2).await?;
                let page = rest(
                    c,
                    c.ctx.rest.pinned_messages_page(channel, n as u32, before.as_deref().and_then(|s| s.parse().ok())),
                )
                .await?;
                Ok(page)
            })
        },
    );

    d.add(
        "sendFiles",
        "messages",
        2,
        4,
        "[canal;url;nome?;conteudo?]",
        "Sends an attachment and returns the message ID.",
        |c| {
            Box::pin(async move {
                let channel = channel_id(&c.arg(0).await?)?;
                let url = c.arg(1).await?;
                let name = match opt_arg(c, 2).await? {
                    Some(name) => name,
                    None => {
                        let tail = url.rsplit('/').next().unwrap_or("file");
                        tail.split('?').next().unwrap_or("file").to_string()
                    }
                };
                if name.trim().is_empty() || name.len() > 255 {
                    return Err(err(
                        codes::FUNCTION_ERROR,
                        "Attachment name must be 1 to 255 characters.",
                    ));
                }
                let content = opt_arg(c, 3).await?;
                let bytes = c
                    .wait(async move {
                        surf::get(&url)
                            .recv_bytes()
                            .await
                            .map_err(|e| err(codes::API_ERROR, e.to_string()))
                    })
                    .await?;
                if bytes.len() > 10_000_000 {
                    return Err(err(codes::FUNCTION_ERROR, "Attachment over 10 MB."));
                }
                let uploads = rest(
                    c,
                    c.ctx.rest.upload_attachments(
                        channel,
                        &[FileUpload {
                            filename: name.clone(),
                            data: bytes,
                            content_type: None,
                            description: None,
                        }],
                    ),
                )
                .await?;
                let attachments: Vec<AttachmentRef> = uploads
                    .iter()
                    .enumerate()
                    .map(|(i, upload)| AttachmentRef {
                        id: i as u64,
                        filename: name.clone(),
                        upload_filename: Some(upload.filename.clone()),
                        ..Default::default()
                    })
                    .collect();
                let send = MessageSend {
                    content,
                    attachments,
                    allowed_mentions: Some(AllowedMentions::no_ping()),
                    ..Default::default()
                };
                let message = rest(c, c.ctx.rest.send_message(channel, &send)).await?;
                Ok(Value::String(message.id.to_string()))
            })
        },
    );

    d.add(
        "attachment",
        "messages",
        1,
        2,
        "[url;nome?]",
        "Builds an attachment spec without sending.",
        |c| {
            Box::pin(async move {
                let url = c.arg(0).await?;
                let name = match opt_arg(c, 1).await? {
                    Some(name) => name,
                    None => {
                        let tail = url.rsplit('/').next().unwrap_or("file");
                        tail.split('?').next().unwrap_or("file").to_string()
                    }
                };
                Ok(json!({ "url": url, "name": name }))
            })
        },
    );

    // ---- entities-messages.ts ----

    d.add(
        "deleteAttachment",
        "messages",
        2,
        3,
        "[mensagem;anexo;canal?]",
        "Removes an attachment from a message; own author or ManageMessages, always with bot authority.",
        |c| {
            Box::pin(async move {
                let message = snowflake(&c.arg(0).await?, "message")?;
                let attachment = snowflake(&c.arg(1).await?, "attachment")?;
                let channel = opt_arg(c, 2)
                    .await?
                    .map(|s| channel_id(&s))
                    .transpose()?
                    .unwrap_or(c.ctx.channel_id());
                rest(
                    c,
                    c.ctx.rest.delete_attachment(channel, message, attachment),
                )
                .await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "bulkDelete",
        "messages",
        1,
        2,
        "[mensagens;canal?]",
        "Deletes 1 to 100 IDs with ManageMessages of author and bot; preserves the partial result without retrying the API.",
        |c| {
            Box::pin(async move {
                let channel = opt_arg(c, 1)
                    .await?
                    .map(|s| channel_id(&s))
                    .transpose()?
                    .unwrap_or(c.ctx.channel_id());
                let channel_obj = context_channel(c, Some(channel)).await?;
                require_perms(c, Some(&channel_obj), Permissions::MANAGE_MESSAGES).await?;
                let value = arg_value(c, 0).await?;
                let ids: Vec<String> = match &value {
                    Value::Array(list) => list
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect(),
                    Value::String(s) => s.split(',').map(|s| s.trim().to_string()).collect(),
                    _ => return Err(err(codes::INVALID_ARGUMENTS, "Expected message IDs.")),
                };
                if ids.is_empty() || ids.len() > 100 {
                    return Err(err(
                        codes::INVALID_ARGUMENTS,
                        "Send 1 to 100 message IDs.",
                    ));
                }
                let ids: Vec<flush_core::Snowflake> = ids
                    .iter()
                    .map(|s| snowflake(s, "message"))
                    .collect::<Result<_, _>>()?;
                let deleted = rest(c, c.ctx.rest.bulk_delete(channel, &ids)).await?;
                Ok(serialize(&deleted))
            })
        },
    );

    d.add(
        "purgeMessages",
        "messages",
        1,
        3,
        "[limite;autor?;canal?]",
        "Deletes up to 100 recent messages with an optional author filter; requires ManageMessages of author and bot.",
        |c| {
            Box::pin(async move {
                let n = integer(&c.arg(0).await?)?;
                if !(1..=100).contains(&n) {
                    return Err(err(codes::FUNCTION_ERROR, format!("Invalid limit: {n}.")));
                }
                let author = opt_arg(c, 1).await?.map(|s| user_id(&s)).transpose()?;
                let channel = opt_arg(c, 2)
                    .await?
                    .map(|s| channel_id(&s))
                    .transpose()?
                    .unwrap_or(c.ctx.channel_id());
                let channel_obj = context_channel(c, Some(channel)).await?;
                require_perms(c, Some(&channel_obj), Permissions::MANAGE_MESSAGES).await?;
                let mut body = json!({ "limit": n });
                if let Some(author) = author {
                    body["author_id"] = json!(author.to_string());
                }
                let result = rest(c, c.ctx.rest.purge_messages_with(channel, body)).await?;
                Ok(result)
            })
        },
    );

    d.add(
        "bulkDeleteMyMessages",
        "messages",
        0,
        2,
        "[limite?;canal?]",
        "Deletes only the bot's messages in a window of up to 100 messages; requires ManageMessages of author and bot.",
        |c| {
            Box::pin(async move {
                let n = match opt_arg(c, 0).await? {
                    Some(s) => integer(&s)?,
                    None => 100,
                };
                if !(1..=100).contains(&n) {
                    return Err(err(codes::FUNCTION_ERROR, format!("Invalid limit: {n}.")));
                }
                let channel = opt_arg(c, 1)
                    .await?
                    .map(|s| channel_id(&s))
                    .transpose()?
                    .unwrap_or(c.ctx.channel_id());
                let channel_obj = context_channel(c, Some(channel)).await?;
                require_perms(c, Some(&channel_obj), Permissions::MANAGE_MESSAGES).await?;
                let result = rest(
                    c,
                    c.ctx.rest.purge_messages_with(
                        channel,
                        json!({ "limit": n, "author_id": c.ctx.bot_id.map(|b| b.to_string()).unwrap_or_default() }),
                    ),
                )
                .await?;
                Ok(result)
            })
        },
    );

    // ---- entities-channels.ts (message-side) ----

    d.add(
        "createDM",
        "messages",
        1,
        1,
        "[usuário]",
        "Opens a DM with a user by ID, without sending a message; returns the normalized channel.",
        |c| {
            Box::pin(async move {
                let user = user_id(&c.arg(0).await?)?;
                let channel = rest(c, c.ctx.rest.create_dm(user)).await?;
                Ok(channel_info(&channel))
            })
        },
    );
}

async fn opt_arg(c: &crate::call::ScriptCall<'_>, i: usize) -> Res<Option<String>> {
    if i >= c.arg_count() {
        return Ok(None);
    }
    Ok(opt(c.arg(i).await?))
}
