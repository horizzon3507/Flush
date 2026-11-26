//! Port of `src/functions/builtins/moderation.ts`.

use serde_json::Value;

use flush_core::{MemberUser, Snowflake};
use flush_framework::CaseType;
use flush_rest::{BanOptions, MemberPatch};

use crate::context::ScriptContext;
use crate::errors::{codes, FnError, GuardError, GuardReason, ScriptFailure};
use crate::types::{BoxFnFuture, Definitions};
use crate::values::text;

use super::support::{self, wait_rest};

#[derive(Clone, Copy)]
enum Action {
    Ban,
    Unban,
    Kick,
    Timeout,
    Move,
    Mute,
    Deafen,
}

impl Action {
    /// The permission flag the author and the bot must hold.
    fn permission(self) -> &'static str {
        match self {
            Self::Ban | Self::Unban => "BanMembers",
            Self::Kick => "KickMembers",
            Self::Timeout => "ModerateMembers",
            Self::Move => "MoveMembers",
            Self::Mute => "MuteMembers",
            Self::Deafen => "DeafenMembers",
        }
    }

    /// `ctx.mod*` also records a case: the `CaseType` written.
    fn case_type(self) -> CaseType {
        match self {
            Self::Ban => CaseType::Ban,
            Self::Unban => CaseType::Unban,
            Self::Kick => CaseType::Kick,
            Self::Timeout => CaseType::Timeout,
            Self::Move => CaseType::Move,
            Self::Mute => CaseType::Mute,
            Self::Deafen => CaseType::Deafen,
        }
    }
}

fn guard(message: impl Into<String>, reason: GuardReason) -> ScriptFailure {
    GuardError::new(message, reason).into()
}

/// Resolves the target member; `Unban` never fetches it (yuzuha skips the
/// member fetch there).
async fn target_member(
    c: &crate::call::ScriptCall<'_>,
    guild_id: Snowflake,
    target_id: Snowflake,
    action: Action,
) -> Result<Option<MemberUser>, ScriptFailure> {
    if target_id == c.ctx.author_id() {
        return Err(guard(
            "You cannot target yourself.",
            GuardReason::SelfTarget,
        ));
    }
    if c.ctx.bot_id == Some(target_id) {
        return Err(guard("You cannot target the bot.", GuardReason::BotTarget));
    }
    if matches!(action, Action::Unban) {
        return Ok(None);
    }
    Ok(Some(
        wait_rest(c, c.ctx.rest.member(guild_id, target_id)).await?,
    ))
}

/// Shared `moderate()`: permission checks (author + bot), target
/// resolution and the built-in guard order.
async fn moderate(
    c: &crate::call::ScriptCall<'_>,
    action: Action,
    target: &Value,
) -> Result<(Snowflake, Option<MemberUser>), ScriptFailure> {
    let guild_id = c.ctx.require_guild()?;
    support::require_permission(c, action.permission(), false, None).await?;
    support::require_permission(c, action.permission(), true, None).await?;
    let raw = support::user_id_arg(target)?;
    let target_id = Snowflake::from(raw.parse::<u64>().map_err(|_| {
        FnError::new(
            codes::USER_ID,
            format!("Expected a user mention or id, got '{0}'.", text(target)),
        )
    })?);
    let guild = c
        .ctx
        .guild
        .clone()
        .ok_or_else(|| guard(crate::context::NOT_IN_GUILD, GuardReason::NotInGuild))?;
    let member = target_member(c, guild_id, target_id, action).await?;

    if let Some(target_member) = &member {
        if guild.owner_id == target_member.user.id {
            return Err(guard(
                "You cannot target the server owner.",
                GuardReason::OwnerTarget,
            ));
        }
        if matches!(action, Action::Timeout)
            && support::has_permission(
                ScriptContext::member_permission_bits(
                    &guild,
                    c.ctx.channel.as_ref(),
                    &target_member.roles,
                    target_member.user.id,
                ) as u128,
                "Administrator",
            )
        {
            return Err(guard(
                "Administrators cannot be timed out.",
                GuardReason::AdminImmune,
            ));
        }
        // Hierarchy: the executor and the bot must rank above the target.
        let check = |who: Option<&MemberUser>, is_owner: bool, msg: &str| match who {
            Some(w)
                if !is_owner
                    && guild.highest_role_position(w)
                        <= guild.highest_role_position(target_member) =>
            {
                Err(guard(msg, GuardReason::Hierarchy))
            }
            _ => Ok(()),
        };
        check(
            c.ctx.member.as_ref(),
            guild.owner_id == c.ctx.author_id(),
            "Your role must be above the target's.",
        )?;
        check(
            c.ctx.bot_member.as_ref(),
            c.ctx.bot_id == Some(guild.owner_id),
            "My role must be above the target's.",
        )?;
    }
    Ok((target_id, member))
}

/// Records a moderation case and marks `moderation_applied`.
fn record_case(
    c: &crate::call::ScriptCall<'_>,
    action: Action,
    target_id: Snowflake,
    reason: &str,
) -> Result<(), ScriptFailure> {
    let store = c.ctx.cases.clone().ok_or_else(|| {
        FnError::new(codes::NATIVE_UNAVAILABLE, "Case storage is not configured.")
    })?;
    let guild = c.ctx.guild_id().to_string();
    let case = store
        .lock()
        .unwrap()
        .add(
            &guild,
            action.case_type(),
            &target_id.to_string(),
            &c.ctx.author_id().to_string(),
            reason,
        )
        .map_err(|e| {
            FnError::new(
                codes::FUNCTION_ERROR,
                format!("Could not record the case: {e}"),
            )
        })?;
    *c.ctx.last_case.lock().unwrap() = Some(case);
    c.ctx
        .moderation_applied
        .store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// One `<name>`/`mod<Name>` pair: `execute` runs the REST call after
/// `moderate()` has cleared the guards.
struct ModFn {
    name: &'static str,
    mod_name: &'static str,
    action: Action,
    usage: &'static str,
    description: &'static str,
    reason_arg: usize,
    min_args: usize,
    max_args: usize,
    execute: for<'a> fn(&'a crate::call::ScriptCall<'a>, Snowflake, String) -> BoxFnFuture<'a>,
}

fn mod_fn(d: &mut Definitions, spec: ModFn) {
    for (i, function) in [spec.name, spec.mod_name].into_iter().enumerate() {
        let (action, reason_arg, execute) = (spec.action, spec.reason_arg, spec.execute);
        d.add(
            function,
            "moderation",
            spec.min_args,
            spec.max_args,
            spec.usage,
            spec.description,
            move |c| {
                Box::pin(async move {
                    let target = Value::String(c.arg(0).await?);
                    let reason = if c.arg_count() > reason_arg {
                        c.arg(reason_arg).await?
                    } else {
                        String::new()
                    };
                    let (target_id, _) = moderate(c, action, &target).await?;
                    let output = execute(c, target_id, reason.clone()).await?;
                    if i == 1 {
                        record_case(c, action, target_id, &reason)?;
                    }
                    Ok(output)
                })
            },
        );
    }
}

fn voice_flag(raw: &str) -> Result<bool, ScriptFailure> {
    match raw.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(FnError::new(
            codes::VOICE_STATE,
            format!("Expected 'true' or 'false', got '{raw}'."),
        )
        .into()),
    }
}

pub fn register(d: &mut Definitions) {
    mod_fn(
        d,
        ModFn {
            name: "kick",
            mod_name: "modKick",
            action: Action::Kick,
            usage: "[usuário;motivo]",
            description: "Kicks a member.",
            reason_arg: 1,
            min_args: 2,
            max_args: 2,
            execute: |c, id, _reason| {
                Box::pin(async move {
                    wait_rest(c, c.ctx.rest.kick(c.ctx.guild_id(), id, None)).await?;
                    Ok(Value::Null)
                })
            },
        },
    );

    mod_fn(
        d,
        ModFn {
            name: "ban",
            mod_name: "modBan",
            action: Action::Ban,
            usage: "[usuário;motivo;diasDeHistórico?]",
            description: "Bans a member.",
            reason_arg: 1,
            min_args: 2,
            max_args: 3,
            execute: |c, id, reason| {
                Box::pin(async move {
                    let days = if c.arg_count() > 2 {
                        let raw = c.arg(2).await?;
                        if raw.is_empty() {
                            None
                        } else {
                            Some(support::index(&Value::String(raw), 0)? as u32)
                        }
                    } else {
                        None
                    };
                    let opts = BanOptions {
                        reason: if reason.is_empty() {
                            None
                        } else {
                            Some(reason)
                        },
                        delete_message_days: days,
                        ..Default::default()
                    };
                    wait_rest(c, c.ctx.rest.ban(c.ctx.guild_id(), id, &opts, None)).await?;
                    Ok(Value::Null)
                })
            },
        },
    );

    mod_fn(
        d,
        ModFn {
            name: "unban",
            mod_name: "modUnban",
            action: Action::Unban,
            usage: "[usuário;motivo]",
            description: "Lifts a ban.",
            reason_arg: 1,
            min_args: 2,
            max_args: 2,
            execute: |c, id, _reason| {
                Box::pin(async move {
                    wait_rest(c, c.ctx.rest.unban(c.ctx.guild_id(), id, None)).await?;
                    Ok(Value::Null)
                })
            },
        },
    );

    mod_fn(
        d,
        ModFn {
            name: "timeout",
            mod_name: "modTimeout",
            action: Action::Timeout,
            usage: "[usuário;duração;motivo]",
            description: "Times a member out for a duration.",
            reason_arg: 2,
            min_args: 2,
            max_args: 3,
            execute: |c, id, reason| {
                Box::pin(async move {
                    let duration = support::parse_duration(&Value::String(c.arg(1).await?))?;
                    let until = support::iso_from_ms((c.ctx.now)() + duration);
                    wait_rest(
                        c,
                        c.ctx.rest.set_timeout(
                            c.ctx.guild_id(),
                            id,
                            Some(until),
                            if reason.is_empty() {
                                None
                            } else {
                                Some(reason.as_str())
                            },
                            None,
                        ),
                    )
                    .await?;
                    Ok(Value::Null)
                })
            },
        },
    );

    mod_fn(
        d,
        ModFn {
            name: "moveMember",
            mod_name: "modMoveMember",
            action: Action::Move,
            usage: "[usuário;canalOuVazio;motivo]",
            description: "Moves a member to a voice channel (or disconnects on empty).",
            reason_arg: 2,
            min_args: 2,
            max_args: 3,
            execute: |c, id, _reason| {
                Box::pin(async move {
                    let raw = c.arg(1).await?;
                    let channel = if raw.trim().is_empty() {
                        None
                    } else {
                        let id_ = support::channel_id_arg(&Value::String(raw))?;
                        Some(Snowflake::from(id_.parse::<u64>().map_err(|_| {
                            FnError::new(codes::CHANNEL_ID, "Expected a channel mention or id.")
                        })?))
                    };
                    let patch = MemberPatch {
                        channel_id: Some(channel),
                        ..Default::default()
                    };
                    wait_rest(
                        c,
                        c.ctx.rest.patch_member(c.ctx.guild_id(), id, &patch, None),
                    )
                    .await?;
                    Ok(Value::Null)
                })
            },
        },
    );

    mod_fn(
        d,
        ModFn {
            name: "muteMember",
            mod_name: "modMuteMember",
            action: Action::Mute,
            usage: "[usuário;habilitado;motivo]",
            description: "Server-mutes a member (`enabled` true|false).",
            reason_arg: 2,
            min_args: 3,
            max_args: 3,
            execute: |c, id, _reason| {
                Box::pin(async move {
                    let enabled = voice_flag(&c.arg(1).await?)?;
                    let patch = MemberPatch {
                        mute: Some(enabled),
                        ..Default::default()
                    };
                    wait_rest(
                        c,
                        c.ctx.rest.patch_member(c.ctx.guild_id(), id, &patch, None),
                    )
                    .await?;
                    Ok(Value::Null)
                })
            },
        },
    );

    mod_fn(
        d,
        ModFn {
            name: "deafenMember",
            mod_name: "modDeafenMember",
            action: Action::Deafen,
            usage: "[usuário;habilitado;motivo]",
            description: "Server-deafens a member (`enabled` true|false).",
            reason_arg: 2,
            min_args: 3,
            max_args: 3,
            execute: |c, id, _reason| {
                Box::pin(async move {
                    let enabled = voice_flag(&c.arg(1).await?)?;
                    let patch = MemberPatch {
                        deaf: Some(enabled),
                        ..Default::default()
                    };
                    wait_rest(
                        c,
                        c.ctx.rest.patch_member(c.ctx.guild_id(), id, &patch, None),
                    )
                    .await?;
                    Ok(Value::Null)
                })
            },
        },
    );
}
