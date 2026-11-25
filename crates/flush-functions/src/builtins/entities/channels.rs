//! yuzuha `native-channels.ts` + channel builtins of `entities-channels.ts`
//! and `entities-guild.ts` (`setChannelPositions`).

use serde_json::{json, Value};

use flush_core::{ChannelType, OverwriteType, Permissions};
use flush_rest::client::ChannelCreate;
use flush_rest::resources::ChannelPosition;

use crate::errors::codes;
use crate::types::Definitions;

use super::common::{
    arg_value, channel_id, context_channel, entity_property, err, found, guard, integer, opt,
    require_perms, rest, role_id, snowflake, top_role, user_id, Res,
};
use super::info::{channel_info, CHANNEL};

/// Channel type names of `native-channels.ts`.
fn channel_kind(name: &str) -> Option<u16> {
    Some(match name {
        "text" | "texto" => 0,
        "voice" | "voz" => 2,
        "dm" => 1,
        "groupDm" => 3,
        "categoria" | "category" => 4,
        "announcement" | "anuncio" => 5,
        "announcementThread" => 10,
        "publicThread" => 11,
        "privateThread" => 12,
        "forum" => 15,
        "media" => 16,
        "link" => 998,
        _ => return None,
    })
}

/// Resolves an overwrite target: `<@&id>`/role id → Role, else member → Member.
fn overwrite_target(raw: &str) -> Res<(flush_core::Snowflake, OverwriteType)> {
    let s = raw.trim();
    if s.starts_with("<@&") {
        Ok((role_id(s)?, OverwriteType::Role))
    } else if s.starts_with("<@") {
        Ok((user_id(s)?, OverwriteType::Member))
    } else {
        Ok((snowflake(s, "target")?, OverwriteType::Role))
    }
}

pub fn register(d: &mut Definitions) {
    d.add(
        "createLinkChannel",
        "channels",
        2,
        3,
        "[nome;url;categoria?]",
        "Creates a link channel with author and bot permission checks.",
        |c| {
            Box::pin(async move {
                let name = c.arg(0).await?;
                let url = c.arg(1).await?;
                let category = opt_arg(c, 2).await?.map(|s| channel_id(&s)).transpose()?;
                let guild = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_CHANNELS).await?;
                let create = ChannelCreate {
                    name,
                    kind: Some(ChannelType::GuildLink as u16),
                    url: Some(url),
                    parent_id: category,
                    ..Default::default()
                };
                let channel = rest(c, c.ctx.rest.create_channel(guild, &create, None)).await?;
                Ok(channel_info(&channel))
            })
        },
    );

    d.add(
        "editLinkChannel",
        "channels",
        2,
        3,
        "[canal;url;motivo?]",
        "Changes the URL of a link channel of the current guild.",
        |c| {
            Box::pin(async move {
                let channel = channel_id(&c.arg(0).await?)?;
                let url = c.arg(1).await?;
                let reason = opt_arg(c, 2).await?;
                c.ctx.require_guild()?;
                let target = rest(c, c.ctx.rest.channel(channel)).await?;
                if target.kind != ChannelType::GuildLink {
                    return Err(err(codes::FUNCTION_ERROR, "Not a link channel."));
                }
                require_perms(c, Some(&target), Permissions::MANAGE_CHANNELS).await?;
                let updated = rest(
                    c,
                    c.ctx
                        .rest
                        .patch_channel(channel, json!({ "url": url }), reason.as_deref()),
                )
                .await?;
                Ok(channel_info(&updated))
            })
        },
    );

    d.add(
        "setContentWarning",
        "channels",
        2,
        4,
        "[canal;nível;texto?;motivo?]",
        "Sets content warning: 0 inherits, 1 enables, empty removes the override.",
        |c| {
            Box::pin(async move {
                let channel = channel_id(&c.arg(0).await?)?;
                let raw = c.arg(1).await?;
                let level = if raw.trim().is_empty() {
                    Value::Null
                } else {
                    let n = integer(&raw)?;
                    if !(0..=1).contains(&n) {
                        return Err(err(codes::FUNCTION_ERROR, format!("Invalid level: {n}.")));
                    }
                    json!(n)
                };
                let text = opt_arg(c, 2).await?;
                let reason = opt_arg(c, 3).await?;
                let target = context_channel(c, Some(channel)).await?;
                require_perms(c, Some(&target), Permissions::MANAGE_CHANNELS).await?;
                let mut body = json!({ "nsfw_override": level });
                if let Some(text) = text {
                    body["content_warning_text"] = json!(text);
                }
                rest(
                    c,
                    c.ctx.rest.patch_channel(channel, body, reason.as_deref()),
                )
                .await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "moveChannel",
        "channels",
        2,
        2,
        "[canal;posição]",
        "Moves a channel to the given position, starting at 0.",
        |c| {
            Box::pin(async move {
                let channel = channel_id(&c.arg(0).await?)?;
                let position = integer(&c.arg(1).await?)?;
                if position < 0 {
                    return Err(err(
                        codes::FUNCTION_ERROR,
                        format!("Invalid position: {position}."),
                    ));
                }
                let target = context_channel(c, Some(channel)).await?;
                let Some(guild) = target.guild_id else {
                    return Err(guard("This operation requires a guild."));
                };
                require_perms(c, Some(&target), Permissions::MANAGE_CHANNELS).await?;
                rest(
                    c,
                    c.ctx.rest.set_channel_positions(
                        guild,
                        &[ChannelPosition {
                            id: channel,
                            position: Some(position as i32),
                            ..Default::default()
                        }],
                        None,
                    ),
                )
                .await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "setCategory",
        "channels",
        2,
        3,
        "[canal;categoria;motivo?]",
        "Sets the channel category; an empty category removes the link.",
        |c| {
            Box::pin(async move {
                let channel = channel_id(&c.arg(0).await?)?;
                let category = opt_arg(c, 1).await?.map(|s| channel_id(&s)).transpose()?;
                let reason = opt_arg(c, 2).await?;
                let target = context_channel(c, Some(channel)).await?;
                if let Some(parent) = category {
                    let parent_channel = rest(c, c.ctx.rest.channel(parent)).await?;
                    if parent_channel.kind != ChannelType::GuildCategory
                        || parent_channel.guild_id != target.guild_id
                    {
                        return Err(err(
                            codes::FUNCTION_ERROR,
                            "The category must belong to the same guild as the channel.",
                        ));
                    }
                }
                require_perms(c, Some(&target), Permissions::MANAGE_CHANNELS).await?;
                let parent = match category {
                    Some(p) => json!(p.to_string()),
                    None => Value::Null,
                };
                let updated = rest(
                    c,
                    c.ctx.rest.patch_channel(
                        channel,
                        json!({ "parent_id": parent }),
                        reason.as_deref(),
                    ),
                )
                .await?;
                Ok(channel_info(&updated))
            })
        },
    );

    d.add(
        "channel",
        "channels",
        1,
        2,
        "[canal;propriedade?]",
        "Shows a channel; with a property, shows only it.",
        |c| {
            Box::pin(async move {
                let id = channel_id(&c.arg(0).await?)?;
                let channel = match found(c, c.ctx.rest.channel(id)).await? {
                    Some(channel) => channel,
                    None => return Ok(Value::Null),
                };
                let info = channel_info(&channel);
                if c.arg_count() < 2 {
                    return Ok(info);
                }
                let raw = c.arg(1).await?;
                entity_property(&info, raw.trim(), CHANNEL)
            })
        },
    );

    d.add(
        "channels",
        "channels",
        0,
        0,
        "",
        "Lists the channels of the current guild by position.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                let mut channels = rest(c, c.ctx.rest.guild_channels(guild)).await?;
                channels.sort_by_key(|channel| channel.position);
                Ok(Value::Array(channels.iter().map(channel_info).collect()))
            })
        },
    );

    d.add(
        "createChannel",
        "channels",
        1,
        3,
        "[nome;tipo?;categoria?]",
        "Creates a channel and shows its data.",
        |c| {
            Box::pin(async move {
                let name = c.arg(0).await?;
                let kind = match opt_arg(c, 1).await? {
                    Some(k) => channel_kind(&k).ok_or_else(|| {
                        err(codes::FUNCTION_ERROR, format!("Invalid channel type: {k}."))
                    })?,
                    None => 0,
                };
                let category = opt_arg(c, 2).await?.map(|s| channel_id(&s)).transpose()?;
                let guild = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_CHANNELS).await?;
                let create = ChannelCreate {
                    name,
                    kind: Some(kind as u16),
                    parent_id: category,
                    ..Default::default()
                };
                let channel = rest(c, c.ctx.rest.create_channel(guild, &create, None)).await?;
                Ok(channel_info(&channel))
            })
        },
    );

    d.add(
        "editChannel",
        "channels",
        1,
        4,
        "[canal;nome?;topico?;slowmode?]",
        "Edits a channel; empty keeps each field.",
        |c| {
            Box::pin(async move {
                let channel = channel_id(&c.arg(0).await?)?;
                let name = opt_arg(c, 1).await?;
                let topic = opt_arg(c, 2).await?;
                let slowmode = opt_arg(c, 3).await?;
                let target = context_channel(c, Some(channel)).await?;
                require_perms(c, Some(&target), Permissions::MANAGE_CHANNELS).await?;
                let mut body = json!({});
                if let Some(name) = name {
                    body["name"] = json!(name);
                }
                if let Some(topic) = topic {
                    body["topic"] = json!(topic);
                }
                if let Some(slowmode) = slowmode {
                    let seconds = integer(&slowmode)?;
                    if !(0..=21600).contains(&seconds) {
                        return Err(err(
                            codes::FUNCTION_ERROR,
                            format!("Invalid slowmode: {seconds}."),
                        ));
                    }
                    body["rate_limit_per_user"] = json!(seconds);
                }
                if body.as_object().map(|m| m.is_empty()).unwrap_or(true) {
                    return Ok(channel_info(&target));
                }
                let updated = rest(c, c.ctx.rest.patch_channel(channel, body, None)).await?;
                Ok(channel_info(&updated))
            })
        },
    );

    d.add(
        "deleteChannel",
        "channels",
        1,
        1,
        "[canal]",
        "Deletes a channel.",
        |c| {
            Box::pin(async move {
                let channel = channel_id(&c.arg(0).await?)?;
                let target = context_channel(c, Some(channel)).await?;
                require_perms(c, Some(&target), Permissions::MANAGE_CHANNELS).await?;
                rest(c, c.ctx.rest.delete_channel(channel, None)).await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "slowmode",
        "channels",
        1,
        2,
        "[segundos;canal?]",
        "Sets slowmode (0 to 21600s).",
        |c| {
            Box::pin(async move {
                let seconds = integer(&c.arg(0).await?)?;
                if !(0..=21600).contains(&seconds) {
                    return Err(err(
                        codes::FUNCTION_ERROR,
                        format!("Invalid slowmode: {seconds}."),
                    ));
                }
                let channel = opt_arg(c, 1)
                    .await?
                    .map(|s| channel_id(&s))
                    .transpose()?
                    .unwrap_or(c.ctx.channel_id());
                let target = context_channel(c, Some(channel)).await?;
                require_perms(c, Some(&target), Permissions::MANAGE_CHANNELS).await?;
                rest(c, c.ctx.rest.set_slowmode(channel, seconds as u64, None)).await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "channelPerms",
        "channels",
        2,
        4,
        "[canal;alvo;permitir?;negar?]",
        "Sets local permissions; comma-separated lists.",
        |c| {
            Box::pin(async move {
                let channel = channel_id(&c.arg(0).await?)?;
                let (target, kind) = overwrite_target(&c.arg(1).await?)?;
                let allow = match opt_arg(c, 2).await? {
                    Some(list) => super::common::permission_bits(&list)?,
                    None => 0,
                };
                let deny = match opt_arg(c, 3).await? {
                    Some(list) => super::common::permission_bits(&list)?,
                    None => 0,
                };
                let channel_obj = context_channel(c, Some(channel)).await?;
                require_perms(c, Some(&channel_obj), Permissions::MANAGE_ROLES).await?;
                rest(
                    c,
                    c.ctx
                        .rest
                        .set_channel_permission(channel, target, kind, allow, deny, None),
                )
                .await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "channelPermsOf",
        "channels",
        2,
        2,
        "[canal;usuario]",
        "Effective permissions of the user in the channel.",
        |c| {
            Box::pin(async move {
                let channel = channel_id(&c.arg(0).await?)?;
                let user = user_id(&c.arg(1).await?)?;
                let channel_obj = context_channel(c, Some(channel)).await?;
                let Some(guild_id) = channel_obj.guild_id else {
                    return Err(guard("This operation requires a guild channel."));
                };
                let guild = rest(c, c.ctx.rest.guild(guild_id)).await?;
                let member = super::common::member(c, guild_id, user).await?;
                let bits = crate::context::ScriptContext::member_permission_bits(
                    &guild,
                    Some(&channel_obj),
                    &member.roles,
                    user,
                );
                Ok(Value::String(bits.to_string()))
            })
        },
    );

    // ---- entities-channels.ts ----

    d.add(
        "typing",
        "channels",
        0,
        0,
        "",
        "Sends the typing indicator in the current channel; requires SendMessages of author and bot.",
        |c| {
            Box::pin(async move {
                let channel = context_channel(c, None).await?;
                require_perms(c, Some(&channel), Permissions::SEND_MESSAGES).await?;
                // Typing never fails a command.
                let _ = rest(c, c.ctx.rest.typing(c.ctx.channel_id())).await;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "rtcRegions",
        "channels",
        0,
        1,
        "[canal?]",
        "Lists RTC regions of the voice channel of the current guild.",
        |c| {
            Box::pin(async move {
                let channel = opt_arg(c, 0)
                    .await?
                    .map(|s| channel_id(&s))
                    .transpose()?
                    .unwrap_or(c.ctx.channel_id());
                let target = context_channel(c, Some(channel)).await?;
                let _ = target;
                let regions = rest(c, c.ctx.rest.rtc_regions(channel)).await?;
                Ok(Value::Array(regions))
            })
        },
    );

    d.add(
        "fetchSlowmode",
        "channels",
        0,
        1,
        "[canal?]",
        "Reads the current slowmode of the guild channel.",
        |c| {
            Box::pin(async move {
                let channel = opt_arg(c, 0)
                    .await?
                    .map(|s| channel_id(&s))
                    .transpose()?
                    .unwrap_or(c.ctx.channel_id());
                let _ = context_channel(c, Some(channel)).await?;
                let state = rest(c, c.ctx.rest.slowmode_state(channel)).await?;
                Ok(state)
            })
        },
    );

    d.add(
        "setDefaultForumLayout",
        "channels",
        2,
        2,
        "[canal;layout]",
        "Sets the default/list/grid forum layout; empty removes the override. Requires ManageChannels of author and bot.",
        |c| {
            Box::pin(async move {
                let channel = channel_id(&c.arg(0).await?)?;
                let layout = c.arg(1).await?;
                let value = match layout.trim() {
                    "" => Value::Null,
                    "default" => json!(0),
                    "list" => json!(1),
                    "grid" => json!(2),
                    other => {
                        return Err(err(
                            codes::FUNCTION_ERROR,
                            format!("Invalid layout: {other}."),
                        ))
                    }
                };
                let target = context_channel(c, Some(channel)).await?;
                if target.kind != ChannelType::GuildForum && target.kind != ChannelType::GuildMedia
                {
                    return Err(err(
                        codes::FUNCTION_ERROR,
                        "Default layout requires a forum channel.",
                    ));
                }
                require_perms(c, Some(&target), Permissions::MANAGE_CHANNELS).await?;
                rest(
                    c,
                    c.ctx.rest.patch_channel(
                        channel,
                        json!({ "default_forum_layout": value }),
                        None,
                    ),
                )
                .await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "deleteChannelOverwrite",
        "channels",
        2,
        3,
        "[canal;alvo;motivo?]",
        "Removes a local overwrite with ManageRoles of author and bot and target hierarchy.",
        |c| {
            Box::pin(async move {
                let channel = channel_id(&c.arg(0).await?)?;
                let (target, _kind) = overwrite_target(&c.arg(1).await?)?;
                let reason = opt_arg(c, 2).await?;
                let channel_obj = context_channel(c, Some(channel)).await?;
                require_perms(c, Some(&channel_obj), Permissions::MANAGE_ROLES).await?;
                if let Some(guild_id) = channel_obj.guild_id {
                    let guild = rest(c, c.ctx.rest.guild(guild_id)).await?;
                    let author = super::common::member(c, guild_id, c.ctx.author_id()).await?;
                    if let Ok(target_member) = rest(c, c.ctx.rest.member(guild_id, target)).await {
                        if top_role(&guild, &author) <= top_role(&guild, &target_member)
                            && c.ctx.author_id() != guild.owner_id
                        {
                            return Err(super::common::guard_reason(
                                "The target outranks the author.",
                                crate::errors::GuardReason::Hierarchy,
                            ));
                        }
                    }
                }
                rest(
                    c,
                    c.ctx
                        .rest
                        .delete_channel_permission(channel, target, reason.as_deref()),
                )
                .await?;
                Ok(Value::Null)
            })
        },
    );

    // ---- entities-guild.ts ----

    d.add(
        "setChannelPositions",
        "channels",
        1,
        1,
        "[posições]",
        "Repositions 1 to 100 channels by id/position; validates all in the current guild with ManageChannels of author and bot.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_CHANNELS).await?;
                let value = arg_value(c, 0).await?;
                let Some(list) = value.as_array() else {
                    return Err(err(codes::INVALID_ARGUMENTS, "Expected a positions array."));
                };
                if list.is_empty() || list.len() > 100 {
                    return Err(err(codes::INVALID_ARGUMENTS, "Expected 1 to 100 entries."));
                }
                let mut positions = Vec::new();
                for item in list {
                    let id = item
                        .get("id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| err(codes::INVALID_ARGUMENTS, "Missing id."))?;
                    let position = item
                        .get("position")
                        .and_then(|v| v.as_i64())
                        .ok_or_else(|| err(codes::INVALID_ARGUMENTS, "Missing position."))?;
                    positions.push(ChannelPosition {
                        id: snowflake(id, "channel")?,
                        position: Some(position as i32),
                        ..Default::default()
                    });
                }
                rest(
                    c,
                    c.ctx.rest.set_channel_positions(guild, &positions, None),
                )
                .await?;
                Ok(Value::Null)
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
