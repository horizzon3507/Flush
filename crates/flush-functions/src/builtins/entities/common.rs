//! Shared helpers for the entity/native builtin group.

use std::future::Future;

use serde_json::{Map, Value};

use flush_core::{Channel, MemberUser, Snowflake};
use flush_rest::error::RestError;

use crate::call::ScriptCall;
use crate::context::ScriptContext;
use crate::errors::{codes, FnError, GuardError, GuardReason, ScriptFailure};

pub type Res<T> = Result<T, ScriptFailure>;

pub fn err(code: &'static str, message: impl Into<String>) -> ScriptFailure {
    ScriptFailure::Fn(FnError::new(code, message))
}

pub fn invalid(message: impl Into<String>) -> ScriptFailure {
    ScriptFailure::Guard(GuardError::new(message, GuardReason::InvalidArgument))
}

pub fn guard(message: impl Into<String>) -> ScriptFailure {
    ScriptFailure::Guard(GuardError::new(message, GuardReason::Generic))
}

pub fn guard_reason(message: impl Into<String>, reason: GuardReason) -> ScriptFailure {
    ScriptFailure::Guard(GuardError::new(message, reason))
}

pub fn guild_required() -> ScriptFailure {
    ScriptFailure::Guard(GuardError::new(
        "This operation requires a guild.",
        GuardReason::NotInGuild,
    ))
}

pub fn native_unavailable(what: &str) -> ScriptFailure {
    err(
        codes::NATIVE_UNAVAILABLE,
        format!("{what} is not available over REST."),
    )
}

/// `c.wait` around a REST call (mirrors yuzuha `transport` calls).
pub async fn rest<T>(
    c: &ScriptCall<'_>,
    fut: impl Future<Output = Result<T, RestError>> + Send,
) -> Res<T> {
    c.wait(async move { Ok(fut.await?) }).await
}

/// `found`: `''` on NOT_FOUND/UNKNOWN API errors, everything else rethrows.
pub async fn found<T>(
    c: &ScriptCall<'_>,
    fut: impl Future<Output = Result<T, RestError>> + Send,
) -> Res<Option<T>> {
    let result: Result<Result<T, RestError>, ScriptFailure> =
        c.wait(async move { Ok(fut.await) }).await;
    match result? {
        Ok(value) => Ok(Some(value)),
        Err(RestError::Api { status, code, .. })
            if status == 404 || code.contains("NOT_FOUND") || code.contains("UNKNOWN") =>
        {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

/// `<@!?id>` or a bare snowflake.
pub fn user_id(raw: &str) -> Res<Snowflake> {
    let s = raw.trim();
    let inner = s
        .strip_prefix("<@")
        .and_then(|s| s.strip_suffix('>'))
        .map(|s| s.trim_start_matches('!'))
        .unwrap_or(s);
    inner
        .parse::<Snowflake>()
        .map_err(|_| invalid(format!("Invalid user: {s}.")))
}

/// `<#id>` or a bare snowflake.
pub fn channel_id(raw: &str) -> Res<Snowflake> {
    let s = raw.trim();
    let inner = s
        .strip_prefix("<#")
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(s);
    inner
        .parse::<Snowflake>()
        .map_err(|_| invalid(format!("Invalid channel: {s}.")))
}

pub fn snowflake(raw: &str, what: &str) -> Res<Snowflake> {
    raw.trim()
        .parse::<Snowflake>()
        .map_err(|_| invalid(format!("Invalid {what}: {raw}.")))
}

/// `<@&id>` or a bare snowflake.
pub fn role_id(raw: &str) -> Res<Snowflake> {
    let s = raw.trim();
    let inner = s
        .strip_prefix("<@&")
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(s);
    inner
        .parse::<Snowflake>()
        .map_err(|_| invalid(format!("Invalid role: {s}.")))
}

/// `parseDuration`: `123ms|s|m|h|d` → milliseconds (FnError DURATION).
pub fn parse_duration(raw: &str) -> Res<u64> {
    let s = raw.trim();
    let split = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let value: f64 = num
        .parse()
        .map_err(|_| err(codes::FUNCTION_ERROR, format!("Invalid duration: {s}.")))?;
    let factor = match unit {
        "ms" => 1.0,
        "s" => 1_000.0,
        "m" => 60_000.0,
        "h" => 3_600_000.0,
        "d" => 86_400_000.0,
        _ => {
            return Err(ScriptFailure::Fn(FnError::new(
                "DURATION",
                format!("Invalid duration: {s}."),
            )))
        }
    };
    if !value.is_finite() || value < 0.0 {
        return Err(ScriptFailure::Fn(FnError::new(
            "DURATION",
            format!("Invalid duration: {s}."),
        )));
    }
    Ok((value * factor) as u64)
}

/// Integer argument mirroring yuzuha `integer` (FnError INDEX).
pub fn integer(raw: &str) -> Res<i64> {
    let s = raw.trim();
    let n: f64 = s
        .parse()
        .map_err(|_| err(codes::FUNCTION_ERROR, format!("Expected an integer: {s}.")))?;
    if n.fract() != 0.0 || !n.is_finite() {
        return Err(err(
            codes::FUNCTION_ERROR,
            format!("Expected an integer: {s}."),
        ));
    }
    Ok(n as i64)
}

/// `null`/`vazio` option argument: `Ok(None)` for empty or `null`.
pub fn opt(raw: String) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s == "null" {
        None
    } else {
        Some(s.to_string())
    }
}

/// Parse an argument into JSON (`arg` for array/object inputs).
pub async fn arg_value(c: &ScriptCall<'_>, index: usize) -> Res<Value> {
    let raw = c.arg(index).await?;
    Ok(serde_json::from_str(&raw).unwrap_or(Value::String(raw)))
}

/// yuzuha `boolean`: "true"/"false" → bool, else FnError.
pub fn boolean(raw: &str) -> Res<bool> {
    match raw.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        s => Err(err(
            codes::FUNCTION_ERROR,
            format!("Expected a boolean: {s}."),
        )),
    }
}

/// Fetch the context guild (or the given one).
pub async fn guild(c: &ScriptCall<'_>, id: Option<Snowflake>) -> Res<flush_core::Guild> {
    let id = match id {
        Some(id) => id,
        None => c.ctx.require_guild()?,
    };
    rest(c, c.ctx.rest.guild(id)).await
}

/// `contextChannel`: the message channel or the given one, same-guild guard.
pub async fn context_channel(c: &ScriptCall<'_>, id: Option<Snowflake>) -> Res<Channel> {
    let channel = match id {
        Some(id) => rest(c, c.ctx.rest.channel(id)).await?,
        None => rest(c, c.ctx.rest.channel(c.ctx.channel_id())).await?,
    };
    if channel.guild_id.is_some()
        && c.ctx.message.guild_id.is_some()
        && channel.guild_id != c.ctx.message.guild_id
    {
        return Err(guard("The channel must belong to the current context."));
    }
    Ok(channel)
}

/// Fetch a member, `None` on not-found.
pub async fn member(c: &ScriptCall<'_>, guild: Snowflake, user: Snowflake) -> Res<MemberUser> {
    rest(c, c.ctx.rest.member(guild, user)).await
}

/// Requires `bits` on both the author and the bot inside `channel`'s guild.
pub async fn require_perms(c: &ScriptCall<'_>, channel: Option<&Channel>, bits: u64) -> Res<()> {
    let guild = guild(c, None).await?;
    let author = member(c, guild.id, c.ctx.author_id()).await?;
    let author_bits =
        ScriptContext::member_permission_bits(&guild, channel, &author.roles, c.ctx.author_id());
    if author_bits & flush_core::Permissions::ADMINISTRATOR
        != flush_core::Permissions::ADMINISTRATOR
        && author_bits & bits != bits
    {
        return Err(guard_reason(
            "The author is missing a required permission.",
            GuardReason::PermissionMissingAuthor,
        ));
    }
    let bot = rest(c, c.ctx.rest.me_member(guild.id)).await?;
    let bot_bits = ScriptContext::member_permission_bits(&guild, channel, &bot.roles, bot.user.id);
    if bot_bits & flush_core::Permissions::ADMINISTRATOR != flush_core::Permissions::ADMINISTRATOR
        && bot_bits & bits != bits
    {
        return Err(guard_reason(
            "The bot is missing a required permission.",
            GuardReason::PermissionMissingBot,
        ));
    }
    Ok(())
}

/// Highest role position of a member inside `guild`.
pub fn top_role(guild: &flush_core::Guild, member: &MemberUser) -> i64 {
    if member.user.id == guild.owner_id {
        return i64::MAX;
    }
    member
        .roles
        .iter()
        .filter_map(|id| guild.roles.iter().find(|r| &r.id == id))
        .map(|r| r.position as i64)
        .max()
        .unwrap_or(0)
}

/// Camelize a serialized entity for `entityProperty` lookups:
/// `guild_id` → `guildID`, `avatar_url` → `avatarURL`, `*_at` stays `*At`.
pub fn camelize(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, value) in map {
                out.insert(camel_key(key), camelize(value));
            }
            Value::Object(out)
        }
        Value::Array(list) => Value::Array(list.iter().map(camelize).collect()),
        other => other.clone(),
    }
}

fn camel_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for (i, part) in key.split('_').enumerate() {
        if i == 0 {
            out.push_str(part);
        } else if part.eq_ignore_ascii_case("id") {
            out.push_str("ID");
        } else if part.eq_ignore_ascii_case("url") {
            out.push_str("URL");
        } else {
            let mut chars = part.chars();
            if let Some(first) = chars.next() {
                out.extend(first.to_uppercase());
                out.push_str(chars.as_str());
            }
        }
    }
    out
}

/// `entityProperty`: whitelist check then lookup (`''` on missing/null).
pub fn entity_property(info: &Value, raw: &str, allowed: &[&str]) -> Res<Value> {
    if !allowed.contains(&raw) {
        return Err(err(
            codes::FUNCTION_ERROR,
            format!("Unknown property: {raw}."),
        ));
    }
    let value = info.get(raw).cloned().unwrap_or(Value::Null);
    Ok(value)
}

/// Serialize + camelize an entity for property reads.
pub fn info_value<T: serde::Serialize>(entity: &T) -> Value {
    camelize(&serialize(entity))
}

/// yuzuha `jsNumber` option: empty → default.
pub fn limit(raw: Option<String>, default: u64, max: u64) -> Res<u64> {
    match raw.and_then(opt) {
        None => Ok(default),
        Some(s) => {
            let n = integer(&s)?;
            if n < 0 {
                return Err(err(
                    codes::FUNCTION_ERROR,
                    format!("Expected a non-negative integer: {s}."),
                ));
            }
            Ok((n as u64).min(max))
        }
    }
}

/// Emoji spec for reaction routes: `name` or `name:id` or `<a:name:id>`.
pub fn reaction_emoji(raw: &str) -> String {
    let s = raw.trim();
    if let Some(inner) = s.strip_prefix('<').and_then(|s| s.strip_suffix('>')) {
        let inner = inner.trim_start_matches('a').trim_start_matches(':');
        return inner.to_string();
    }
    s.to_string()
}

/// yuzuha `PermissionFlags` name → bit.
pub fn permission_bit(name: &str) -> Option<u64> {
    use flush_core::Permissions as P;
    Some(match name {
        n if n.eq_ignore_ascii_case("CreateInstantInvite") => P::CREATE_INSTANT_INVITE,
        n if n.eq_ignore_ascii_case("KickMembers") => P::KICK_MEMBERS,
        n if n.eq_ignore_ascii_case("BanMembers") => P::BAN_MEMBERS,
        n if n.eq_ignore_ascii_case("Administrator") => P::ADMINISTRATOR,
        n if n.eq_ignore_ascii_case("ManageChannels") => P::MANAGE_CHANNELS,
        n if n.eq_ignore_ascii_case("ManageGuild") => P::MANAGE_GUILD,
        n if n.eq_ignore_ascii_case("AddReactions") => P::ADD_REACTIONS,
        n if n.eq_ignore_ascii_case("ViewAuditLog") => P::VIEW_AUDIT_LOG,
        n if n.eq_ignore_ascii_case("PrioritySpeaker") => P::PRIORITY_SPEAKER,
        n if n.eq_ignore_ascii_case("Stream") => P::STREAM,
        n if n.eq_ignore_ascii_case("ViewChannel") => P::VIEW_CHANNEL,
        n if n.eq_ignore_ascii_case("SendMessages") => P::SEND_MESSAGES,
        n if n.eq_ignore_ascii_case("SendTtsMessages") => P::SEND_TTS_MESSAGES,
        n if n.eq_ignore_ascii_case("ManageMessages") => P::MANAGE_MESSAGES,
        n if n.eq_ignore_ascii_case("EmbedLinks") => P::EMBED_LINKS,
        n if n.eq_ignore_ascii_case("AttachFiles") => P::ATTACH_FILES,
        n if n.eq_ignore_ascii_case("ReadMessageHistory") => P::READ_MESSAGE_HISTORY,
        n if n.eq_ignore_ascii_case("MentionEveryone") => P::MENTION_EVERYONE,
        n if n.eq_ignore_ascii_case("UseExternalEmojis") => P::USE_EXTERNAL_EMOJIS,
        n if n.eq_ignore_ascii_case("Connect") => P::CONNECT,
        n if n.eq_ignore_ascii_case("Speak") => P::SPEAK,
        n if n.eq_ignore_ascii_case("MuteMembers") => P::MUTE_MEMBERS,
        n if n.eq_ignore_ascii_case("DeafenMembers") => P::DEAFEN_MEMBERS,
        n if n.eq_ignore_ascii_case("MoveMembers") => P::MOVE_MEMBERS,
        n if n.eq_ignore_ascii_case("UseVad") => P::USE_VAD,
        n if n.eq_ignore_ascii_case("ChangeNickname") => P::CHANGE_NICKNAME,
        n if n.eq_ignore_ascii_case("ManageNicknames") => P::MANAGE_NICKNAMES,
        n if n.eq_ignore_ascii_case("ManageRoles") => P::MANAGE_ROLES,
        n if n.eq_ignore_ascii_case("ManageWebhooks") => P::MANAGE_WEBHOOKS,
        n if n.eq_ignore_ascii_case("ManageExpressions") => P::MANAGE_EXPRESSIONS,
        n if n.eq_ignore_ascii_case("ManageThreads") => P::MANAGE_THREADS,
        n if n.eq_ignore_ascii_case("CreatePublicThreads") => P::CREATE_PUBLIC_THREADS,
        n if n.eq_ignore_ascii_case("CreatePrivateThreads") => P::CREATE_PRIVATE_THREADS,
        n if n.eq_ignore_ascii_case("UseExternalStickers") => P::USE_EXTERNAL_STICKERS,
        n if n.eq_ignore_ascii_case("SendMessagesInThreads") => P::SEND_MESSAGES_IN_THREADS,
        n if n.eq_ignore_ascii_case("ModerateMembers") => P::MODERATE_MEMBERS,
        n if n.eq_ignore_ascii_case("CreateExpressions") => P::CREATE_EXPRESSIONS,
        n if n.eq_ignore_ascii_case("PinMessages") => P::PIN_MESSAGES,
        n if n.eq_ignore_ascii_case("BypassSlowmode") => P::BYPASS_SLOWMODE,
        n if n.eq_ignore_ascii_case("UpdateRtcRegion") => P::UPDATE_RTC_REGION,
        n if n.eq_ignore_ascii_case("ViewChannelMembers") => P::VIEW_CHANNEL_MEMBERS,
        _ => return None,
    })
}

/// Comma-separated permission names → combined bits (guard on unknown).
pub fn permission_bits(raw: &str) -> Res<u64> {
    let mut bits = 0u64;
    for name in csv(raw) {
        bits |=
            permission_bit(&name).ok_or_else(|| guard(format!("Unknown permission: {name}.")))?;
    }
    Ok(bits)
}

/// Split a comma-separated id/name list.
pub fn csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// `<a:name:id>`/`name:id`/name → `ReactionEmoji` for the typed REST calls.
pub fn reaction_emoji_value(raw: &str) -> flush_core::ReactionEmoji {
    let route = reaction_emoji(raw);
    match route.rsplit_once(':') {
        Some((name, id)) if id.chars().all(|c| c.is_ascii_digit()) && !id.is_empty() => {
            flush_core::ReactionEmoji {
                id: id.parse().ok(),
                name: name.to_string(),
            }
        }
        _ => flush_core::ReactionEmoji {
            id: None,
            name: route,
        },
    }
}

/// `serde_json::to_value`, infallible.
pub fn serialize<T: serde::Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or_default()
}
