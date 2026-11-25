//! `$` functions that touch platform resources: instance info, gateway
//! diagnostics, collectors, webhooks, messages, embeds and payload
//! normalization. Ported from yuzuha `builtins/resource-*.ts`, `payload.ts`,
//! `embeds.ts` and `messages.ts`.

pub mod diagnostics;
pub mod embeds;
pub mod extras;
pub mod messages;
pub mod operations;
pub mod payload;

use serde_json::Value;

use flush_core::{Channel, Guild, MemberUser, Permissions, Snowflake};

use crate::call::ScriptCall;
use crate::errors::{codes, FnError, FnResult, GuardError, GuardReason, ScriptFailure};
use crate::types::Definitions;

pub fn register(d: &mut Definitions) {
    operations::register(d);
    extras::register(d);
    diagnostics::register(d);
    payload::register(d);
    embeds::register(d);
    messages::register(d);
}

/// `invalid(message)`: a guard failure for malformed arguments.
pub fn invalid<T>(message: impl Into<String>) -> FnResult<T> {
    Err(GuardError::new(message, GuardReason::InvalidArgument).into())
}

/// `operator(c)`: only the bot owner may run the function.
pub fn operator(c: &ScriptCall<'_>) -> FnResult<()> {
    if c.ctx.is_owner() {
        Ok(())
    } else {
        Err(GuardError::new(
            "Only the bot owner can use this function.",
            GuardReason::OwnerOnly,
        )
        .into())
    }
}

/// `snowflake(raw)`: a numeric 64-bit ID as `Snowflake`.
pub fn snowflake(raw: &str) -> FnResult<Snowflake> {
    let ok = raw.len() <= 20 && !raw.is_empty() && raw.chars().all(|c| c.is_ascii_digit());
    if !ok || raw.parse::<u64>().is_err() {
        return invalid("Enter a numeric 64-bit ID.");
    }
    Ok(raw.parse().unwrap())
}

/// `integer(raw, min, max)` over a JSON value (strings of digits allowed).
pub fn integer(raw: &Value, min: i64, max: i64) -> FnResult<i64> {
    let n = match raw {
        Value::String(s) if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) => {
            s.parse::<i64>().ok().map(|v| v as f64)
        }
        Value::Number(n) => n.as_f64(),
        _ => None,
    };
    match n {
        Some(v) if v.fract() == 0.0 && v >= min as f64 && v <= max as f64 => Ok(v as i64),
        _ => invalid(format!("Enter an integer between {min} and {max}.")),
    }
}

/// Bounded text argument (mirrors `boundedText`).
pub fn bounded_text(raw: &Value, max: usize) -> FnResult<String> {
    match raw {
        Value::String(s) if s.chars().count() <= max => Ok(s.clone()),
        _ => invalid(format!("Enter text of up to {max} characters.")),
    }
}

/// `bool(raw)`: `true`/`false` or their spellings.
pub fn bool_value(raw: &Value) -> FnResult<bool> {
    match raw {
        Value::Bool(b) => Ok(*b),
        Value::String(s) if s == "true" => Ok(true),
        Value::String(s) if s == "false" => Ok(false),
        _ => invalid("Use true or false."),
    }
}

/// `options(raw, keys)`: a plain object restricted to `keys`.
pub fn options<'a>(raw: &'a Value, keys: &[&str]) -> FnResult<&'a serde_json::Map<String, Value>> {
    let Value::Object(map) = raw else {
        return invalid("Invalid options.");
    };
    if map.keys().any(|k| !keys.contains(&k.as_str())) {
        return invalid("Invalid options.");
    }
    Ok(map)
}

/// `batch(raw, max, parse)`: a non-empty array of at most `max` items.
pub fn batch(raw: &Value, max: usize) -> FnResult<&[Value]> {
    let Value::Array(items) = raw else {
        return invalid(format!("Enter 1 to {max} items."));
    };
    if items.is_empty() || items.len() > max {
        return invalid(format!("Enter 1 to {max} items."));
    }
    Ok(items)
}

/// Parsed argument value: JSON text becomes a `Value`, everything else a
/// `String` (mirrors yuzuha's typed `arg(i)`).
pub async fn arg_value(c: &ScriptCall<'_>, index: usize) -> FnResult<Value> {
    let raw = c.arg(index).await?;
    Ok(match serde_json::from_str::<Value>(&raw) {
        Ok(value) => value,
        Err(_) => Value::String(raw),
    })
}

/// `required(c, index, label)` from `resource-extras.ts`: non-empty text.
pub async fn required(c: &ScriptCall<'_>, index: usize, label: &str) -> FnResult<String> {
    let value = c.arg(index).await?.trim().to_string();
    if value.is_empty() {
        return Err(FnError::new("ARGUMENT", format!("{label} is required.")).into());
    }
    Ok(value)
}

/// `number(c, index, fallback, min, max)` from `resource-extras.ts`.
pub async fn number_arg(
    c: &ScriptCall<'_>,
    index: usize,
    fallback: i64,
    min: i64,
    max: i64,
) -> FnResult<i64> {
    let raw = if c.arg_count() > index {
        c.arg(index).await?.trim().to_string()
    } else {
        String::new()
    };
    let result = if raw.is_empty() {
        fallback as f64
    } else {
        match raw.parse::<f64>() {
            Ok(v) => v,
            Err(_) => {
                return Err(FnError::new(
                    "ARGUMENT",
                    format!("Value must be an integer between {min} and {max}."),
                )
                .into());
            }
        }
    };
    if result.fract() != 0.0 || result < min as f64 || result > max as f64 {
        return Err(FnError::new(
            "ARGUMENT",
            format!("Value must be an integer between {min} and {max}."),
        )
        .into());
    }
    Ok(result as i64)
}

/// `userID(value)`: `<@id>`, `<@!id>` or a bare numeric ID.
pub fn user_id(raw: &str) -> FnResult<Snowflake> {
    let value = raw.trim();
    let inner = value
        .strip_prefix("<@")
        .and_then(|s| s.strip_suffix('>'))
        .map(|s| s.trim_start_matches('!'));
    let id = inner.unwrap_or(value);
    if id.is_empty() || id.len() > 20 || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(GuardError::new("Invalid user.", GuardReason::InvalidArgument).into());
    }
    id.parse()
        .map_err(|_| GuardError::new("Invalid user.", GuardReason::InvalidArgument).into())
}

/// Permission flag name → bit (mirrors `Permissions` of `@fluxerjs/types`).
pub fn flag_bit(name: &str) -> Option<u64> {
    Some(match name {
        "ViewChannel" => Permissions::VIEW_CHANNEL,
        "SendMessages" => Permissions::SEND_MESSAGES,
        "SendTtsMessages" => Permissions::SEND_TTS_MESSAGES,
        "ManageMessages" => Permissions::MANAGE_MESSAGES,
        "ReadMessageHistory" => Permissions::READ_MESSAGE_HISTORY,
        "MentionEveryone" => Permissions::MENTION_EVERYONE,
        "ManageGuild" => Permissions::MANAGE_GUILD,
        "ManageChannels" => Permissions::MANAGE_CHANNELS,
        "ManageNicknames" => Permissions::MANAGE_NICKNAMES,
        "ManageRoles" => Permissions::MANAGE_ROLES,
        "ManageWebhooks" => Permissions::MANAGE_WEBHOOKS,
        "ManageExpressions" => Permissions::MANAGE_EXPRESSIONS,
        "ManageThreads" => Permissions::MANAGE_THREADS,
        "ModerateMembers" => Permissions::MODERATE_MEMBERS,
        _ => return None,
    })
}

async fn guild_snapshot(c: &ScriptCall<'_>, guild: Snowflake) -> FnResult<Guild> {
    if let Some(g) = &c.ctx.guild {
        if g.id == guild {
            return Ok(g.clone());
        }
    }
    let rest = c.ctx.rest.clone();
    c.wait(async move { rest.guild(guild).await.map_err(ScriptFailure::from) })
        .await
}

async fn member_snapshot(
    c: &ScriptCall<'_>,
    guild: Snowflake,
    user: Snowflake,
) -> FnResult<MemberUser> {
    let is_author = c.ctx.author_id() == user;
    let is_bot = c.ctx.bot_id == Some(user);
    if is_author {
        if let Some(m) = &c.ctx.member {
            return Ok(m.clone());
        }
    }
    if is_bot {
        if let Some(m) = &c.ctx.bot_member {
            return Ok(m.clone());
        }
    }
    let rest = c.ctx.rest.clone();
    c.wait(async move {
        if is_bot {
            rest.me_member(guild).await.map_err(ScriptFailure::from)
        } else {
            rest.member(guild, user).await.map_err(ScriptFailure::from)
        }
    })
    .await
}

/// `requirePermission(flag, bot)`: checks one permission flag for the author
/// (`bot = false`) or the bot (`bot = true`), with optional channel overwrites.
pub async fn require_permission(
    c: &ScriptCall<'_>,
    flag: &str,
    bot: bool,
    channel: Option<&Channel>,
) -> FnResult<()> {
    let guild_id = c.ctx.require_guild()?;
    let user = if bot {
        match c.ctx.bot_id {
            Some(id) => id,
            None => return invalid("Could not resolve the bot user."),
        }
    } else {
        c.ctx.author_id()
    };
    if user.get() == 0 {
        return Err(GuardError::new(
            "Could not resolve the command author.",
            GuardReason::NoAuthor,
        )
        .into());
    }
    let guild = guild_snapshot(c, guild_id).await?;
    let member = member_snapshot(c, guild_id, user).await?;
    let is_owner = guild.owner_id == user;
    let bits =
        crate::context::ScriptContext::member_permission_bits(&guild, channel, &member.roles, user);
    let bit = flag_bit(flag).unwrap_or(0);
    let granted = is_owner || Permissions(bits).has(bit);
    if granted {
        return Ok(());
    }
    let reason = if bot {
        GuardReason::PermissionMissingBot
    } else {
        GuardReason::PermissionMissingAuthor
    };
    Err(GuardError::new(format!("Missing permission {flag}."), reason).into())
}

/// `both(c, flag, channelID)`: the flag must hold for author and bot.
pub async fn both(c: &ScriptCall<'_>, flag: &str, channel: Option<&Channel>) -> FnResult<()> {
    require_permission(c, flag, false, channel).await?;
    require_permission(c, flag, true, channel).await
}

/// `permissions(c, flag)` from `resource-extras.ts`: guild + both checks.
pub async fn permissions(c: &ScriptCall<'_>, flag: &str) -> FnResult<Snowflake> {
    let guild = c.ctx.require_guild()?;
    require_permission(c, flag, false, None).await?;
    require_permission(c, flag, true, None).await?;
    Ok(guild)
}

/// `guildChannel(c, id, flag)`: channel must belong to the current guild and
/// both author and bot need `flag` (default `ViewChannel`) inside it.
pub async fn guild_channel(c: &ScriptCall<'_>, id: Snowflake, flag: &str) -> FnResult<Channel> {
    let guild = c.ctx.require_guild()?;
    let rest = c.ctx.rest.clone();
    let channel: Channel = c
        .wait(async move { rest.channel(id).await.map_err(ScriptFailure::from) })
        .await?;
    if channel.guild_id != Some(guild) {
        return Err(GuardError::new(
            "The channel must belong to the current server.",
            GuardReason::Generic,
        )
        .into());
    }
    both(c, flag, Some(&channel)).await?;
    Ok(channel)
}

/// `collectorChannel(c)`: the command's channel, checked for collection.
pub async fn collector_channel(c: &ScriptCall<'_>) -> FnResult<Snowflake> {
    let id = snowflake(&c.ctx.channel_id().to_string())?;
    if c.ctx.guild_id().get() != 0 {
        guild_channel(c, id, "ViewChannel").await?;
        let rest = c.ctx.rest.clone();
        let channel: Channel = c
            .wait(async move { rest.channel(id).await.map_err(ScriptFailure::from) })
            .await?;
        both(c, "ReadMessageHistory", Some(&channel)).await?;
    } else if c.ctx.author_id().get() == 0 {
        return Err(GuardError::new(
            "Collection requires an author in context.",
            GuardReason::Generic,
        )
        .into());
    }
    Ok(id)
}

/// Native-only features that have no REST port yet.
pub fn native_unavailable<T>(what: &str) -> FnResult<T> {
    Err(FnError::new(
        codes::NATIVE_UNAVAILABLE,
        format!("{what} is not available in the native runtime."),
    )
    .into())
}
