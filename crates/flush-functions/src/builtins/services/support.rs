//! Shared helpers for the `services` builtin group (mirrors
//! `src/commands/arguments.ts` and `src/domain/values.ts` of yuzuha).

use std::sync::Arc;

use flush_core::Permissions;
use flush_framework::{VariableScope, VariableService, VariableStore};
use serde_json::Value;

use crate::call::ScriptCall;
use crate::context::{ScriptContext, NOT_IN_GUILD};
use crate::errors::{codes, FnError, GuardError, GuardReason, ScriptFailure};
use crate::values::{js_number, text};

/// Mirrors `index(raw, min)`: a safe integer >= min.
pub fn index(value: &Value, min: usize) -> Result<usize, ScriptFailure> {
    let raw = text(value);
    let n = js_number(&raw).ok_or_else(|| {
        FnError::new(
            codes::INDEX,
            format!("Expected an index of at least {min}, got '{raw}'."),
        )
    })?;
    if n.fract() == 0.0 && n >= min as f64 && n <= 9_007_199_254_740_991.0 {
        Ok(n as usize)
    } else {
        Err(FnError::new(
            codes::INDEX,
            format!("Expected an index of at least {min}, got '{raw}'."),
        )
        .into())
    }
}

/// Mirrors `finite(raw)`: parses to a finite number.
pub fn finite(value: &Value) -> Result<f64, ScriptFailure> {
    let raw = text(value);
    js_number(&raw).filter(|n| n.is_finite()).ok_or_else(|| {
        FnError::new(codes::NUMBER, format!("Expected a number, got '{raw}'.")).into()
    })
}

/// `parseDuration`: `500ms`, `5s`, `10m`, `2h`, `1d` or a positive safe
/// integer (milliseconds).
pub fn parse_duration(value: &Value) -> Result<u64, ScriptFailure> {
    let raw = text(value);
    let trimmed = raw.trim();
    let parsed = if let Some(n) = trimmed.strip_suffix("ms") {
        n.parse::<u64>().ok()
    } else if let Some(n) = trimmed.strip_suffix('s') {
        n.parse::<u64>().ok().map(|v| v * 1_000)
    } else if let Some(n) = trimmed.strip_suffix('m') {
        n.parse::<u64>().ok().map(|v| v * 60_000)
    } else if let Some(n) = trimmed.strip_suffix('h') {
        n.parse::<u64>().ok().map(|v| v * 3_600_000)
    } else if let Some(n) = trimmed.strip_suffix('d') {
        n.parse::<u64>().ok().map(|v| v * 86_400_000)
    } else {
        trimmed.parse::<u64>().ok()
    };
    match parsed {
        Some(n) if n > 0 => Ok(n),
        _ => Err(FnError::new(
            codes::DURATION,
            format!("Expected a duration like '500ms', '5s', '10m', '2h' or '1d', got '{raw}'."),
        )
        .into()),
    }
}

/// `toJson(arg)` where the builtin receives the argument as text: parse
/// as JSON when possible, otherwise keep the raw string.
pub fn json_arg(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

/// The `VariableService` or `NATIVE_UNAVAILABLE` when no store is bound.
pub fn variables(
    ctx: &ScriptContext,
) -> Result<Arc<VariableService<Arc<dyn VariableStore + Send + Sync>>>, ScriptFailure> {
    ctx.variables.clone().ok_or_else(|| {
        FnError::new(
            codes::NATIVE_UNAVAILABLE,
            "Variable storage is not configured.",
        )
        .into()
    })
}

/// `variableScope(scope)`: `global` | `guild` | `user`.
pub fn parse_scope(value: &Value) -> Result<VariableScope, ScriptFailure> {
    match text(value).as_str() {
        "global" => Ok(VariableScope::Global),
        "guild" => Ok(VariableScope::Guild),
        "user" => Ok(VariableScope::User),
        s => Err(FnError::new(
            codes::VARIABLE_QUERY,
            format!("Unknown variable scope '{s}': expected 'global', 'guild' or 'user'."),
        )
        .into()),
    }
}

/// `userID`: `<@123>` / `<@!123>` / `123` or a safe integer.
pub fn user_id_arg(value: &Value) -> Result<String, ScriptFailure> {
    let raw = text(value);
    let trimmed = raw.trim();
    if let Some(cap) = regex::Regex::new(r"^<@!?(\d+)>$")
        .unwrap()
        .captures(trimmed)
    {
        return Ok(cap[1].to_string());
    }
    if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Ok(trimmed.to_string());
    }
    Err(FnError::new(
        codes::USER_ID,
        format!("Expected a user mention or id, got '{raw}'."),
    )
    .into())
}

/// `channelID`: `<#123>` / `123`.
pub fn channel_id_arg(value: &Value) -> Result<String, ScriptFailure> {
    let raw = text(value);
    let trimmed = raw.trim();
    if let Some(cap) = regex::Regex::new(r"^<#(\d+)>$").unwrap().captures(trimmed) {
        return Ok(cap[1].to_string());
    }
    if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
        return Ok(trimmed.to_string());
    }
    Err(FnError::new(
        codes::CHANNEL_ID,
        format!("Expected a channel mention or id, got '{raw}'."),
    )
    .into())
}

/// Empty string for a missing id (yuzuha returns `''` in a DM).
pub fn id_text(id: u64) -> String {
    if id == 0 {
        String::new()
    } else {
        id.to_string()
    }
}

/// `tokenize(input)`: whitespace split honoring `'`/`"` quotes and `\`
/// escapes (mirrors `arguments.ts`).
pub fn tokenize(input: &str) -> Result<Vec<String>, ScriptFailure> {
    if !input.contains('\\') && !input.contains('"') && !input.contains('\'') {
        return Ok(input.split_whitespace().map(str::to_string).collect());
    }
    let chars: Vec<char> = input.chars().collect();
    let mut result = Vec::new();
    let mut token = String::new();
    let mut quote = '\0';
    let mut last = '\0';
    let mut started = false;
    let mut start = 0usize;
    let mut index_ = 0usize;
    while index_ < chars.len() {
        let character = chars[index_];
        if character == '\\' && index_ + 1 < chars.len() {
            token.push_str(&chars[start..index_].iter().collect::<String>());
            index_ += 1;
            start = index_;
            last = chars[index_];
            started = true;
        } else if quote != '\0' && character == quote {
            token.push_str(&chars[start..index_].iter().collect::<String>());
            start = index_ + 1;
            quote = '\0';
        } else if quote == '\0'
            && (!started || last == '=')
            && (character == '"' || character == '\'')
        {
            token.push_str(&chars[start..index_].iter().collect::<String>());
            start = index_ + 1;
            quote = character;
            started = true;
        } else if quote == '\0' && character.is_whitespace() {
            if started {
                token.push_str(&chars[start..index_].iter().collect::<String>());
                result.push(std::mem::take(&mut token));
            }
            token.clear();
            last = '\0';
            started = false;
            start = index_ + 1;
        } else {
            last = character;
            started = true;
        }
        index_ += 1;
    }
    if quote != '\0' {
        return Err(GuardError::new("Unclosed quote.", GuardReason::InvalidArgument).into());
    }
    if started {
        token.push_str(&chars[start..].iter().collect::<String>());
        result.push(token);
    }
    Ok(result)
}

/// Reads a nested `a.b[0]` path (mirrors `readPath` of `objects.ts`).
pub fn read_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return Some(root);
    }
    let mut current = root;
    for segment in path.split('.') {
        let mut rest = segment;
        while let Some(open) = rest.find('[') {
            let name = &rest[..open];
            if !name.is_empty() {
                current = current.get(name)?;
            }
            let close = rest[open..].find(']')? + open;
            let idx: usize = rest[open + 1..close].parse().ok()?;
            current = current.get(idx)?;
            rest = &rest[close + 1..];
        }
        if !rest.is_empty() {
            current = current.get(rest)?;
        }
    }
    Some(current)
}

/// RFC 3339 timestamp from epoch milliseconds (civil-from-days, no deps).
pub fn iso_from_ms(ms: u64) -> String {
    let days = ms / 86_400_000;
    let rem = ms % 86_400_000;
    let (h, m, s) = (
        rem / 3_600_000,
        rem % 3_600_000 / 60_000,
        rem % 60_000 / 1_000,
    );
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

thread_local! {
    static PERMISSION_FLAGS: std::collections::HashMap<String, u128> = {
        crate::builtins::services::sdk_data::CONSTANTS
            .get("PermissionFlags")
            .and_then(Value::as_object)
            .map(|table| {
                table
                    .iter()
                    .filter_map(|(k, v)| v.as_u64().map(|n| (k.to_lowercase(), n as u128)))
                    .collect()
            })
            .unwrap_or_default()
    };
}

/// Permission flag name (e.g. `BanMembers`, `ManageGuild`) → bit.
pub fn permission_bit(name: &str) -> Option<u128> {
    PERMISSION_FLAGS.with(|table| table.get(&name.to_lowercase()).copied())
}

/// `hasPermission(bits, name)` with the `Administrator` bypass.
pub fn has_permission(bits: u128, name: &str) -> bool {
    permission_bit(name).is_some_and(|bit| Permissions::new(bits as u64).has(bit as u64))
}

/// `requirePermission(name, bot, message?)`: effective bits from the
/// context snapshot, else a REST member fetch.
pub async fn require_permission(
    c: &ScriptCall<'_>,
    name: &str,
    bot: bool,
    message: Option<String>,
) -> Result<(), ScriptFailure> {
    let ctx = c.ctx;
    let guild_id = ctx.require_guild()?;
    if permission_bit(name).is_none() {
        return Err(FnError::new(
            codes::INVALID_ARGUMENTS,
            format!("Unknown permission '{name}'."),
        )
        .into());
    }
    let reason = if bot {
        GuardReason::PermissionMissingBot
    } else {
        GuardReason::PermissionMissingAuthor
    };
    let fail = |m: Option<String>| {
        ScriptFailure::from(GuardError::new(
            m.unwrap_or_else(|| format!("Missing the {name} permission.")),
            reason,
        ))
    };
    let guild = ctx
        .guild
        .clone()
        .ok_or_else(|| GuardError::new(NOT_IN_GUILD, GuardReason::NotInGuild))?;
    let user_id = if bot {
        ctx.bot_id.unwrap_or_default()
    } else {
        ctx.author_id()
    };
    let member = if bot {
        ctx.bot_member.clone()
    } else {
        ctx.member.clone()
    };
    let member = match member {
        Some(member) => Some(member),
        None => wait_rest(c, ctx.rest.member(guild_id, user_id)).await.ok(),
    };
    let member = member.ok_or_else(|| fail(message.clone()))?;
    let is_owner = guild.owner_id == member.user.id;
    let bits = ScriptContext::member_permission_bits(
        &guild,
        ctx.channel.as_ref(),
        &member.roles,
        member.user.id,
    ) as u128;
    if !is_owner && !has_permission(bits, name) {
        return Err(fail(message));
    }
    Ok(())
}

/// `c.wait` over a REST future: maps `RestError` into `ScriptFailure`.
pub async fn wait_rest<F, T>(c: &ScriptCall<'_>, future: F) -> Result<T, ScriptFailure>
where
    F: std::future::Future<Output = Result<T, flush_rest::RestError>> + Send,
{
    c.wait(async move { future.await.map_err(ScriptFailure::from) })
        .await
}
