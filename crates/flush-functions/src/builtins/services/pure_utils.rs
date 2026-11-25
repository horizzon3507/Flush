//! Port of `src/functions/builtins/pure-utils.ts`.

use serde_json::{json, Value};

use flush_core::{Snowflake, FLUXER_EPOCH};

use crate::errors::{codes, FnError, ScriptFailure};
use crate::types::Definitions;
use crate::values::{js_number, text};

use super::{sdk_data, support};

fn sdk_error(message: impl Into<String>) -> ScriptFailure {
    FnError::new(codes::SDK, message.into()).into()
}

/// `SnowflakeUtil.isValid`: all digits, fits u64.
fn snowflake_valid(raw: &str) -> bool {
    let s = raw.trim();
    !s.is_empty()
        && s.len() <= 20
        && s.chars().all(|c| c.is_ascii_digit())
        && s.parse::<u64>().is_ok()
}

fn snowflake_op(op: &str, value: &Value, extra: Option<&Value>) -> Result<Value, ScriptFailure> {
    let raw = text(value);
    match op {
        "valid" => Ok(Value::Bool(snowflake_valid(&raw))),
        "parse" => {
            if !snowflake_valid(&raw) {
                return Err(sdk_error(format!("'{raw}' is not a valid snowflake.")));
            }
            Ok(json!(raw.trim().parse::<u64>().unwrap().to_string()))
        }
        "timestamp" => {
            let id = raw
                .trim()
                .parse::<u64>()
                .map_err(|_| sdk_error(format!("'{raw}' is not a valid snowflake.")))?;
            Ok(json!(((id >> 22) + FLUXER_EPOCH).to_string()))
        }
        "date" => {
            let id = raw
                .trim()
                .parse::<u64>()
                .map_err(|_| sdk_error(format!("'{raw}' is not a valid snowflake.")))?;
            Ok(Value::String(support::iso_from_ms(
                (id >> 22) + FLUXER_EPOCH,
            )))
        }
        "deconstruct" => {
            let id = raw
                .trim()
                .parse::<u64>()
                .map_err(|_| sdk_error(format!("'{raw}' is not a valid snowflake.")))?;
            let snowflake = Snowflake::from(id);
            Ok(json!({
                "timestamp": snowflake.timestamp_ms(),
                "workerID": snowflake.worker_id(),
                "sequence": snowflake.sequence(),
                "date": support::iso_from_ms(snowflake.timestamp_ms()),
            }))
        }
        "fromTimestamp" => {
            let ms = support::finite(value)? as i64;
            if ms < FLUXER_EPOCH as i64 {
                return Err(sdk_error("The timestamp predates the Fluxer epoch."));
            }
            let _ = extra;
            Ok(json!(((ms as u64 - FLUXER_EPOCH) << 22).to_string()))
        }
        _ => Err(sdk_error(format!("Unknown snowflake operation '{op}'."))),
    }
}

fn parse_op(op: &str, value: &Value, extra: Option<&Value>) -> Result<Value, ScriptFailure> {
    let raw = text(value);
    match op {
        "prefix" => {
            let content = raw.trim();
            let prefix = extra.map(text).unwrap_or_else(|| "!".into());
            if content.starts_with(&prefix) {
                let rest = content[prefix.len()..].trim();
                let mut parts = rest.split_whitespace();
                let command = parts.next().unwrap_or_default().to_lowercase();
                if command.is_empty() {
                    Ok(Value::Null)
                } else {
                    Ok(json!({
                        "command": command,
                        "args": parts.map(str::to_string).collect::<Vec<_>>(),
                    }))
                }
            } else {
                Ok(Value::Null)
            }
        }
        "userMention" => {
            let trimmed = raw.trim();
            if let Some(cap) = regex::Regex::new(r"^<@!?(\d{17,19})>$")
                .unwrap()
                .captures(trimmed)
            {
                return Ok(json!(cap[1]));
            }
            if snowflake_valid(trimmed) && trimmed.len() >= 17 && trimmed.len() <= 19 {
                return Ok(json!(trimmed));
            }
            Ok(Value::Null)
        }
        "roleMention" => {
            let re = regex::Regex::new(r"^<@&(\d{17,19})>$").unwrap();
            Ok(re
                .captures(raw.trim())
                .map(|c| json!(c[1].to_string()))
                .unwrap_or(Value::Null))
        }
        "emoji" => {
            let trimmed = raw.trim();
            if let Some(cap) = regex::Regex::new(r"^<(a?):(\w+):(\d+)>$")
                .unwrap()
                .captures(trimmed)
            {
                return Ok(json!({
                    "id": cap[3].to_string(),
                    "name": cap[2].to_string(),
                    "animated": &cap[1] == "a",
                }));
            }
            if let Some(cap) = regex::Regex::new(r"^(\w+):(\d{17,19})$")
                .unwrap()
                .captures(trimmed)
            {
                return Ok(json!({ "id": cap[2].to_string(), "name": cap[1].to_string() }));
            }
            if let Some(cap) = regex::Regex::new(r"^:(\w+):$").unwrap().captures(trimmed) {
                return Ok(json!({ "id": Value::Null, "name": cap[1].to_string() }));
            }
            if trimmed.is_empty() {
                Ok(Value::Null)
            } else {
                Ok(json!({ "id": Value::Null, "name": trimmed }))
            }
        }
        "shortcode" => {
            let name = raw.trim().trim_matches(':').to_lowercase();
            match sdk_data::shortcodes().get(&name) {
                Some(emoji) => Ok(Value::String(emoji.clone())),
                None => Ok(Value::Null),
            }
        }
        "guildInvite" | "groupDmInvite" => {
            let is_guild = op == "guildInvite";
            let type_ok =
                value.get("type").and_then(Value::as_i64) == Some(if is_guild { 0 } else { 1 });
            if type_ok {
                Ok(value.clone())
            } else {
                Ok(Value::Null)
            }
        }
        "uploadedAttachment" => {
            if value.get("upload_filename").is_some_and(|v| v.is_string()) {
                Ok(value.clone())
            } else {
                Ok(Value::Null)
            }
        }
        "isRecord" => Ok(Value::Bool(value.is_object())),
        "record" => Ok(if value.is_object() {
            value.clone()
        } else {
            Value::Null
        }),
        "number" => {
            let n = js_number(&raw).filter(|n| n.is_finite());
            Ok(n.map(|n| json!(n)).unwrap_or(Value::Null))
        }
        "string" => Ok(if value.is_string() {
            value.clone()
        } else {
            Value::Null
        }),
        _ => Err(sdk_error(format!("Unknown sdkParse operation '{op}'."))),
    }
}

fn format_op(op: &str, value: &Value, extra: &[String]) -> Result<Value, ScriptFailure> {
    match op {
        "color" => {
            let n = support::finite(value)? as u64;
            if n > 0xFF_FFFF {
                return Err(sdk_error(
                    "Color must be an integer between 0 and 16777215.",
                ));
            }
            Ok(Value::String(format!("#{n:06x}")))
        }
        "resolveColor" => {
            let n = resolve_color(value)?;
            Ok(json!(n))
        }
        "timestamp" => {
            let ms = support::finite(value)? as i64;
            let seconds = ms / 1_000;
            let style = extra.first().map(String::as_str).unwrap_or("");
            Ok(Value::String(if style.is_empty() {
                format!("<t:{seconds}>")
            } else {
                format!("<t:{seconds}:{style}>")
            }))
        }
        "emoji" => {
            let id = value.get("id").map(text);
            let name = text(value.get("name").unwrap_or(&Value::Null));
            let animated = value
                .get("animated")
                .map(|v| v.as_bool().unwrap_or(false))
                .unwrap_or(false);
            Ok(Value::String(match id {
                Some(id) if !id.is_empty() => {
                    if animated {
                        format!("a:{name}:{id}")
                    } else {
                        format!("{name}:{id}")
                    }
                }
                _ => name,
            }))
        }
        "truncate" => {
            let max = extra
                .first()
                .and_then(|e| js_number(e))
                .filter(|n| n.fract() == 0.0 && *n >= 0.0)
                .map(|n| n as usize)
                .unwrap_or(2000);
            let suffix = extra.get(1).cloned().unwrap_or_else(|| "...".into());
            let raw = text(value);
            let chars: Vec<char> = raw.chars().collect();
            if chars.len() <= max {
                return Ok(Value::String(raw));
            }
            if max <= suffix.chars().count() {
                return Ok(Value::String(suffix));
            }
            let keep = max - suffix.chars().count();
            let mut out: String = chars.into_iter().take(keep).collect();
            out.push_str(&suffix);
            Ok(Value::String(out))
        }
        _ => Err(sdk_error(format!("Unknown sdkFormat operation '{op}'."))),
    }
}

fn resolve_color(value: &Value) -> Result<u64, ScriptFailure> {
    match value {
        Value::Number(n) => {
            let v = n.as_u64().unwrap_or(0);
            if v > 0xFF_FFFF {
                return Err(sdk_error("Color must be between 0 and 16777215."));
            }
            Ok(v)
        }
        Value::Array(rgb) if rgb.len() == 3 => {
            let mut out = 0u64;
            for (i, component) in rgb.iter().enumerate() {
                let n = component
                    .as_u64()
                    .ok_or_else(|| sdk_error("RGB components must be integers 0–255."))?;
                if n > 255 {
                    return Err(sdk_error("RGB components must be integers 0–255."));
                }
                out |= n << (16 - i * 8);
            }
            Ok(out)
        }
        _ => {
            let raw = text(value);
            let hex = raw.strip_prefix('#').unwrap_or(&raw);
            if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                u64::from_str_radix(hex, 16).map_err(|_| sdk_error("Invalid hex color."))
            } else {
                Err(sdk_error(
                    "Color must be a number, hex string, or RGB array.",
                ))
            }
        }
    }
}

/// `normalizeCacheLimit(value, fallback)`.
fn normalize_cache_limit(value: Option<f64>, fallback: f64) -> f64 {
    let raw = value.unwrap_or(fallback);
    if !raw.is_finite() || raw <= 0.0 {
        f64::INFINITY
    } else {
        raw.floor()
    }
}

fn cache_limit_op(op: &str, value: &Value, extra: &[String]) -> Result<Value, ScriptFailure> {
    match op {
        "normalize" => {
            let raw = if value.is_null() {
                None
            } else {
                Some(support::finite(value)?)
            };
            let fallback = extra
                .first()
                .and_then(|e| js_number(e))
                .unwrap_or(f64::INFINITY);
            let result = normalize_cache_limit(raw, fallback);
            Ok(if result.is_infinite() {
                Value::Null
            } else {
                json!(result)
            })
        }
        "resolve" => {
            let input = value;
            let defaults = sdk_data::cache_limits();
            let merge = |key: &str| -> Value {
                input
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| defaults.get(key).cloned().unwrap_or(Value::Null))
            };
            let mut limits = serde_json::Map::new();
            for key in [
                "guilds", "channels", "users", "members", "roles", "emojis", "stickers", "messages",
            ] {
                let raw = merge(key);
                let raw_num = match &raw {
                    Value::Bool(false) => None,
                    v => v.as_f64(),
                };
                let normalized = normalize_cache_limit(raw_num, f64::INFINITY);
                limits.insert(
                    key.to_string(),
                    if normalized.is_infinite() {
                        Value::Null
                    } else {
                        json!(normalized)
                    },
                );
            }
            Ok(json!({
                "cache": {
                    "guilds": merge("guilds"),
                    "channels": merge("channels"),
                    "users": merge("users"),
                    "members": merge("members"),
                    "roles": merge("roles"),
                    "emojis": merge("emojis"),
                    "stickers": merge("stickers"),
                    "messages": merge("messages"),
                },
                "limits": Value::Object(limits),
            }))
        }
        _ => Err(sdk_error(format!(
            "Unknown sdkCacheLimit operation '{op}'."
        ))),
    }
}

pub fn register(d: &mut Definitions) {
    d.add(
        "sdkParse",
        "util",
        2,
        3,
        "[operação;valor;extra?]",
        "Parses values with @fluxerjs helpers.",
        |c| {
            Box::pin(async move {
                let op = c.arg(0).await?;
                let value = support::json_arg(&c.arg(1).await?);
                let extra = if c.arg_count() > 2 {
                    Some(support::json_arg(&c.arg(2).await?))
                } else {
                    None
                };
                parse_op(&op, &value, extra.as_ref())
            })
        },
    );

    d.add(
        "sdkFormat",
        "util",
        2,
        4,
        "[operação;valor;extra?;extra2?]",
        "Formats values with @fluxerjs helpers.",
        |c| {
            Box::pin(async move {
                let op = c.arg(0).await?;
                let value = support::json_arg(&c.arg(1).await?);
                let mut extra = Vec::new();
                for i in 2..c.arg_count() {
                    extra.push(c.arg(i).await?);
                }
                format_op(&op, &value, &extra)
            })
        },
    );

    d.add(
        "snowflake",
        "util",
        2,
        2,
        "[operação;valor]",
        "Snowflake helpers (valid, parse, timestamp, date, deconstruct, fromTimestamp).",
        |c| {
            Box::pin(async move {
                let op = c.arg(0).await?;
                let value = support::json_arg(&c.arg(1).await?);
                snowflake_op(&op, &value, None)
            })
        },
    );

    d.add(
        "sdkCacheLimit",
        "util",
        1,
        3,
        "[operação;valor?;extra?]",
        "Cache limit helpers (normalize, resolve).",
        |c| {
            Box::pin(async move {
                let op = c.arg(0).await?;
                let value = if c.arg_count() > 1 {
                    support::json_arg(&c.arg(1).await?)
                } else {
                    Value::Null
                };
                let mut extra = Vec::new();
                for i in 2..c.arg_count() {
                    extra.push(c.arg(i).await?);
                }
                cache_limit_op(&op, &value, &extra)
            })
        },
    );
}
