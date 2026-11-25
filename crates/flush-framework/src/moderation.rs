//! Moderation checks (mirrors `Sour lib/modPermissions.ts` + `lib/modCore.ts`).
//!
//! - `check_member`: does the executor hold the action's permission? (owner/ADMIN pass)
//! - `check_hierarchy`: does the executor outrank the target? does the bot?

use std::future::Future;

use flush_core::{Channel, Guild, MemberUser, Permissions, Snowflake};

use crate::command::{mod_success, plain_error, CommandCtx, CommandError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionAction {
    Kick,
    Ban,
    Timeout,
    Moderate,
    Channel,
    Messages,
    Roles,
    Guild,
}

impl PermissionAction {
    pub fn flag(self) -> u64 {
        match self {
            Self::Kick => Permissions::KICK_MEMBERS,
            Self::Ban => Permissions::BAN_MEMBERS,
            Self::Timeout | Self::Moderate => Permissions::MODERATE_MEMBERS,
            Self::Channel => Permissions::MANAGE_CHANNELS,
            Self::Messages => Permissions::MANAGE_MESSAGES,
            Self::Roles => Permissions::MANAGE_ROLES,
            Self::Guild => Permissions::MANAGE_GUILD,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Kick => "Kick Members",
            Self::Ban => "Ban Members",
            Self::Timeout | Self::Moderate => "Moderate Members",
            Self::Channel => "Manage Channels",
            Self::Messages => "Manage Messages",
            Self::Roles => "Manage Roles",
            Self::Guild => "Manage Server",
        }
    }
}

pub struct Moderation;

impl Moderation {
    /// Effective permission = union of everyone + roles (+ channel overwrites).
    /// Official order: everyone overwrite → role union → member overwrite.
    /// ADMIN returns the full mask (bypasses overwrites).
    pub fn member_permissions(
        everyone_bits: u64,
        role_bits: &[u64],
        channel: Option<&Channel>,
        member_id: Snowflake,
        roles: &[Snowflake],
        everyone_id: Option<Snowflake>,
    ) -> Permissions {
        let mut iter = vec![everyone_bits];
        iter.extend_from_slice(role_bits);
        let base = Permissions::union(iter);
        if base.has(Permissions::ADMINISTRATOR) {
            return Permissions::new(u64::MAX);
        }
        let Some(ch) = channel else {
            return base;
        };
        let mut bits = base;
        if let Some(eid) = everyone_id {
            if let Some(ow) = ch.permission_overwrites.iter().find(|o| o.id == eid) {
                bits = bits.apply_overwrite(ow.allow, ow.deny);
            }
        }
        let mut allow = 0u64;
        let mut deny = 0u64;
        for ow in &ch.permission_overwrites {
            if roles.contains(&ow.id) {
                allow |= ow.allow;
                deny |= ow.deny;
            }
        }
        bits = bits.apply_overwrite(allow, deny);
        if let Some(ow) = ch.permission_overwrites.iter().find(|o| o.id == member_id) {
            bits = bits.apply_overwrite(ow.allow, ow.deny);
        }
        bits
    }

    pub fn check_member(
        guild: &Guild,
        executor: Option<&MemberUser>,
        action: PermissionAction,
        channel: Option<&Channel>,
        is_owner: bool,
    ) -> Option<String> {
        Self::check_member_with_owner(guild, executor, action, channel, is_owner, "You need")
    }

    /// Variant taking the message subject ("You need" / "I need").
    pub fn check_member_with_owner(
        guild: &Guild,
        executor: Option<&MemberUser>,
        action: PermissionAction,
        channel: Option<&Channel>,
        is_owner: bool,
        who: &'static str,
    ) -> Option<String> {
        let m = executor?;
        if is_owner {
            return None;
        }
        let role_bits: Vec<u64> = m
            .roles
            .iter()
            .filter_map(|id| guild.role(*id))
            .map(|r| r.permissions)
            .collect();
        let everyone_bits = guild.role(guild.id).map(|r| r.permissions).unwrap_or(0);
        let perms = Self::member_permissions(
            everyone_bits,
            &role_bits,
            channel,
            m.user.id,
            &m.roles,
            Some(guild.id),
        );
        if perms.has(action.flag()) {
            None
        } else {
            Some(plain_error(format!(
                "{who} the **{}** permission for that.",
                action.label()
            )))
        }
    }

    /// Hierarchy: executor above the target and bot above the target (owner passes).
    pub fn check_hierarchy(
        guild: &Guild,
        executor: Option<&MemberUser>,
        bot: Option<&MemberUser>,
        target: Option<&MemberUser>,
        executor_is_owner: bool,
        bot_is_owner: bool,
    ) -> Option<String> {
        let t = target?;
        if let Some(e) = executor {
            if !executor_is_owner
                && guild.highest_role_position(e) <= guild.highest_role_position(t)
            {
                return Some(plain_error("Your role must be above the target's."));
            }
        }
        if let Some(b) = bot {
            if !bot_is_owner && guild.highest_role_position(b) <= guild.highest_role_position(t) {
                return Some(plain_error("My role must be above the target's."));
            }
        }
        None
    }
}

/// Parameters for [`with_mod_action`].
pub struct ModAction<'a, F> {
    pub ctx: &'a CommandCtx<'a>,
    pub executor: Option<&'a MemberUser>,
    pub action: PermissionAction,
    pub target: Option<&'a MemberUser>,
    pub reason: &'a str,
    pub success_icon: &'a str,
    pub success_description: &'a str,
    pub case_id: u64,
    pub extra: Option<&'a str>,
    /// Moderation log channel (optional). Receives the same success
    /// message, without a ping. Send errors are ignored.
    pub log_channel: Option<Snowflake>,
    pub run: F,
}

/// Standard flow: checks → `run` → success reply.
/// Returns `Ok(true)` when `run` succeeded, `Ok(false)` when a check blocked it or `run` failed (after replying).
pub async fn with_mod_action<F, Fut>(opts: ModAction<'_, F>) -> Result<bool, CommandError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let ModAction {
        ctx,
        executor,
        action,
        target,
        reason,
        success_icon,
        success_description,
        case_id,
        extra,
        log_channel,
        run,
    } = opts;
    let Some(g) = ctx.guild.as_ref() else {
        ctx.reply_no_ping(plain_error("This command only works in servers."))
            .await?;
        return Ok(false);
    };
    let executor_is_owner = executor.map(|e| g.owner_id == e.user.id).unwrap_or(false);
    if let Some(err) =
        Moderation::check_member(g, executor, action, ctx.channel.as_ref(), executor_is_owner)
    {
        ctx.reply_no_ping(err).await?;
        return Ok(false);
    }
    let bot_is_owner = ctx
        .bot_member
        .as_ref()
        .map(|b| g.owner_id == b.user.id)
        .unwrap_or(false);
    if let Some(err) = Moderation::check_hierarchy(
        g,
        executor,
        ctx.bot_member.as_ref(),
        target,
        executor_is_owner,
        bot_is_owner,
    ) {
        ctx.reply_no_ping(err).await?;
        return Ok(false);
    }
    match run().await {
        Ok(()) => {
            let text = mod_success(success_icon, success_description, reason, case_id, extra);
            if let Some(log) = log_channel {
                let _ = ctx
                    .rest
                    .send_message(log, &flush_core::MessageSend::no_ping_text(text.clone()))
                    .await;
            }
            ctx.reply_no_ping(text).await?;
            Ok(true)
        }
        Err(detail) => {
            ctx.reply_no_ping(plain_error(format!(
                "Could not complete the action. {detail}"
            )))
            .await?;
            Ok(false)
        }
    }
}
