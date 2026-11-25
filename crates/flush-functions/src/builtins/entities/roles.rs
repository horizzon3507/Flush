//! yuzuha `native-roles.ts` + `entities-members.ts` (`setMemberRoles`) +
//! `entities-guild.ts` (role position functions).

use serde_json::Value;

use flush_core::Permissions;
use flush_rest::account::RoleHoistPosition;
use flush_rest::client::{MemberPatch, RolePatch};
use flush_rest::resources::RolePosition;

use crate::errors::{codes, GuardReason};
use crate::types::Definitions;

use super::common::{
    arg_value, entity_property, err, found, guard_reason, integer, member, opt, permission_bits,
    require_perms, rest, role_id, serialize, top_role, user_id, Res,
};

const ROLE: &[&str] = &[
    "id",
    "name",
    "color",
    "position",
    "hoistPosition",
    "permissions",
    "hoist",
    "mentionable",
    "guildID",
    "createdTimestamp",
    "managed",
    "unicodeEmoji",
    "icon",
    "tags",
];

fn role_info(role: &flush_core::Role, guild_id: flush_core::Snowflake) -> Value {
    let mut info = super::common::camelize(&serialize(role));
    if let Some(map) = info.as_object_mut() {
        map.insert("guildID".into(), Value::String(guild_id.to_string()));
        map.insert("hoistPosition".into(), Value::from(role.hoist_position));
    }
    info
}

/// Resolves a role by id or name inside `roles` (yuzuha `resolveRole`).
async fn find_role(c: &crate::call::ScriptCall<'_>, raw: &str) -> Res<flush_core::Role> {
    let guild = c.ctx.require_guild()?;
    let roles = rest(c, c.ctx.rest.roles(guild)).await?;
    if let Ok(id) = role_id(raw) {
        if let Some(role) = roles.iter().find(|r| r.id == id) {
            return Ok(role.clone());
        }
    }
    roles
        .into_iter()
        .find(|r| r.name == raw)
        .ok_or_else(|| err(codes::FUNCTION_ERROR, format!("Role not found: {raw}.")))
}

/// Author top role must be above `role`'s position.
async fn check_hierarchy(c: &crate::call::ScriptCall<'_>, role_position: i64) -> Res<()> {
    let guild_id = c.ctx.require_guild()?;
    let guild = rest(c, c.ctx.rest.guild(guild_id)).await?;
    let author = member(c, guild_id, c.ctx.author_id()).await?;
    if c.ctx.author_id() == guild.owner_id {
        return Ok(());
    }
    if top_role(&guild, &author) <= role_position {
        return Err(guard_reason(
            "The role is at or above the author's top role.",
            GuardReason::Hierarchy,
        ));
    }
    let bot = rest(c, c.ctx.rest.me_member(guild_id)).await?;
    if top_role(&guild, &bot) <= role_position {
        return Err(guard_reason(
            "The role is at or above the bot's top role.",
            GuardReason::Hierarchy,
        ));
    }
    Ok(())
}

fn parse_color(raw: &str) -> Res<u32> {
    let s = raw.trim().trim_start_matches('#');
    u32::from_str_radix(s, 16)
        .or_else(|_| raw.trim().parse::<u32>())
        .map_err(|_| err(codes::FUNCTION_ERROR, format!("Invalid color: {raw}.")))
}

fn check_name(name: &str) -> Res<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 100 {
        return Err(err(
            codes::FUNCTION_ERROR,
            "Role name must be 1 to 100 characters.",
        ));
    }
    Ok(trimmed.to_string())
}

/// Validates a `[{id, position}]` positions batch (positionBatch).
fn position_batch(value: &Value, field: &str) -> Res<Vec<(flush_core::Snowflake, Option<i64>)>> {
    let Some(list) = value.as_array() else {
        return Err(err(codes::INVALID_ARGUMENTS, "Expected a positions array."));
    };
    if list.is_empty() || list.len() > 100 {
        return Err(err(codes::INVALID_ARGUMENTS, "Expected 1 to 100 entries."));
    }
    let mut out = Vec::new();
    for item in list {
        let id = item
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| err(codes::INVALID_ARGUMENTS, "Missing id."))?;
        let position = match item.get(field) {
            Some(Value::Null) | None => None,
            Some(v) => Some(
                v.as_i64()
                    .ok_or_else(|| err(codes::INVALID_ARGUMENTS, "Invalid position."))?,
            ),
        };
        out.push((
            id.parse::<flush_core::Snowflake>()
                .map_err(|_| err(codes::INVALID_ARGUMENTS, "Invalid id."))?,
            position,
        ));
    }
    Ok(out)
}

pub fn register(d: &mut Definitions) {
    d.add(
        "moveRole",
        "roles",
        2,
        2,
        "[cargo;posição]",
        "Moves a role with author and bot authorization and hierarchy; position starts at 1.",
        |c| {
            Box::pin(async move {
                let role = find_role(c, &c.arg(0).await?).await?;
                let position = integer(&c.arg(1).await?)?;
                if position < 1 {
                    return Err(err(
                        codes::FUNCTION_ERROR,
                        format!("Invalid position: {position}."),
                    ));
                }
                let guild = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_ROLES).await?;
                check_hierarchy(c, role.position as i64).await?;
                rest(
                    c,
                    c.ctx.rest.set_role_positions(
                        guild,
                        &[RolePosition {
                            id: role.id,
                            position: Some(position as i32),
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
        "role",
        "roles",
        1,
        2,
        "[cargo;propriedade?]",
        "Resolves a role of the current guild; returns the role or a property.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                let role = find_role(c, &c.arg(0).await?).await?;
                let info = role_info(&role, guild);
                if c.arg_count() < 2 {
                    return Ok(info);
                }
                let raw = c.arg(1).await?;
                entity_property(&info, raw.trim(), ROLE)
            })
        },
    );

    d.add(
        "roles",
        "roles",
        0,
        0,
        "",
        "Lists roles of the guild ordered by position.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                let mut roles = rest(c, c.ctx.rest.roles(guild)).await?;
                roles.sort_by_key(|r| r.position);
                Ok(Value::Array(
                    roles.iter().map(|r| role_info(r, guild)).collect(),
                ))
            })
        },
    );

    d.add(
        "hasRole",
        "roles",
        2,
        2,
        "[usuário;cargo]",
        "Checks whether the user has the role.",
        |c| {
            Box::pin(async move {
                let user = user_id(&c.arg(0).await?)?;
                let guild = c.ctx.require_guild()?;
                let member = match found(c, c.ctx.rest.member(guild, user)).await? {
                    Some(member) => member,
                    None => return Ok(Value::Bool(false)),
                };
                let role = find_role(c, &c.arg(1).await?).await?;
                Ok(Value::Bool(member.roles.contains(&role.id)))
            })
        },
    );

    d.add(
        "addRole",
        "roles",
        2,
        2,
        "[usuário;cargo]",
        "Adds a role to the user.",
        |c| {
            Box::pin(async move {
                let user = user_id(&c.arg(0).await?)?;
                let role = find_role(c, &c.arg(1).await?).await?;
                c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_ROLES).await?;
                check_hierarchy(c, role.position as i64).await?;
                let guild = c.ctx.require_guild()?;
                rest(c, c.ctx.rest.add_role(guild, user, role.id, None)).await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "removeRole",
        "roles",
        2,
        2,
        "[usuário;cargo]",
        "Removes a role from the user.",
        |c| {
            Box::pin(async move {
                let user = user_id(&c.arg(0).await?)?;
                let role = find_role(c, &c.arg(1).await?).await?;
                c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_ROLES).await?;
                check_hierarchy(c, role.position as i64).await?;
                let guild = c.ctx.require_guild()?;
                rest(c, c.ctx.rest.remove_role(guild, user, role.id, None)).await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "createRole",
        "roles",
        1,
        3,
        "[nome;cor?;permissões?]",
        "Creates a role; color #RRGGBB or number; permissions comma-separated.",
        |c| {
            Box::pin(async move {
                let name = check_name(&c.arg(0).await?)?;
                let color = opt_arg(c, 1).await?.map(|s| parse_color(&s)).transpose()?;
                let permissions = match opt_arg(c, 2).await? {
                    Some(list) => Some(permission_bits(&list)?),
                    None => None,
                };
                let guild = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_ROLES).await?;
                let role = rest(
                    c,
                    c.ctx.rest.create_role(
                        guild,
                        &RolePatch {
                            name: Some(name),
                            color,
                            permissions,
                            ..Default::default()
                        },
                        None,
                    ),
                )
                .await?;
                Ok(role_info(&role, guild))
            })
        },
    );

    d.add(
        "editRole",
        "roles",
        1,
        4,
        "[cargo;nome?;cor?;permissões?]",
        "Edits a role; empty keeps each field.",
        |c| {
            Box::pin(async move {
                let role = find_role(c, &c.arg(0).await?).await?;
                let name = opt_arg(c, 1).await?.map(|s| check_name(&s)).transpose()?;
                let color = opt_arg(c, 2).await?.map(|s| parse_color(&s)).transpose()?;
                let permissions = match opt_arg(c, 3).await? {
                    Some(list) => Some(permission_bits(&list)?),
                    None => None,
                };
                let guild = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_ROLES).await?;
                check_hierarchy(c, role.position as i64).await?;
                let patch = RolePatch {
                    name,
                    color,
                    permissions,
                    ..Default::default()
                };
                let updated = rest(c, c.ctx.rest.edit_role(guild, role.id, &patch, None)).await?;
                Ok(role_info(&updated, guild))
            })
        },
    );

    d.add(
        "deleteRole",
        "roles",
        1,
        1,
        "[cargo]",
        "Deletes a role from the guild.",
        |c| {
            Box::pin(async move {
                let role = find_role(c, &c.arg(0).await?).await?;
                let guild = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_ROLES).await?;
                check_hierarchy(c, role.position as i64).await?;
                rest(c, c.ctx.rest.delete_role(guild, role.id, None)).await?;
                Ok(Value::Null)
            })
        },
    );

    // ---- entities-members.ts ----

    d.add(
        "setMemberRoles",
        "roles",
        3,
        3,
        "[usuário;cargos;motivo]",
        "Replaces a member's roles atomically; up to 100 IDs as texts, empty array clears. Requires ManageRoles and hierarchy.",
        |c| {
            Box::pin(async move {
                let user = user_id(&c.arg(0).await?)?;
                let value = arg_value(c, 1).await?;
                let reason = c.arg(2).await?;
                let Some(list) = value.as_array() else {
                    return Err(err(codes::INVALID_ARGUMENTS, "Expected a roles array."));
                };
                if list.len() > 100 {
                    return Err(err(codes::INVALID_ARGUMENTS, "Expected up to 100 roles."));
                }
                let guild_id = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_ROLES).await?;
                let guild = rest(c, c.ctx.rest.guild(guild_id)).await?;
                let author = member(c, guild_id, c.ctx.author_id()).await?;
                let mut ids = Vec::new();
                for item in list {
                    let id = item
                        .as_str()
                        .and_then(|s| s.parse::<flush_core::Snowflake>().ok())
                        .ok_or_else(|| err(codes::INVALID_ARGUMENTS, "Invalid role id."))?;
                    let Some(role) = guild.roles.iter().find(|r| r.id == id) else {
                        return Err(err(codes::INVALID_ARGUMENTS, "Role not in this guild."));
                    };
                    if top_role(&guild, &author) <= i64::from(role.position)
                        && c.ctx.author_id() != guild.owner_id
                    {
                        return Err(guard_reason(
                            "A role outranks the author.",
                            GuardReason::Hierarchy,
                        ));
                    }
                    ids.push(id);
                }
                let target = member(c, guild_id, user).await?;
                if top_role(&guild, &author) <= top_role(&guild, &target)
                    && c.ctx.author_id() != guild.owner_id
                {
                    return Err(guard_reason(
                        "The target outranks the author.",
                        GuardReason::Hierarchy,
                    ));
                }
                let patch = MemberPatch {
                    roles: Some(ids),
                    timeout_reason: Some(reason.clone()),
                    ..Default::default()
                };
                let updated = rest(
                    c,
                    c.ctx.rest.patch_member(guild_id, user, &patch, Some(&reason)),
                )
                .await?;
                Ok(super::info::member_info(&updated, guild_id))
            })
        },
    );

    // ---- entities-guild.ts ----

    d.add(
        "setRolePositions",
        "roles",
        1,
        1,
        "[posições]",
        "Repositions 1 to 100 roles by id/position; validates the batch, permissions and full hierarchy before the API.",
        |c| {
            Box::pin(async move {
                let guild_id = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_ROLES).await?;
                let batch = position_batch(&arg_value(c, 0).await?, "position")?;
                let guild = rest(c, c.ctx.rest.guild(guild_id)).await?;
                let author = member(c, guild_id, c.ctx.author_id()).await?;
                let positions: Vec<RolePosition> = batch
                    .iter()
                    .map(|(id, position)| {
                        let Some(role) = guild.roles.iter().find(|r| r.id == *id) else {
                            return Err(err(codes::INVALID_ARGUMENTS, "Role not in this guild."));
                        };
                        if top_role(&guild, &author) <= i64::from(role.position)
                            && c.ctx.author_id() != guild.owner_id
                        {
                            return Err(guard_reason(
                                "A role outranks the author.",
                                GuardReason::Hierarchy,
                            ));
                        }
                        Ok(RolePosition {
                            id: *id,
                            position: position.map(|p| p as i32),
                        })
                    })
                    .collect::<Result<_, _>>()?;
                rest(
                    c,
                    c.ctx.rest.set_role_positions(guild_id, &positions, None),
                )
                .await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "setRoleHoistPositions",
        "roles",
        1,
        1,
        "[posições]",
        "Sets separators of 1 to 100 roles by id/position; null means zero. Requires ManageRoles and hierarchy.",
        |c| {
            Box::pin(async move {
                let guild_id = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_ROLES).await?;
                let batch = position_batch(&arg_value(c, 0).await?, "position")?;
                let guild = rest(c, c.ctx.rest.guild(guild_id)).await?;
                let author = member(c, guild_id, c.ctx.author_id()).await?;
                let positions: Vec<RoleHoistPosition> = batch
                    .iter()
                    .map(|(id, position)| {
                        let Some(role) = guild.roles.iter().find(|r| r.id == *id) else {
                            return Err(err(codes::INVALID_ARGUMENTS, "Role not in this guild."));
                        };
                        if top_role(&guild, &author) <= i64::from(role.position)
                            && c.ctx.author_id() != guild.owner_id
                        {
                            return Err(guard_reason(
                                "A role outranks the author.",
                                GuardReason::Hierarchy,
                            ));
                        }
                        Ok(RoleHoistPosition {
                            id: *id,
                            hoist_position: position.map(|p| p as i32),
                        })
                    })
                    .collect::<Result<_, _>>()?;
                let value = rest(
                    c,
                    c.ctx.rest.set_role_hoist_positions(guild_id, &positions),
                )
                .await?;
                Ok(serialize(&value))
            })
        },
    );

    d.add(
        "resetRoleHoistPositions",
        "roles",
        0,
        0,
        "",
        "Zeroes role separators; requires ManageRoles and hierarchy on every affected role.",
        |c| {
            Box::pin(async move {
                let guild_id = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_ROLES).await?;
                let guild = rest(c, c.ctx.rest.guild(guild_id)).await?;
                let author = member(c, guild_id, c.ctx.author_id()).await?;
                let affected: Vec<&flush_core::Role> = guild
                    .roles
                    .iter()
                    .filter(|r| r.hoist_position.unwrap_or(0) > 0)
                    .collect();
                for role in &affected {
                    if top_role(&guild, &author) <= i64::from(role.position)
                        && c.ctx.author_id() != guild.owner_id
                    {
                        return Err(guard_reason(
                            "A role outranks the author.",
                            GuardReason::Hierarchy,
                        ));
                    }
                }
                let _ = affected;
                let value = rest(c, c.ctx.rest.reset_role_hoist_positions(guild_id)).await?;
                Ok(serialize(&value))
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
