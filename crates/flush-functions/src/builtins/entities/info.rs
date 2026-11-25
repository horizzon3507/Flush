//! yuzuha `entities-info.ts`: `entityProperty` whitelists and the
//! `*Info`/`canSend`/`roleHasPermission` builtins.

use serde_json::Value;

use flush_core::Permissions;

use crate::errors::codes;
use crate::types::Definitions;

use super::common::{
    channel_id, err, guard, info_value, member, reaction_emoji, require_perms, rest, snowflake,
    user_id, Res,
};

/// Whitelists mirroring `entities-info.ts`.
pub const USER: &[&str] = &[
    "id",
    "username",
    "displayName",
    "bot",
    "avatarURL",
    "bannerURL",
    "createdTimestamp",
    "avatar",
    "banner",
    "avatarColor",
    "discriminator",
    "flags",
    "globalName",
    "system",
];
pub const MEMBER: &[&str] = &[
    "id",
    "username",
    "displayName",
    "bot",
    "avatarURL",
    "bannerURL",
    "createdTimestamp",
    "avatar",
    "banner",
    "avatarColor",
    "discriminator",
    "flags",
    "globalName",
    "system",
    "nickname",
    "joinedTimestamp",
    "roles",
    "permissions",
    "timedOutUntil",
    "guildID",
    "partial",
    "deaf",
    "mute",
    "profileFlags",
    "mentionFlags",
    "bannable",
    "kickable",
    "moderatable",
];
pub const GUILD: &[&str] = &[
    "id",
    "name",
    "ownerID",
    "iconURL",
    "bannerURL",
    "memberCount",
    "createdTimestamp",
    "features",
    "available",
    "afkChannelId",
    "afkTimeout",
    "banner",
    "bannerHeight",
    "bannerWidth",
    "icon",
    "splash",
    "splashURL",
    "splashHeight",
    "splashWidth",
    "splashCardAlignment",
    "embedSplash",
    "verificationLevel",
    "mfaLevel",
    "nsfwLevel",
    "nsfw",
    "contentWarningLevel",
    "contentWarningText",
    "disabledOperations",
    "defaultMessageNotifications",
    "explicitContentFilter",
    "messageHistoryCutoff",
    "systemChannelFlags",
    "systemChannelId",
    "rulesChannelId",
    "vanityURLCode",
    "onlineCount",
    "permissions",
];
pub const CHANNEL: &[&str] = &[
    "id",
    "name",
    "type",
    "guildID",
    "parentID",
    "position",
    "topic",
    "slowmode",
    "nsfw",
    "url",
    "recipientIDs",
    "ownerID",
    "contentWarningLevel",
    "contentWarningText",
    "createdTimestamp",
    "icon",
    "createdAt",
    "lastPinTimestamp",
    "lastMessageId",
    "nsfwOverride",
    "bitrate",
    "userLimit",
    "voiceConnectionLimit",
    "rtcRegion",
    "defaultAutoArchiveDuration",
    "defaultThreadRateLimitPerUser",
    "defaultForumLayout",
    "nicks",
    "permissionOverwrites",
];
pub const THREAD: &[&str] = &[
    "id",
    "name",
    "parentID",
    "archived",
    "locked",
    "messageCount",
    "ownerID",
    "guildID",
    "archiveTimestamp",
    "autoArchiveDuration",
    "memberCount",
    "totalMessageSent",
    "appliedTags",
    "rateLimitPerUser",
    "invitable",
    "flags",
    "lastMessageId",
    "member",
];
pub const MESSAGE: &[&str] = &[
    "id",
    "channelID",
    "authorID",
    "authorName",
    "content",
    "createdTimestamp",
    "pinned",
    "attachments",
    "embeds",
    "guildID",
    "author",
    "partial",
    "flags",
    "type",
    "tts",
    "nonce",
    "webhookId",
    "editedAt",
    "mentionEveryone",
    "mentionRoles",
    "mentions",
    "nsfwEmojis",
    "messageReference",
    "messageSnapshots",
    "stickers",
    "reactions",
    "call",
    "thread",
    "referencedMessage",
];
pub const INVITE: &[&str] = &[
    "code",
    "url",
    "guildID",
    "channelID",
    "inviterID",
    "uses",
    "maxUses",
    "expiresTimestamp",
    "type",
    "createdAt",
    "expiresAt",
    "temporary",
    "maxAge",
    "memberCount",
    "presenceCount",
    "isGuild",
    "isGroupDM",
    "inviter",
    "guildSnapshot",
    "channelSnapshot",
];
pub const WEBHOOK: &[&str] = &[
    "id",
    "name",
    "channelID",
    "guildID",
    "avatar",
    "avatarURL",
    "createdTimestamp",
    "createdAt",
    "type",
    "user",
];
pub const BAN: &[&str] = &[
    "userID",
    "username",
    "reason",
    "guildID",
    "expiresAt",
    "user",
];
pub const REACTION: &[&str] = &[
    "messageID",
    "channelID",
    "guildID",
    "count",
    "me",
    "emoji",
    "emojiIdentifier",
];

/// `property()` from `entities-info.ts` (FnError on unknown key).
pub fn property(raw: &str, allowed: &[&str]) -> Res<String> {
    let key = raw.trim();
    if !allowed.contains(&key) {
        return Err(err(
            codes::FUNCTION_ERROR,
            format!("Invalid property. Use: {}.", allowed.join(", ")),
        ));
    }
    Ok(key.to_string())
}

/// Extra aliases entities carry for property lookups.
pub fn message_info(message: &flush_core::Message) -> Value {
    let mut info = info_value(message);
    if let Some(map) = info.as_object_mut() {
        if let Some(t) = map.remove("timestamp") {
            map.insert("createdTimestamp".into(), t);
        }
        map.insert(
            "authorID".into(),
            Value::String(message.author.id.to_string()),
        );
        map.insert(
            "authorName".into(),
            Value::String(message.author.username.clone()),
        );
    }
    info
}

pub fn member_info(member: &flush_core::MemberUser, guild_id: flush_core::Snowflake) -> Value {
    let mut info = info_value(member);
    if let Some(map) = info.as_object_mut() {
        map.insert("guildID".into(), Value::String(guild_id.to_string()));
        if let Some(Value::Object(user)) = map.get("user").cloned() {
            for (k, v) in user {
                map.entry(k).or_insert(v);
            }
        }
        if let Some(nick) = map.get("nick").cloned() {
            map.insert("nickname".into(), nick);
        }
        if let Some(joined) = map.remove("joinedAt") {
            map.insert("joinedTimestamp".into(), joined);
        }
    }
    info
}

pub fn guild_info(guild: &flush_core::Guild) -> Value {
    let mut info = info_value(guild);
    if let Some(map) = info.as_object_mut() {
        if let Some(icon) = guild.icon.as_deref() {
            if let Some(url) = flush_core::cdn::guild_asset_url(
                flush_core::cdn::GuildAsset::Icon,
                guild.id,
                icon,
                None,
            ) {
                map.insert("iconURL".into(), Value::String(url));
            }
        }
        map.insert("available".into(), Value::Bool(!guild.unavailable));
        map.insert(
            "icon".into(),
            match &guild.icon {
                Some(icon) => Value::String(icon.clone()),
                None => Value::Null,
            },
        );
    }
    info
}

pub fn channel_info(channel: &flush_core::Channel) -> Value {
    let mut info = info_value(channel);
    if let Some(map) = info.as_object_mut() {
        map.insert("type".into(), Value::from(channel.kind as u16));
        if let Some(overwrites) = map.remove("permissionOverwrites") {
            map.insert("permissionOverwrites".into(), overwrites);
        }
        if let Some(slowmode) = map.get("rateLimitPerUser").cloned() {
            map.insert("slowmode".into(), slowmode);
        }
        if !channel.recipients.is_empty() {
            map.insert(
                "recipientIDs".into(),
                Value::Array(
                    channel
                        .recipients
                        .iter()
                        .map(|r| Value::String(r.id.to_string()))
                        .collect(),
                ),
            );
        }
    }
    info
}

/// Thread channel info: the channel shape plus thread fields
/// (`createdTimestamp`, `memberCount`, `messageCount`, `ownerID`,
/// `parentID`, `archived`, `autoArchiveDuration`, `locked`) already carried
/// by `channel_info`.
pub fn channel_thread_info(channel: &flush_core::Channel) -> Value {
    channel_info(channel)
}

pub fn user_info(user: &flush_core::PartialUser) -> Value {
    let mut info = info_value(user);
    if let Some(map) = info.as_object_mut() {
        if let Some(name) = map.get("globalName").cloned().filter(|v| !v.is_null()) {
            map.insert("displayName".into(), name);
        } else {
            map.insert("displayName".into(), Value::String(user.username.clone()));
        }
        map.insert(
            "avatarURL".into(),
            Value::String(flush_core::cdn::display_avatar_url(
                user.id,
                user.avatar.as_deref(),
                None,
            )),
        );
    }
    info
}

/// Reads a public field: whitelist, then `''` on missing/null.
fn public_field(info: &Value, key: &str) -> Value {
    info.get(key).cloned().unwrap_or(Value::Null)
}

pub fn register(d: &mut Definitions) {
    d.add(
        "threadInfo",
        "threads",
        2,
        2,
        "[propriedade;thread]",
        "Reads a public field of a thread in the current guild; members and tags are JSON.",
        |c| {
            Box::pin(async move {
                let prop = property(&c.arg(0).await?, THREAD)?;
                let thread = channel_id(&c.arg(1).await?)?;
                let channel = super::common::context_channel(c, Some(thread)).await?;
                let info = channel_info(&channel);
                Ok(public_field(&info, &prop))
            })
        },
    );

    d.add(
        "inviteInfo",
        "invites",
        2,
        2,
        "[propriedade;código]",
        "Reads a public invite field of the current guild; objects are JSON and null is empty.",
        |c| {
            Box::pin(async move {
                let prop = property(&c.arg(0).await?, INVITE)?;
                let code = c.arg(1).await?;
                let code = code.rsplit('/').next().unwrap_or(&code).to_string();
                let invite = rest(c, c.ctx.rest.invite(&code)).await?;
                let info = invite_info(&invite);
                Ok(public_field(&info, &prop))
            })
        },
    );

    d.add(
        "webhookInfo",
        "webhooks",
        2,
        2,
        "[propriedade;id]",
        "Reads a webhook of the current guild; ManageWebhooks of author and bot; never exposes the token.",
        |c| {
            Box::pin(async move {
                let prop = property(&c.arg(0).await?, WEBHOOK)?;
                let id = snowflake(&c.arg(1).await?, "webhook")?;
                c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_WEBHOOKS).await?;
                let mut webhook = rest(c, c.ctx.rest.webhook(id)).await?;
                webhook.token = None;
                let info = info_value(&webhook);
                Ok(public_field(&info, &prop))
            })
        },
    );

    d.add(
        "banInfo",
        "users",
        2,
        2,
        "[propriedade;usuário]",
        "Fetches a ban in the current guild; BanMembers of author and bot; empty when absent.",
        |c| {
            Box::pin(async move {
                let prop = property(&c.arg(0).await?, BAN)?;
                let user = user_id(&c.arg(1).await?)?;
                let guild = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::BAN_MEMBERS).await?;
                let ban = super::common::found(c, c.ctx.rest.ban_info(guild, user)).await?;
                let Some(ban) = ban else {
                    return Ok(Value::Null);
                };
                let info = super::common::camelize(&ban);
                Ok(public_field(&info, &prop))
            })
        },
    );

    d.add(
        "reactionInfo",
        "messages",
        3,
        4,
        "[propriedade;emoji;mensagem;canal?]",
        "Reads a reaction aggregate of a message; empty when absent, objects are JSON.",
        |c| {
            Box::pin(async move {
                let prop = property(&c.arg(0).await?, REACTION)?;
                let emoji = reaction_emoji(&c.arg(1).await?);
                let message_id = snowflake(&c.arg(2).await?, "message")?;
                let channel = match c.arg_count() > 3 {
                    true => {
                        let raw = c.arg(3).await?;
                        if raw.trim().is_empty() {
                            c.ctx.channel_id()
                        } else {
                            channel_id(&raw)?
                        }
                    }
                    false => c.ctx.channel_id(),
                };
                let message =
                    match super::common::found(c, c.ctx.rest.message(channel, message_id)).await? {
                        Some(message) => message,
                        None => return Ok(Value::Null),
                    };
                let Some(reaction) = message.reactions.iter().find(|r| {
                    r.emoji.escaped() == emoji
                        || r.emoji.name == emoji
                        || r.emoji.id.map(|id| id.to_string()).as_deref() == Some(emoji.as_str())
                }) else {
                    return Ok(Value::Null);
                };
                let mut info = info_value(reaction);
                if let Some(map) = info.as_object_mut() {
                    map.insert("messageID".into(), Value::String(message.id.to_string()));
                    map.insert(
                        "channelID".into(),
                        Value::String(message.channel_id.to_string()),
                    );
                    if let Some(g) = message.guild_id {
                        map.insert("guildID".into(), Value::String(g.to_string()));
                    }
                    map.insert(
                        "emojiIdentifier".into(),
                        Value::String(reaction.emoji.escaped()),
                    );
                }
                Ok(public_field(&info, &prop))
            })
        },
    );

    d.add(
        "canSend",
        "channels",
        0,
        2,
        "[canal?;usuário?]",
        "Checks effective ViewChannel and SendMessages of the bot or author in a guild channel.",
        |c| {
            Box::pin(async move {
                let channel_id_arg = match c.arg_count() > 0 {
                    true => opt_val(c, 0).await?,
                    false => None,
                };
                let channel = super::common::context_channel(
                    c,
                    channel_id_arg.as_deref().map(channel_id).transpose()?,
                )
                .await?;
                let Some(guild_id) = channel.guild_id else {
                    return Err(guard("This operation requires a guild channel."));
                };
                let target = match c.arg_count() > 1 {
                    true => match opt_val(c, 1).await? {
                        Some(raw) => user_id(&raw)?,
                        None => c.ctx.bot_id.unwrap_or_else(|| c.ctx.author_id()),
                    },
                    false => c.ctx.bot_id.unwrap_or_else(|| c.ctx.author_id()),
                };
                let guild = rest(c, c.ctx.rest.guild(guild_id)).await?;
                let member = member(c, guild_id, target).await?;
                let bits = crate::context::ScriptContext::member_permission_bits(
                    &guild,
                    Some(&channel),
                    &member.roles,
                    target,
                );
                let needed = Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES;
                Ok(Value::Bool(
                    bits & Permissions::ADMINISTRATOR == Permissions::ADMINISTRATOR
                        || bits & needed == needed,
                ))
            })
        },
    );

    d.add(
        "roleHasPermission",
        "roles",
        2,
        2,
        "[cargo;permissão]",
        "Checks one permission of a role in the current guild, respecting Administrator.",
        |c| {
            Box::pin(async move {
                let role = super::common::role_id(&c.arg(0).await?)?;
                let name = c.arg(1).await?;
                let guild = c.ctx.require_guild()?;
                let bit = super::common::permission_bit(name.trim())
                    .ok_or_else(|| guard(format!("Unknown permission: {name}.")))?;
                let role = match rest(c, c.ctx.rest.role(guild, role)).await? {
                    Some(role) => role,
                    None => return Ok(Value::Bool(false)),
                };
                let bits = role.permissions;
                Ok(Value::Bool(
                    bits & Permissions::ADMINISTRATOR == Permissions::ADMINISTRATOR
                        || bits & bit == bit,
                ))
            })
        },
    );
}

async fn opt_val(c: &crate::call::ScriptCall<'_>, i: usize) -> Res<Option<String>> {
    Ok(super::common::opt(c.arg(i).await?))
}

/// Invite → the yuzuha invite info object (`Invite` is wire-decode only).
pub fn invite_info(invite: &flush_core::Invite) -> Value {
    use serde_json::json;
    let guild = invite.guild.as_ref().map(|g| g.id.to_string());
    let channel = invite.channel.as_ref().map(|c| c.id.to_string());
    let inviter = invite.inviter.as_ref().map(|u| u.id.to_string());
    let expires_ts = invite.expires_at.as_deref().and_then(chrono_like_ms);
    let mut map = serde_json::Map::new();
    map.insert("code".into(), json!(invite.code));
    map.insert(
        "url".into(),
        json!(format!("https://fluxer.gg/invite/{}", invite.code)),
    );
    map.insert("guildID".into(), json!(guild));
    map.insert("channelID".into(), json!(channel));
    map.insert("inviterID".into(), json!(inviter));
    map.insert("uses".into(), json!(invite.uses));
    map.insert("maxUses".into(), json!(invite.max_uses));
    map.insert("expiresTimestamp".into(), json!(expires_ts));
    map.insert("type".into(), json!("guild"));
    map.insert("createdAt".into(), json!(invite.created_at));
    map.insert("expiresAt".into(), json!(invite.expires_at));
    map.insert("temporary".into(), json!(invite.temporary));
    map.insert("maxAge".into(), json!(invite.max_age));
    map.insert("memberCount".into(), json!(invite.approximate_member_count));
    map.insert(
        "presenceCount".into(),
        json!(invite.approximate_online_count),
    );
    map.insert("isGuild".into(), json!(guild.is_some()));
    map.insert(
        "isGroupDM".into(),
        json!(invite
            .channel
            .as_ref()
            .map(|c| c.kind == flush_core::channel::ChannelType::GroupDm)
            .unwrap_or(false)),
    );
    map.insert(
        "inviter".into(),
        invite
            .inviter
            .as_ref()
            .map(user_info)
            .unwrap_or(Value::Null),
    );
    map.insert(
        "guildSnapshot".into(),
        invite.guild.as_ref().map(guild_info).unwrap_or(Value::Null),
    );
    map.insert(
        "channelSnapshot".into(),
        invite
            .channel
            .as_ref()
            .map(channel_info)
            .unwrap_or(Value::Null),
    );
    Value::Object(map)
}

/// ISO-8601 string → epoch ms, best effort.
fn chrono_like_ms(s: &str) -> Option<u64> {
    let parse = |fmt: &str| chrono_free_parse(s, fmt);
    parse("")
}

/// Minimal RFC-3339 parse without chrono (seconds precision only).
fn chrono_free_parse(s: &str, _unused: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |a: usize, e: usize| -> Option<i64> { s.get(a..e)?.parse().ok() };
    let (y, mo, d, h, mi, se) = (
        num(0, 4)?,
        num(5, 7)?,
        num(8, 10)?,
        num(11, 13)?,
        num(14, 16)?,
        num(17, 19)?,
    );
    // days from civil (Howard Hinnant).
    let (y, mo) = if mo <= 2 { (y - 1, mo + 12) } else { (y, mo) };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (mo - 3) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let secs = days * 86400 + h * 3600 + mi * 60 + se;
    if secs < 0 {
        return None;
    }
    Some(secs as u64 * 1000)
}
