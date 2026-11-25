//! Port of `src/functions/builtins/pure-cdn.ts`.

use serde_json::{json, Value};

use flush_core::Snowflake;

use crate::errors::{codes, FnError, ScriptFailure};
use crate::types::Definitions;
use crate::values::text;

use super::{sdk_data, support};

fn cdn_error(message: impl Into<String>) -> ScriptFailure {
    FnError::new(codes::CDN, message.into()).into()
}

/// `/^\d{1,20}$/` — a valid snowflake id string.
fn id_ok(value: &Value) -> bool {
    let raw = text(value);
    !raw.is_empty() && raw.len() <= 20 && raw.chars().all(|c| c.is_ascii_digit())
}

/// `/^[\w-]{1,128}$/` — a valid asset hash.
fn hash_ok(value: &Value) -> bool {
    let raw = text(value);
    !raw.is_empty()
        && raw.len() <= 128
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn require_id(value: &Value, name: &str) -> Result<u64, ScriptFailure> {
    if id_ok(value) {
        text(value)
            .parse::<u64>()
            .map_err(|_| cdn_error(format!("Invalid {name}.")))
    } else {
        Err(cdn_error(format!(
            "Expected a {name} id, got '{0}'.",
            text(value)
        )))
    }
}

fn require_hash(value: &Value) -> Result<String, ScriptFailure> {
    if hash_ok(value) {
        Ok(text(value))
    } else {
        Err(cdn_error(format!("Invalid asset hash '{0}'.", text(value))))
    }
}

/// `parseAssetOptions`: `{size}` object or raw size, 1–16384.
fn asset_size(value: Option<&Value>) -> Result<u32, ScriptFailure> {
    let Some(value) = value else { return Ok(128) };
    let size = match value {
        Value::Object(map) => map.get("size").cloned().unwrap_or(json!(128)),
        other => other.clone(),
    };
    let n = size
        .as_u64()
        .or_else(|| text(&size).parse::<u64>().ok())
        .ok_or_else(|| cdn_error("Asset size must be an integer."))? as u32;
    if !(1..=16384).contains(&n) {
        return Err(cdn_error("Asset size must be between 1 and 16384."));
    }
    Ok(n)
}

/// `httpBase`: http(s), no credentials, query or fragment, <= 2048 chars.
/// Returns (scheme, host[:port]).
fn http_base(raw: &str) -> Result<(String, String), ScriptFailure> {
    if raw.len() > 2048 {
        return Err(cdn_error("The URL exceeds 2048 characters."));
    }
    let (scheme, rest) = raw
        .split_once("://")
        .ok_or_else(|| cdn_error(format!("Invalid URL '{raw}'.")))?;
    if scheme != "http" && scheme != "https" {
        return Err(cdn_error("The URL must use http or https."));
    }
    if rest.contains('?') || rest.contains('#') {
        return Err(cdn_error("Query strings and fragments are not allowed."));
    }
    let authority = rest
        .trim_end_matches('/')
        .split('/')
        .next()
        .unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return Err(cdn_error("Credentials are not allowed in the URL."));
    }
    if !authority.split(':').all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    }) {
        return Err(cdn_error(format!("Invalid host in '{raw}'.")));
    }
    Ok((scheme.to_string(), authority.to_string()))
}

fn normalize_instance(value: &Value) -> Result<String, ScriptFailure> {
    let raw = text(value);
    let candidate = if raw.contains("://") {
        raw
    } else {
        format!("https://{raw}/")
    };
    let (scheme, authority) = http_base(&candidate)?;
    Ok(format!("{scheme}://{authority}"))
}

fn instance_op(op: &str, value: &Value, extra: Option<&Value>) -> Result<Value, ScriptFailure> {
    match op {
        "normalize" => Ok(Value::String(normalize_instance(value)?)),
        "discoveryURL" => {
            let normalized = normalize_instance(value)?;
            Ok(Value::String(format!("{normalized}/.well-known/fluxer")))
        }
        "parse" => {
            let normalized = normalize_instance(value)?;
            let (_, host) = http_base(&normalized)?;
            let host = host.split(':').next().unwrap_or_default().to_string();
            Ok(json!({ "endpoint": normalized, "instanceDomain": host }))
        }
        "resolve" => {
            let endpoint = normalize_instance(value)?;
            let mut out = serde_json::Map::new();
            let keys = [
                "api",
                "gateway",
                "cdn",
                "staticCdn",
                "invite",
                "gift",
                "development",
                "apiDocs",
                "status",
                "support",
            ];
            let raw = extra.cloned().unwrap_or(Value::Null);
            for key in keys {
                let path = raw
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| sdk_data::instance_endpoints().get(key).cloned())
                    .unwrap_or_default();
                out.insert(key.to_string(), json!(format!("{endpoint}{path}")));
            }
            Ok(Value::Object(out))
        }
        _ => Err(cdn_error(format!("Unknown sdkInstance operation '{op}'."))),
    }
}

fn cdn_url(
    c: &crate::call::ScriptCall<'_>,
    kind: &str,
    args: &[Value],
) -> Result<Value, ScriptFailure> {
    let guild_id = c.ctx.guild_id();
    match kind {
        "guildIcon" | "guildBanner" | "guildSplash" => {
            let id = Snowflake::from(require_id(
                args.first().ok_or_else(|| cdn_error("Missing id."))?,
                "guild",
            )?);
            let hash = require_hash(args.get(1).ok_or_else(|| cdn_error("Missing hash."))?)?;
            let size = asset_size(args.get(2))?;
            let asset = match kind {
                "guildIcon" => flush_core::cdn::GuildAsset::Icon,
                "guildBanner" => flush_core::cdn::GuildAsset::Banner,
                _ => flush_core::cdn::GuildAsset::Splash,
            };
            Ok(
                flush_core::cdn::guild_asset_url(asset, id, &hash, Some(size))
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            )
        }
        "avatar" => {
            let id = Snowflake::from(require_id(
                args.first().ok_or_else(|| cdn_error("Missing id."))?,
                "user",
            )?);
            let hash = require_hash(args.get(1).ok_or_else(|| cdn_error("Missing hash."))?)?;
            let size = asset_size(args.get(2))?;
            Ok(flush_core::cdn::avatar_url(id, &hash, Some(size))
                .map(Value::String)
                .unwrap_or(Value::Null))
        }
        "defaultAvatar" => {
            let id = Snowflake::from(require_id(
                args.first().ok_or_else(|| cdn_error("Missing id."))?,
                "user",
            )?);
            Ok(Value::String(flush_core::cdn::default_avatar_url(id)))
        }
        "banner" => {
            let id = Snowflake::from(require_id(
                args.first().ok_or_else(|| cdn_error("Missing id."))?,
                "user",
            )?);
            let hash = require_hash(args.get(1).ok_or_else(|| cdn_error("Missing hash."))?)?;
            let size = asset_size(args.get(2))?;
            Ok(flush_core::cdn::banner_url(id, &hash, Some(size))
                .map(Value::String)
                .unwrap_or(Value::Null))
        }
        "memberAvatar" => {
            let member = Snowflake::from(require_id(
                args.first().ok_or_else(|| cdn_error("Missing id."))?,
                "member",
            )?);
            let hash = require_hash(args.get(1).ok_or_else(|| cdn_error("Missing hash."))?)?;
            let size = asset_size(args.get(2))?;
            Ok(
                flush_core::cdn::member_avatar_url(guild_id, member, &hash, Some(size))
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            )
        }
        "memberBanner" => {
            let member = Snowflake::from(require_id(
                args.first().ok_or_else(|| cdn_error("Missing id."))?,
                "member",
            )?);
            let hash = require_hash(args.get(1).ok_or_else(|| cdn_error("Missing hash."))?)?;
            let size = asset_size(args.get(2))?;
            Ok(
                flush_core::cdn::member_banner_url(guild_id, member, &hash, Some(size))
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            )
        }
        "emoji" | "sticker" => {
            let id = Snowflake::from(require_id(
                args.first().ok_or_else(|| cdn_error("Missing id."))?,
                kind,
            )?);
            let animated = args.get(1).map(|v| text(v) == "true").unwrap_or(false);
            let size = asset_size(args.get(2))?;
            Ok(Value::String(if kind == "emoji" {
                flush_core::cdn::emoji_url(id, animated, Some(size))
            } else {
                flush_core::cdn::sticker_url(id, animated, Some(size))
            }))
        }
        "displayAvatar" => {
            let id = Snowflake::from(require_id(
                args.first().ok_or_else(|| cdn_error("Missing id."))?,
                "user",
            )?);
            let hash = match args.get(1) {
                Some(v) if !v.is_null() && !text(v).is_empty() => Some(require_hash(v)?),
                _ => None,
            };
            let size = asset_size(args.get(2))?;
            Ok(Value::String(flush_core::cdn::display_avatar_url(
                id,
                hash.as_deref(),
                Some(size),
            )))
        }
        _ => Err(cdn_error(format!("Unknown cdnURL kind '{kind}'."))),
    }
}

pub fn register(d: &mut Definitions) {
    d.add(
        "cdnURL",
        "util",
        1,
        6,
        "[tipo;id?;hash?;opções?]",
        "Builds CDN URLs (guildIcon, avatar, emoji, ...).",
        |c| {
            Box::pin(async move {
                let kind = c.arg(0).await?;
                let mut args = Vec::new();
                for i in 1..c.arg_count() {
                    args.push(support::json_arg(&c.arg(i).await?));
                }
                cdn_url(c, &kind, &args)
            })
        },
    );

    d.add(
        "inviteURL",
        "util",
        1,
        2,
        "[operação;código?]",
        "Builds invite URLs (url).",
        |c| {
            Box::pin(async move {
                let op = c.arg(0).await?;
                let code = if c.arg_count() > 1 {
                    c.arg(1).await?
                } else {
                    String::new()
                };
                match op.as_str() {
                    "url" | "create" => {
                        let base = sdk_data::instance_endpoints()
                            .get("invite")
                            .cloned()
                            .unwrap_or_else(|| "https://fluxer.gg/".into());
                        Ok(Value::String(if code.is_empty() {
                            base
                        } else {
                            format!("{base}{code}")
                        }))
                    }
                    _ => Err(cdn_error(format!("Unknown inviteURL operation '{op}'."))),
                }
            })
        },
    );

    d.add(
        "sdkInstance",
        "util",
        1,
        3,
        "[operação;valor?;endpoints?]",
        "Instance helpers (normalize, discoveryURL, parse, resolve).",
        |c| {
            Box::pin(async move {
                let op = c.arg(0).await?;
                let value = if c.arg_count() > 1 {
                    support::json_arg(&c.arg(1).await?)
                } else {
                    Value::Null
                };
                let extra = if c.arg_count() > 2 {
                    Some(support::json_arg(&c.arg(2).await?))
                } else {
                    None
                };
                instance_op(&op, &value, extra.as_ref())
            })
        },
    );
}
