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

/// `httpBase`: http(s) URL, no credentials, query, fragment or `\`, <= 2048
/// chars; returns the trimmed URL.
fn http_base_str(raw: &str) -> Result<String, ScriptFailure> {
    let raw = raw.trim();
    if raw.len() > 2048 || raw.contains('\\') {
        return Err(cdn_error("Invalid HTTP(S) base URL."));
    }
    let parsed = url::Url::parse(raw).map_err(|_| cdn_error("Invalid HTTP(S) base URL."))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(cdn_error(
            "HTTP(S) base URL must not include credentials, a query or a fragment.",
        ));
    }
    Ok(raw.to_string())
}

/// `normalizeApiOrigin`: trim, strip trailing `/`, strip a `/vN` suffix.
fn normalize_api_origin(api: &str) -> String {
    let trimmed = api.trim().trim_end_matches('/');
    let stripped = match trimmed.rfind('/') {
        Some(i)
            if trimmed[i + 1..].len() > 1
                && trimmed[i + 1..].starts_with(['v', 'V'])
                && trimmed[i + 2..].chars().all(|c| c.is_ascii_digit()) =>
        {
            &trimmed[..i]
        }
        _ => trimmed,
    };
    stripped.to_string()
}

/// `normalizeInviteBase`: trim, strip trailing `/`.
fn normalize_invite_base(invite: &str) -> String {
    invite.trim().trim_end_matches('/').to_string()
}

const ENDPOINT_KEYS: [&str; 11] = [
    "api",
    "api_client",
    "api_public",
    "gateway",
    "media",
    "static_cdn",
    "marketing",
    "admin",
    "invite",
    "gift",
    "webapp",
];

fn is_object(value: &Value) -> bool {
    value.as_object().is_some()
}

fn require_endpoint_str(
    map: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<String, ScriptFailure> {
    match map.get(key).and_then(Value::as_str) {
        Some(s) if !s.is_empty() => Ok(s.to_string()),
        _ => Err(cdn_error(format!(
            "Invalid instance discovery: endpoints.{key} must be a non-empty string"
        ))),
    }
}

/// `parseInstanceDiscovery`: validates the document and normalizes its
/// endpoint URLs, defaulting every optional section like yuzuha.
fn parse_instance_discovery(raw: &Value) -> Result<Value, ScriptFailure> {
    if !is_object(raw) {
        return Err(cdn_error("Invalid instance discovery: expected an object"));
    }
    match raw.get("api_code_version").and_then(Value::as_f64) {
        Some(v) if v.is_finite() => {}
        _ => {
            return Err(cdn_error(
                "Invalid instance discovery: api_code_version must be a number",
            ))
        }
    }
    let endpoints_raw = match raw.get("endpoints").and_then(Value::as_object) {
        Some(e) => e,
        None => {
            return Err(cdn_error(
                "Invalid instance discovery: endpoints must be an object",
            ))
        }
    };
    let mut endpoints = serde_json::Map::new();
    for key in ENDPOINT_KEYS {
        endpoints.insert(
            key.to_string(),
            Value::String(require_endpoint_str(endpoints_raw, key)?),
        );
    }
    for key in ["api", "api_client", "api_public"] {
        let v = normalize_api_origin(endpoints[key].as_str().unwrap_or_default());
        endpoints.insert(key.to_string(), Value::String(v));
    }
    let invite = normalize_invite_base(endpoints["invite"].as_str().unwrap_or_default());
    endpoints.insert("invite".into(), Value::String(invite));
    for key in ["media", "static_cdn"] {
        let v = endpoints[key]
            .as_str()
            .unwrap_or_default()
            .trim_end_matches('/')
            .to_string();
        endpoints.insert(key.to_string(), Value::String(v));
    }
    let mut out = serde_json::Map::new();
    out.insert("api_code_version".into(), raw["api_code_version"].clone());
    out.insert("endpoints".into(), Value::Object(endpoints));
    let defaults: [(&str, Value); 10] = [
        (
            "captcha",
            json!({"provider":"none","hcaptcha_site_key":null,"turnstile_site_key":null}),
        ),
        (
            "features",
            json!({"voice_enabled":true,"stripe_enabled":false,"self_hosted":false,"presigned_attachment_uploads":false,"emails_enabled":false}),
        ),
        (
            "gif",
            json!({"provider":"none","display_name":"","attribution_required":false}),
        ),
        (
            "sso",
            json!({"enabled":false,"enforced":false,"display_name":null,"redirect_uri":""}),
        ),
        (
            "registration",
            json!({"mode":"open","admin_registration_urls_enabled":false}),
        ),
        (
            "community",
            json!({"single_community":false,"single_community_guild_id":null,"direct_messages_disabled":false}),
        ),
        (
            "services",
            json!({"gif_enabled":false,"youtube_enabled":false,"bluesky_enabled":false}),
        ),
        (
            "limits",
            json!({"version":2,"traitDefinitions":[],"rules":[],"defaultsHash":""}),
        ),
        ("push", json!({"public_vapid_key":null})),
        ("app_public", json!({})),
    ];
    for (key, default) in defaults {
        out.insert(key.to_string(), raw.get(key).cloned().unwrap_or(default));
    }
    Ok(Value::Object(out))
}

/// `resolveInstanceEndpoints`: defaults, or a full discovery document, or a
/// partial `{key: url}` map merged over the defaults.
fn resolve_instance_endpoints(input: Option<&Value>) -> Result<Value, ScriptFailure> {
    let Some(input) = input else {
        return Ok(json!({
            "endpoints": sdk_data::instance_endpoints(),
            "discovery": Value::Null,
        }));
    };
    let is_full = input.get("endpoints").map(is_object).unwrap_or(false)
        && input
            .get("api_code_version")
            .and_then(Value::as_f64)
            .is_some();
    if is_full {
        let discovery = parse_instance_discovery(input)?;
        return Ok(json!({
            "endpoints": discovery["endpoints"].clone(),
            "discovery": discovery,
        }));
    }
    let partial = match input {
        Value::Object(map) => map,
        _ => return Err(cdn_error("Invalid instance endpoints.")),
    };
    let mut merged: serde_json::Map<String, Value> = sdk_data::instance_endpoints()
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    for key in ENDPOINT_KEYS {
        if let Some(v) = partial.get(key).and_then(Value::as_str) {
            if !v.is_empty() {
                merged.insert(key.to_string(), Value::String(v.to_string()));
            }
        }
    }
    for key in ["api", "api_client", "api_public"] {
        let v = normalize_api_origin(merged[key].as_str().unwrap_or_default());
        merged.insert(key.to_string(), Value::String(v));
    }
    if partial
        .get("api")
        .and_then(Value::as_str)
        .map(|s| !s.is_empty())
        .unwrap_or(false)
        && partial
            .get("api_public")
            .and_then(Value::as_str)
            .map(|s| s.is_empty())
            .unwrap_or(true)
    {
        merged.insert("api_public".into(), merged["api"].clone());
    }
    let invite = normalize_invite_base(merged["invite"].as_str().unwrap_or_default());
    merged.insert("invite".into(), Value::String(invite));
    for key in ["media", "static_cdn"] {
        let v = merged[key]
            .as_str()
            .unwrap_or_default()
            .trim_end_matches('/')
            .to_string();
        merged.insert(key.to_string(), Value::String(v));
    }
    Ok(json!({
        "endpoints": Value::Object(merged),
        "discovery": Value::Null,
    }))
}

fn instance_value(op: &str, value: Option<&Value>) -> Result<Value, ScriptFailure> {
    match op {
        "normalize" | "discoveryURL" => {
            let url = http_base_str(&text(value.unwrap_or(&Value::Null)))?;
            Ok(Value::String(if op == "normalize" {
                normalize_api_origin(&url)
            } else {
                format!("{}/.well-known/fluxer", normalize_api_origin(&url))
            }))
        }
        "parse" => parse_instance_discovery(value.unwrap_or(&Value::Null)),
        "resolve" => resolve_instance_endpoints(value),
        _ => Err(cdn_error(format!("Unknown instance operation '{op}'."))),
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
        "sdk",
        1,
        2,
        "[código;base?]",
        "Builds an invite URL without any network. Uses the instance base when omitted.",
        |c| {
            Box::pin(async move {
                let code = c.arg(0).await?;
                let base = if c.arg_count() > 1 {
                    normalize_invite_base(&http_base_str(&c.arg(1).await?)?)
                } else {
                    sdk_data::instance_endpoints()
                        .get("invite")
                        .cloned()
                        .unwrap_or_else(|| "https://fluxer.gg".into())
                };
                Ok(Value::String(format!("{base}/{code}")))
            })
        },
    );

    d.add(
        "instanceValue",
        "sdk",
        1,
        2,
        "[operação;valor?]",
        "Computes instance values without network discovery: normalize/discoveryURL (base URL), parse (document), resolve (document or partial endpoints; defaults when omitted).",
        |c| {
            Box::pin(async move {
                let op = c.arg(0).await?;
                let value = if c.arg_count() > 1 {
                    Some(support::json_arg(&c.arg(1).await?))
                } else {
                    None
                };
                instance_value(&op, value.as_ref())
            })
        },
    );
}
