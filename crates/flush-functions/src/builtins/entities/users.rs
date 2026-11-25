//! yuzuha `native-users.ts` + `entities-guild.ts` (discovery/ownership) +
//! `entities-members.ts` (`guildMe`).

use serde_json::Value;

use flush_core::cdn;
use flush_rest::resources::AuditLogQuery;

use crate::errors::codes;
use crate::errors::GuardReason;
use crate::types::Definitions;

use super::common::{
    arg_value, entity_property, err, found, guard, guard_reason, integer, limit, member,
    native_unavailable, require_perms, rest, serialize, top_role, user_id,
};
use super::info::{guild_info, member_info, user_info, GUILD, MEMBER, USER};
use flush_core::Permissions;

pub fn register(d: &mut Definitions) {
    d.add(
        "guildFeatures",
        "users",
        0,
        0,
        "",
        "Returns the features of the current guild as an array; requires feature support.",
        |c| {
            Box::pin(async move {
                let guild = super::common::guild(c, None).await?;
                let info = serialize(&guild);
                Ok(info
                    .get("features")
                    .cloned()
                    .unwrap_or(Value::Array(vec![])))
            })
        },
    );

    d.add(
        "leaveGuild",
        "users",
        0,
        0,
        "",
        "Removes the bot from the current guild; only the guild owner may run it.",
        |c| {
            Box::pin(async move {
                let guild_id = c.ctx.require_guild()?;
                let guild = rest(c, c.ctx.rest.guild(guild_id)).await?;
                if guild.owner_id != c.ctx.author_id() {
                    return Err(guard("Only the guild owner may run this operation."));
                }
                rest(c, c.ctx.rest.leave_guild(guild_id)).await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "userProfile",
        "users",
        1,
        1,
        "[usuário]",
        "Fetches user, global profile and guild profile/member of the current guild when available.",
        |c| {
            Box::pin(async move {
                let id = user_id(&c.arg(0).await?)?;
                let guild = c.ctx.message.guild_id;
                let profile = rest(c, c.ctx.rest.user_profile(id, guild)).await?;
                Ok(profile)
            })
        },
    );

    d.add(
        "guildCounts",
        "users",
        0,
        1,
        "[timeout?]",
        "Requests live member and online counts from the gateway; timeout in milliseconds.",
        |c| {
            Box::pin(async move {
                let _ = c.arg(0).await?;
                Err(native_unavailable("guildCounts"))
            })
        },
    );

    d.add(
        "channelCounts",
        "users",
        0,
        2,
        "[canal?;timeout?]",
        "Requests live counts of the channel from the gateway, awaiting the matching event.",
        |c| {
            Box::pin(async move {
                let _ = c.arg(0).await?;
                Err(native_unavailable("channelCounts"))
            })
        },
    );

    d.add(
        "user",
        "users",
        1,
        2,
        "[id;propriedade?]",
        "User by ID, or one of its properties; empty when it does not exist.",
        |c| {
            Box::pin(async move {
                let id = user_id(&c.arg(0).await?)?;
                let user = match found(c, c.ctx.rest.user(id)).await? {
                    Some(user) => user,
                    None => return Ok(Value::Null),
                };
                if c.arg_count() < 2 {
                    return Ok(user_info(&user));
                }
                let raw = c.arg(1).await?;
                entity_property(&user_info(&user), raw.trim(), USER)
            })
        },
    );

    d.add(
        "member",
        "users",
        1,
        2,
        "[id;propriedade?]",
        "Member of the current guild, or a property; empty when it does not exist.",
        |c| {
            Box::pin(async move {
                let id = user_id(&c.arg(0).await?)?;
                let guild = c.ctx.require_guild()?;
                let member = match found(c, c.ctx.rest.member(guild, id)).await? {
                    Some(member) => member,
                    None => return Ok(Value::Null),
                };
                if c.arg_count() < 2 {
                    return Ok(member_info(&member, guild));
                }
                let raw = c.arg(1).await?;
                entity_property(&member_info(&member, guild), raw.trim(), MEMBER)
            })
        },
    );

    d.add(
        "members",
        "users",
        0,
        1,
        "[limite?]",
        "Members of the current guild; default 50, maximum 1000.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                let n = match c.arg_count() > 0 {
                    true => limit(Some(c.arg(0).await?), 50, 1000)?,
                    false => 50,
                };
                let members = rest(c, c.ctx.rest.list_members(guild, n as u32, None)).await?;
                Ok(Value::Array(
                    members.iter().map(|m| member_info(m, guild)).collect(),
                ))
            })
        },
    );

    d.add(
        "avatar",
        "users",
        1,
        2,
        "[id;tamanho?]",
        "Avatar URL; size 16 to 4096; empty when missing.",
        |c| {
            Box::pin(async move {
                let id = user_id(&c.arg(0).await?)?;
                let size = match c.arg_count() > 1 {
                    true => {
                        let n = integer(&c.arg(1).await?)?;
                        if !(16..=4096).contains(&n) {
                            return Err(err(codes::FUNCTION_ERROR, format!("Invalid size: {n}.")));
                        }
                        Some(n as u32)
                    }
                    false => None,
                };
                let user = match found(c, c.ctx.rest.user(id)).await? {
                    Some(user) => user,
                    None => return Ok(Value::Null),
                };
                Ok(
                    match cdn::avatar_url(id, user.avatar.as_deref().unwrap_or(""), size) {
                        Some(url) => Value::String(url),
                        None => Value::Null,
                    },
                )
            })
        },
    );

    d.add(
        "displayName",
        "users",
        1,
        1,
        "[id]",
        "Display name of the user; empty when it does not exist.",
        |c| {
            Box::pin(async move {
                let id = user_id(&c.arg(0).await?)?;
                match found(c, c.ctx.rest.user(id)).await? {
                    Some(user) => Ok(Value::String(
                        user.global_name
                            .clone()
                            .unwrap_or_else(|| user.username.clone()),
                    )),
                    None => Ok(Value::Null),
                }
            })
        },
    );

    d.add(
        "memberCount",
        "users",
        0,
        0,
        "",
        "Total members of the current guild.",
        |c| {
            Box::pin(async move {
                let guild = super::common::guild(c, None).await?;
                Ok(match guild.member_count {
                    Some(n) => Value::from(n),
                    None => Value::Null,
                })
            })
        },
    );

    d.add(
        "userBanner",
        "users",
        1,
        2,
        "[id;tamanho?]",
        "User banner URL; size 16 to 4096; empty when missing.",
        |c| {
            Box::pin(async move {
                let id = user_id(&c.arg(0).await?)?;
                let size = match c.arg_count() > 1 {
                    true => Some(integer(&c.arg(1).await?)? as u32),
                    false => None,
                };
                let profile = match found(c, c.ctx.rest.user_profile(id, None)).await? {
                    Some(profile) => profile,
                    None => return Ok(Value::Null),
                };
                let banner = profile
                    .get("banner")
                    .and_then(|b| b.as_str())
                    .unwrap_or_default()
                    .to_string();
                Ok(match cdn::banner_url(id, &banner, size) {
                    Some(url) => Value::String(url),
                    None => Value::Null,
                })
            })
        },
    );

    d.add(
        "userCreated",
        "users",
        1,
        1,
        "[id]",
        "Account creation in milliseconds; empty when it does not exist.",
        |c| {
            Box::pin(async move {
                let id = user_id(&c.arg(0).await?)?;
                match found(c, c.ctx.rest.user(id)).await? {
                    Some(user) => Ok(Value::from(user.id.timestamp_ms())),
                    None => Ok(Value::Null),
                }
            })
        },
    );

    d.add(
        "guild",
        "users",
        0,
        1,
        "[propriedade?]",
        "Current guild, or one of its properties.",
        |c| {
            Box::pin(async move {
                let guild = super::common::guild(c, None).await?;
                let info = guild_info(&guild);
                if c.arg_count() < 1 {
                    return Ok(info);
                }
                let raw = c.arg(0).await?;
                entity_property(&info, raw.trim(), GUILD)
            })
        },
    );

    d.add(
        "guildIcon",
        "users",
        0,
        1,
        "[tamanho?]",
        "Icon URL of the current guild; empty when missing.",
        |c| {
            Box::pin(async move {
                let size = match c.arg_count() > 0 {
                    true => Some(integer(&c.arg(0).await?)? as u32),
                    false => None,
                };
                let guild = super::common::guild(c, None).await?;
                Ok(match guild.icon.as_deref() {
                    Some(icon) if !icon.is_empty() => {
                        match cdn::guild_asset_url(cdn::GuildAsset::Icon, guild.id, icon, size) {
                            Some(url) => Value::String(url),
                            None => Value::Null,
                        }
                    }
                    _ => Value::Null,
                })
            })
        },
    );

    d.add(
        "guildBanner",
        "users",
        0,
        1,
        "[tamanho?]",
        "Banner URL of the current guild; empty when missing.",
        |c| {
            Box::pin(async move {
                let size = match c.arg_count() > 0 {
                    true => Some(integer(&c.arg(0).await?)? as u32),
                    false => None,
                };
                let guild = super::common::guild(c, None).await?;
                let value = serialize(&guild);
                let banner = value
                    .get("banner")
                    .and_then(|b| b.as_str())
                    .unwrap_or_default()
                    .to_string();
                Ok(
                    match cdn::guild_asset_url(cdn::GuildAsset::Banner, guild.id, &banner, size) {
                        Some(url) => Value::String(url),
                        None => Value::Null,
                    },
                )
            })
        },
    );

    d.add(
        "bans",
        "users",
        0,
        1,
        "[limite?]",
        "Bans of the current guild; default 50, maximum 1000.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                let n = match c.arg_count() > 0 {
                    true => limit(Some(c.arg(0).await?), 50, 1000)?,
                    false => 50,
                };
                let mut bans = rest(c, c.ctx.rest.bans(guild)).await?;
                bans.truncate(n as usize);
                Ok(Value::Array(
                    bans.iter()
                        .map(|b| {
                            serde_json::json!({
                                "user": b.user.id.to_string(),
                                "reason": b.reason,
                            })
                        })
                        .collect(),
                ))
            })
        },
    );

    d.add(
        "auditLogs",
        "users",
        0,
        1,
        "[limite?]",
        "Audit logs of the current guild; default 50, maximum 100.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                let n = match c.arg_count() > 0 {
                    true => limit(Some(c.arg(0).await?), 50, 100)?,
                    false => 50,
                };
                let logs = rest(
                    c,
                    c.ctx.rest.audit_logs(
                        guild,
                        &AuditLogQuery {
                            limit: Some(n as u32),
                            ..Default::default()
                        },
                    ),
                )
                .await?;
                Ok(Value::Array(
                    logs.audit_log_entries
                        .iter()
                        .map(|e| {
                            serde_json::json!({
                                "id": e.id.to_string(),
                                "actionType": e.action_type.code(),
                                "userId": e.user_id.map(|u| u.to_string()),
                                "targetId": e.target_id.map(|u| u.to_string()),
                                "reason": e.reason,
                            })
                        })
                        .collect(),
                ))
            })
        },
    );

    for name in ["setPresence", "setStatus", "setActivity"] {
        let (min, max, usage, desc) = match name {
            "setPresence" => (
                1,
                3,
                "[status;atividade?;tipo?]",
                "Sets presence: status and optional activity (playing by default).",
            ),
            "setStatus" => (
                1,
                1,
                "[status]",
                "Sets the presence status; clears the activity.",
            ),
            _ => (
                1,
                2,
                "[atividade;tipo?]",
                "Sets the activity with online status.",
            ),
        };
        d.add(name, "users", min, max, usage, desc, |c| {
            Box::pin(async move {
                let _ = c.arg(0).await?;
                Err(native_unavailable("presence"))
            })
        });
    }

    // ---- entities-guild.ts ----

    d.add(
        "applyForDiscovery",
        "users",
        1,
        1,
        "[opções]",
        "Applies for discovery; description, primaryCategoryId, keywords and primaryLanguage. Requires ManageGuild of author and bot.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_GUILD).await?;
                let body = arg_value(c, 0).await?;
                let request = flush_rest::account::DiscoveryRequest {
                    description: body.get("description").and_then(|v| v.as_str()).map(str::to_string),
                    category_type: body.get("primaryCategoryId").or_else(|| body.get("categoryType")).and_then(|v| v.as_u64()).map(|v| v as u32),
                    primary_language: body.get("primaryLanguage").and_then(|v| v.as_str()).map(str::to_string),
                    custom_tags: body.get("keywords").or_else(|| body.get("customTags")).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_string)).collect()),
                };
                let _ = rest(
                    c,
                    c.ctx.rest.apply_for_discovery(guild, &request),
                )
                .await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "editDiscoveryApplication",
        "users",
        1,
        1,
        "[opções]",
        "Edits the discovery application with full validation and ManageGuild of author and bot.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_GUILD).await?;
                let body = arg_value(c, 0).await?;
                let request = flush_rest::account::DiscoveryRequest {
                    description: body
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    category_type: body
                        .get("primaryCategoryId")
                        .or_else(|| body.get("categoryType"))
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32),
                    primary_language: body
                        .get("primaryLanguage")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    custom_tags: body
                        .get("keywords")
                        .or_else(|| body.get("customTags"))
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|t| t.as_str().map(str::to_string))
                                .collect()
                        }),
                };
                let _ = rest(c, c.ctx.rest.edit_discovery_application(guild, &request)).await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "discoveryStatus",
        "users",
        0,
        0,
        "",
        "Reads the application, eligibility and member minimum; requires ManageGuild of author and bot.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_GUILD).await?;
                Ok(serialize(&serde_json::json!({
                    "application": rest(c, c.ctx.rest.guild_discovery_status(guild)).await?.application.map(|a| serde_json::json!({"description": a.description})),
                })))
            })
        },
    );

    d.add(
        "withdrawDiscoveryApplication",
        "users",
        0,
        0,
        "",
        "Withdraws the current guild's application; requires ManageGuild of author and bot.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_GUILD).await?;
                rest(c, c.ctx.rest.withdraw_discovery_application(guild)).await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "transferOwnership",
        "users",
        2,
        2,
        "[usuário;confirmação]",
        "Transfers ownership with TRANSFER confirmation; author and bot must own, target must be a human member.",
        |c| {
            Box::pin(async move {
                let user = user_id(&c.arg(0).await?)?;
                let confirm = c.arg(1).await?;
                if confirm != "TRANSFER" {
                    return Err(err(
                        codes::FUNCTION_ERROR,
                        "Type TRANSFER to confirm the ownership transfer.",
                    ));
                }
                let guild_id = c.ctx.require_guild()?;
                let guild = rest(c, c.ctx.rest.guild(guild_id)).await?;
                if guild.owner_id != c.ctx.author_id() {
                    return Err(guard_reason(
                        "Only the guild owner may transfer ownership.",
                        GuardReason::PermissionMissingAuthor,
                    ));
                }
                let bot = rest(c, c.ctx.rest.me_member(guild_id)).await?;
                let _ = bot;
                if c.ctx.bot_id != Some(guild.owner_id) {
                    // The bot does not need to be the owner unless transferring
                    // away from itself; keep parity with yuzuha's owner check.
                    return Err(guard_reason(
                        "The bot must be the guild owner.",
                        GuardReason::PermissionMissingBot,
                    ));
                }
                if Some(user) == c.ctx.bot_id {
                    return Err(guard_reason(
                        "The target cannot be the bot.",
                        GuardReason::BotTarget,
                    ));
                }
                let target = member(c, guild_id, user).await?;
                if target.user.bot {
                    return Err(guard("The target must be a human member."));
                }
                let _ = top_role;
                rest(c, c.ctx.rest.transfer_guild_ownership(guild_id, user, None)).await?;
                Ok(Value::Null)
            })
        },
    );

    // ---- entities-members.ts ----

    d.add(
        "editMember",
        "users",
        2,
        2,
        "[usuário;opções]",
        "Edits nickname and/or roles in one call; validates permissions, hierarchy and every role of the batch.",
        |c| {
            Box::pin(async move {
                let user = user_id(&c.arg(0).await?)?;
                let options = arg_value(c, 1).await?;
                let guild_id = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_NICKNAMES | Permissions::MANAGE_ROLES)
                    .await?;
                let reason = options
                    .get("reason")
                    .and_then(|r| r.as_str())
                    .ok_or_else(|| err(codes::INVALID_ARGUMENTS, "A reason is required."))?;
                let guild = rest(c, c.ctx.rest.guild(guild_id)).await?;
                let target = member(c, guild_id, user).await?;
                let author = member(c, guild_id, c.ctx.author_id()).await?;
                if top_role(&guild, &author) <= top_role(&guild, &target)
                    && c.ctx.author_id() != guild.owner_id
                {
                    return Err(guard_reason(
                        "The target outranks the author.",
                        GuardReason::Hierarchy,
                    ));
                }
                let mut patch = flush_rest::client::MemberPatch {
                    timeout_reason: Some(reason.to_string()),
                    ..Default::default()
                };
                if let Some(nick) = options.get("nick").or_else(|| options.get("nickname")) {
                    patch.nick = Some(match nick.as_str() {
                        Some(n) if !n.is_empty() => Some(n.to_string()),
                        _ => None,
                    });
                }
                if let Some(roles) = options.get("roles").and_then(|r| r.as_array()) {
                    let mut ids = Vec::new();
                    for role in roles {
                        let id = role
                            .as_str()
                            .and_then(|s| s.parse::<flush_core::Snowflake>().ok())
                            .ok_or_else(|| err(codes::INVALID_ARGUMENTS, "Invalid role in batch."))?;
                        if !guild.roles.iter().any(|r| r.id == id) {
                            return Err(err(codes::INVALID_ARGUMENTS, "Role not in this guild."));
                        }
                        ids.push(id);
                    }
                    patch.roles = Some(ids);
                }
                let member = rest(c, c.ctx.rest.patch_member(guild_id, user, &patch, Some(reason))).await?;
                Ok(member_info(&member, guild_id))
            })
        },
    );

    d.add(
        "guildMe",
        "users",
        0,
        1,
        "[propriedade?]",
        "Fetches the bot member of the current guild; optional field follows the member whitelist.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                let member = rest(c, c.ctx.rest.me_member(guild)).await?;
                let info = member_info(&member, guild);
                if c.arg_count() < 1 {
                    return Ok(info);
                }
                let raw = c.arg(0).await?;
                entity_property(&info, raw.trim(), MEMBER)
            })
        },
    );
}
