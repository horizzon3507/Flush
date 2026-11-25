//! Embed normalization and `$` embed functions, ported from yuzuha
//! `domain/embeds.ts` and `builtins/embeds.ts`. Script-facing embeds keep the
//! yuzuha camelCase shape (`iconURL`, `description` for alt text).

use serde_json::{json, Map, Value};

use crate::call::ScriptCall;
use crate::errors::{FnError, FnResult};
use crate::types::Definitions;
use crate::values::truthy;

use super::arg_value;

pub const EMBED_TITLE: usize = 256;
pub const EMBED_DESCRIPTION: usize = 4096;
pub const EMBED_AUTHOR_NAME: usize = 256;
pub const EMBED_FOOTER_TEXT: usize = 2048;
pub const EMBED_ALT_TEXT: usize = 1024;
pub const EMBED_FIELDS: usize = 25;
pub const EMBED_FIELD_NAME: usize = 256;
pub const EMBED_FIELD_VALUE: usize = 1024;
pub const EMBED_TOTAL: usize = 6000;
pub const EMBEDS_PER_MESSAGE: usize = 10;

const COLOR_NAMES: &[(&str, u32)] = &[
    ("black", 0x000000),
    ("white", 0xFFFFFF),
    ("gray", 0x95A5A6),
    ("red", 0xE74C3C),
    ("orange", 0xE67E22),
    ("yellow", 0xF1C40F),
    ("green", 0x2ECC71),
    ("cyan", 0x1ABC9C),
    ("blue", 0x3498DB),
    ("purple", 0x9B59B6),
    ("pink", 0xEB459E),
];

const EMBED_KEYS: &[&str] = &[
    "url",
    "title",
    "description",
    "color",
    "timestamp",
    "author",
    "fields",
    "footer",
    "image",
    "thumbnail",
];

const READ_ONLY_KEYS: &[&str] = &[
    "type",
    "provider",
    "video",
    "audio",
    "nsfw",
    "children",
    "html",
    "htmlWidth",
    "htmlHeight",
    "proxyURL",
    "proxyIconURL",
    "contentType",
    "width",
    "height",
    "flags",
    "duration",
    "placeholder",
    "contentHash",
];

pub fn fail<T>(code: &'static str, message: impl Into<String>) -> FnResult<T> {
    Err(FnError::new(code, message).into())
}

/// `httpURL`: only http/https URLs pass.
pub fn http_url(value: &str, code: &'static str) -> FnResult<String> {
    match value.parse::<url::Url>() {
        Ok(u) if u.scheme() == "http" || u.scheme() == "https" => Ok(value.to_string()),
        _ => fail(code, "Use an HTTP/HTTPS URL."),
    }
}

fn iso_timestamp(value: &str) -> FnResult<String> {
    let re = regex::Regex::new(
        r"(?i)^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2}(?:\.\d{1,3})?)?(?:Z|[+-]\d{2}:\d{2})$",
    )
    .expect("regex");
    let ok = re.is_match(value) && valid_date(value);
    if ok {
        Ok(value.to_string())
    } else {
        fail(
            "EMBED_TIMESTAMP",
            "Use an ISO-8601 timestamp with a timezone, like 2026-01-20T21:30:00Z.",
        )
    }
}

/// `Date.parse` parity: the date part must be a real calendar date.
fn valid_date(value: &str) -> bool {
    let (Ok(y), Ok(m), Ok(d)) = (
        value[0..4].parse::<i32>(),
        value[5..7].parse::<u32>(),
        value[8..10].parse::<u32>(),
    ) else {
        return false;
    };
    if !(1..=12).contains(&m) {
        return false;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let dim = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    (1..=dim[(m - 1) as usize]).contains(&d)
}

/// Milliseconds since epoch → `YYYY-MM-DDTHH:MM:SS.MMMZ` (UTC).
pub fn iso_from_ms(ms: u64) -> String {
    let days = ms / 86_400_000;
    let rem = ms % 86_400_000;
    let (h, mi, s, ms3) = (
        rem / 3_600_000,
        (rem / 60_000) % 60,
        (rem / 1000) % 60,
        rem % 1000,
    );
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{ms3:03}Z")
}

fn rgb(parts: &[f64]) -> FnResult<u32> {
    if parts.len() != 3
        || parts
            .iter()
            .any(|p| p.fract() != 0.0 || !(0.0..=255.0).contains(p))
    {
        return fail("COLOR", "An RGB color needs three integers from 0 to 255.");
    }
    Ok(((parts[0] as u32) << 16) | ((parts[1] as u32) << 8) | parts[2] as u32)
}

/// `parseColor`: int 0..0xFFFFFF, `[r,g,b]`, `#RRGGBB`, `RRGGBB`, `r,g,b` or a
/// named color.
pub fn parse_color(value: &Value) -> FnResult<u32> {
    match value {
        Value::Number(n) => match n.as_f64() {
            Some(v) if v.fract() == 0.0 && (0.0..=0xFFFFFF as f64).contains(&v) => Ok(v as u32),
            _ => fail(
                "COLOR",
                "Use #RRGGBB, r,g,b (0 to 255) or a simple name: red, blue, green...",
            ),
        },
        Value::Array(items) => rgb(&items
            .iter()
            .map(|i| i.as_f64().unwrap_or(f64::NAN))
            .collect::<Vec<_>>()),
        Value::String(s) => {
            let raw = s.trim().to_ascii_lowercase();
            if let Some((_, c)) = COLOR_NAMES.iter().find(|(n, _)| *n == raw) {
                return Ok(*c);
            }
            let hex = raw.strip_prefix('#').unwrap_or(&raw);
            if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Ok(u32::from_str_radix(hex, 16).unwrap());
            }
            let parts: Vec<&str> = raw.split(',').map(str::trim).collect();
            if parts.len() == 3
                && parts
                    .iter()
                    .all(|p| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty())
            {
                let nums: Vec<f64> = parts.iter().filter_map(|p| p.parse().ok()).collect();
                return rgb(&nums);
            }
            fail(
                "COLOR",
                "Use #RRGGBB, r,g,b (0 to 255) or a simple name: red, blue, green...",
            )
        }
        _ => fail(
            "COLOR",
            "Use #RRGGBB, r,g,b (0 to 255) or a simple name: red, blue, green...",
        ),
    }
}

/// `mergeEmbedParts`: scalars are last-wins, nested objects merge, fields
/// accumulate in order.
pub fn merge_embed_parts(parts: &[Value]) -> Value {
    let mut result = Map::new();
    let mut fields: Vec<Value> = Vec::new();
    for part in parts {
        let Value::Object(map) = part else { continue };
        for (key, value) in map {
            match key.as_str() {
                "fields" => {
                    if let Value::Array(items) = value {
                        fields.extend(items.iter().cloned());
                    }
                }
                "author" | "footer" | "image" | "thumbnail" if value.is_object() => {
                    let entry = result
                        .entry(key.clone())
                        .or_insert_with(|| Value::Object(Map::new()));
                    if let (Value::Object(base), Value::Object(add)) = (&mut *entry, value) {
                        for (k, v) in add {
                            base.insert(k.clone(), v.clone());
                        }
                    }
                }
                "color" => {
                    if let Ok(c) = parse_color(value) {
                        result.insert("color".into(), json!(c));
                    }
                }
                _ => {
                    result.insert(key.clone(), value.clone());
                }
            }
        }
    }
    if !fields.is_empty() {
        result.insert("fields".into(), Value::Array(fields));
    }
    Value::Object(result)
}

/// `assertEmbedCount`.
pub fn assert_embed_count(count: usize) -> FnResult<()> {
    if count > EMBEDS_PER_MESSAGE {
        return fail(
            "EMBED_LIMIT",
            format!("A message accepts at most {EMBEDS_PER_MESSAGE} embeds."),
        );
    }
    Ok(())
}

fn measure(value: Option<&str>, max: usize, label: &str) -> FnResult<usize> {
    let len = value.map(|v| v.chars().count()).unwrap_or(0);
    if len > max {
        return fail(
            "EMBED_LIMIT",
            format!("{label} must have at most {max} characters."),
        );
    }
    Ok(len)
}

/// `validateEmbed` on a merged embed.
pub fn validate_embed(embed: &Value) -> FnResult<()> {
    let Value::Object(map) = embed else {
        return fail("EMBED", "Use the embed options.");
    };
    let str_of = |k: &str| map.get(k).and_then(|v| v.as_str());
    let mut total = measure(str_of("title"), EMBED_TITLE, "Title")?
        + measure(str_of("description"), EMBED_DESCRIPTION, "Description")?;
    if let Some(author) = map.get("author") {
        let name = author
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if name.is_empty() {
            return fail("EMBED_AUTHOR", "The author needs a name.");
        }
        total += measure(Some(name), EMBED_AUTHOR_NAME, "Author name")?;
        if let Some(Value::String(u)) = author.get("url") {
            http_url(u, "EMBED_URL")?;
        }
        if let Some(Value::String(u)) = author.get("iconURL") {
            http_url(u, "EMBED_URL")?;
        }
    }
    if let Some(footer) = map.get("footer") {
        let text = footer.get("text").and_then(|v| v.as_str());
        if text.is_none() {
            return fail(
                "EMBED_FOOTER",
                "The footer needs text; use $footer before $footerIcon.",
            );
        }
        total += measure(text, EMBED_FOOTER_TEXT, "Footer text")?;
        if let Some(Value::String(u)) = footer.get("iconURL") {
            http_url(u, "EMBED_URL")?;
        }
    }
    for (label, key) in [("image", "image"), ("thumbnail", "thumbnail")] {
        let Some(media) = map.get(key) else { continue };
        let url = media.get("url").and_then(|v| v.as_str());
        match url {
            Some(u) => {
                http_url(u, "EMBED_URL")?;
            }
            None => {
                return fail(
                    "EMBED_MEDIA",
                    format!("Set the {label} URL before the description."),
                );
            }
        }
        total += measure(
            media.get("description").and_then(|v| v.as_str()),
            EMBED_ALT_TEXT,
            "Alternative description",
        )?;
    }
    if let Some(Value::String(u)) = map.get("url") {
        http_url(u, "EMBED_URL")?;
    }
    if let Some(Value::String(t)) = map.get("timestamp") {
        iso_timestamp(t)?;
    }
    if let Some(c) = map.get("color") {
        parse_color(c)?;
    }
    if let Some(Value::Array(fields)) = map.get("fields") {
        if fields.len() > EMBED_FIELDS {
            return fail(
                "EMBED_LIMIT",
                format!("An embed accepts at most {EMBED_FIELDS} fields."),
            );
        }
        for f in fields {
            let name = f.get("name").and_then(|v| v.as_str()).unwrap_or_default();
            let value = f.get("value").and_then(|v| v.as_str()).unwrap_or_default();
            if name.is_empty() || name.chars().count() > EMBED_FIELD_NAME {
                return fail(
                    "EMBED_FIELD",
                    "A field needs a name of 1 to 256 characters.",
                );
            }
            if value.chars().count() > EMBED_FIELD_VALUE {
                return fail(
                    "EMBED_FIELD",
                    "A field value must have at most 1024 characters.",
                );
            }
            let inline = matches!(f.get("inline"), Some(Value::Bool(true)));
            if value.is_empty() && !inline {
                return fail(
                    "EMBED_FIELD",
                    "An empty value is only allowed on inline fields.",
                );
            }
            total += name.chars().count() + value.chars().count();
        }
    }
    if total > EMBED_TOTAL {
        return fail(
            "EMBED_LIMIT",
            format!("The embed exceeds {EMBED_TOTAL} characters."),
        );
    }
    let has_content = total > 0
        || map.get("url").is_some()
        || map.get("author").is_some()
        || map.get("image").is_some()
        || map.get("thumbnail").is_some();
    if !has_content {
        return fail(
            "EMBED_EMPTY",
            "The embed needs content: text, fields, image, thumbnail, author or URL.",
        );
    }
    Ok(())
}

fn normalize_record(value: &Value, label: &str, allowed: &[&str]) -> FnResult<Map<String, Value>> {
    let Value::Object(map) = value else {
        return fail("EMBED", format!("{label} must be an object."));
    };
    let mut result = Map::new();
    for (key, item) in map {
        if item.is_null() || READ_ONLY_KEYS.contains(&key.as_str()) {
            continue;
        }
        if !allowed.contains(&key.as_str()) {
            return fail("EMBED", format!("Unknown {label} field: {key}."));
        }
        result.insert(key.clone(), item.clone());
    }
    Ok(result)
}

fn expect_string(value: Option<&Value>, label: &str) -> FnResult<String> {
    match value {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(Value::Number(n)) => Ok(n.to_string()),
        Some(Value::Bool(b)) => Ok(b.to_string()),
        _ => fail("EMBED", format!("{label} must be text.")),
    }
}

fn boolean_of(value: &Value) -> FnResult<bool> {
    match value {
        Value::Bool(b) => Ok(*b),
        Value::String(s) if s == "true" => Ok(true),
        Value::String(s) if s == "false" => Ok(false),
        _ => fail("EMBED_FIELD", "inline must be true or false."),
    }
}

fn normalize_fields(value: &Value) -> FnResult<Vec<Value>> {
    let Value::Array(items) = value else {
        return fail("EMBED_FIELD", "fields must be an array of fields.");
    };
    items
        .iter()
        .map(|item| {
            let record = normalize_record(item, "field", &["name", "value", "inline"])?;
            let mut field = json!({
                "name": expect_string(record.get("name"), "field.name")?,
                "value": expect_string(record.get("value"), "field.value")?,
            });
            if let Some(v) = record.get("inline") {
                field["inline"] = Value::Bool(boolean_of(v)?);
            }
            Ok(field)
        })
        .collect()
}

/// `normalizeEmbed`: object or JSON text → validated embed (camelCase).
pub fn normalize_embed(input: &Value) -> FnResult<Value> {
    let result = normalize_record(input, "embed", EMBED_KEYS)?;
    let mut embed = Map::new();
    for (key, value) in result {
        match key.as_str() {
            "color" => {
                embed.insert("color".into(), json!(parse_color(&value)?));
            }
            "timestamp" => {
                embed.insert(
                    "timestamp".into(),
                    Value::String(iso_timestamp(&expect_string(Some(&value), "timestamp")?)?),
                );
            }
            "author" => {
                let rec = normalize_record(&value, "author", &["name", "url", "iconURL"])?;
                let mut author = Map::new();
                author.insert(
                    "name".into(),
                    Value::String(expect_string(rec.get("name"), "author.name")?),
                );
                if let Some(v) = rec.get("url") {
                    author.insert(
                        "url".into(),
                        Value::String(expect_string(Some(v), "author.url")?),
                    );
                }
                if let Some(v) = rec.get("iconURL") {
                    author.insert(
                        "iconURL".into(),
                        Value::String(expect_string(Some(v), "author.iconURL")?),
                    );
                }
                embed.insert("author".into(), Value::Object(author));
            }
            "footer" => {
                let rec = normalize_record(&value, "footer", &["text", "iconURL"])?;
                let mut footer = Map::new();
                footer.insert(
                    "text".into(),
                    Value::String(expect_string(rec.get("text"), "footer.text")?),
                );
                if let Some(v) = rec.get("iconURL") {
                    footer.insert(
                        "iconURL".into(),
                        Value::String(expect_string(Some(v), "footer.iconURL")?),
                    );
                }
                embed.insert("footer".into(), Value::Object(footer));
            }
            key @ ("image" | "thumbnail") => {
                let rec = normalize_record(&value, key, &["url", "description"])?;
                let mut media = Map::new();
                media.insert(
                    "url".into(),
                    Value::String(expect_string(rec.get("url"), "url")?),
                );
                if let Some(v) = rec.get("description") {
                    media.insert(
                        "description".into(),
                        Value::String(expect_string(Some(v), "description")?),
                    );
                }
                embed.insert(key.into(), Value::Object(media));
            }
            "fields" => {
                embed.insert("fields".into(), Value::Array(normalize_fields(&value)?));
            }
            key => {
                embed.insert(key.into(), Value::String(expect_string(Some(&value), key)?));
            }
        }
    }
    let embed = Value::Object(embed);
    validate_embed(&embed)?;
    Ok(embed)
}

/// Convert the script-facing embed (camelCase) into flush-core's wire `Embed`.
pub fn embed_to_wire(embed: &Value) -> Option<flush_core::Embed> {
    let map = embed.as_object()?;
    let str_of = |k: &str| map.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let media = |k: &str| {
        map.get(k).map(|m| flush_core::EmbedMedia {
            url: m
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            description: m
                .get("description")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        })
    };
    let fields = map
        .get("fields")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|f| {
                    Some(flush_core::EmbedField {
                        name: f.get("name")?.as_str()?.to_string(),
                        value: f.get("value")?.as_str()?.to_string(),
                        inline: f.get("inline").and_then(|v| v.as_bool()).unwrap_or(false),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(flush_core::Embed {
        title: str_of("title"),
        description: str_of("description"),
        url: str_of("url"),
        color: map.get("color").and_then(|v| v.as_u64()).map(|v| v as u32),
        timestamp: str_of("timestamp"),
        author: map.get("author").map(|a| flush_core::EmbedAuthor {
            name: a
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            url: a.get("url").and_then(|v| v.as_str()).map(str::to_string),
            icon_url: a
                .get("iconURL")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        }),
        footer: map.get("footer").map(|f| flush_core::EmbedFooter {
            text: f
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            icon_url: f
                .get("iconURL")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        }),
        image: media("image"),
        thumbnail: media("thumbnail"),
        fields,
    })
}

fn field_index(raw: &str, length: usize) -> FnResult<usize> {
    let index = raw.trim().parse::<f64>().ok().filter(|v| v.fract() == 0.0);
    match index {
        Some(v) if v >= 0.0 && (v as usize) < length => Ok(v as usize),
        _ => fail(
            "EMBED_FIELD",
            format!(
                "Invalid field index. Use an integer from 0 to {}.",
                length.saturating_sub(1)
            ),
        ),
    }
}

pub fn register(d: &mut Definitions) {
    d.add(
        "embed",
        "embeds",
        1,
        32,
        "[part;part;...]",
        "Combines typed embed parts. Separate them with semicolons.",
        |c| {
            Box::pin(async move {
                let mut parts = Vec::new();
                for i in 0..c.arg_count() {
                    let part = arg_value(c, i).await?;
                    if !part.is_object() || part.is_array() {
                        return fail("EMBED", "Use embed functions separated by ;.");
                    }
                    parts.push(part);
                }
                let result = merge_embed_parts(&parts);
                validate_embed(&result)?;
                Ok(result)
            })
        },
    );

    for name in ["title", "description", "footer"] {
        d.add(name, "embeds", 1, 1, "[text]", "Embed part.", move |c| {
            Box::pin(async move {
                let value = c.arg(0).await?;
                Ok(if name == "footer" {
                    json!({ "footer": { "text": value } })
                } else {
                    json!({ name: value })
                })
            })
        });
    }

    d.add(
        "url",
        "embeds",
        1,
        1,
        "[url]",
        "Clickable embed title URL (HTTP/HTTPS).",
        |c| {
            Box::pin(async move {
                let url = c.arg(0).await?;
                Ok(json!({ "url": http_url(&url, "EMBED_URL")? }))
            })
        },
    );

    d.add(
        "embedtimestamp",
        "embeds",
        1,
        1,
        "[iso|now]",
        "Embed date in ISO-8601 with timezone, or 'now' for the current instant.",
        |c| {
            Box::pin(async move {
                let value = c.arg(0).await?.trim().to_string();
                let ts = if value.eq_ignore_ascii_case("now") || value.eq_ignore_ascii_case("agora")
                {
                    iso_from_ms((c.ctx.now)())
                } else {
                    iso_timestamp(&value)?
                };
                Ok(json!({ "timestamp": ts }))
            })
        },
    );

    d.add(
        "footericon",
        "embeds",
        1,
        1,
        "[iconURL]",
        "Embed footer icon (HTTP/HTTPS). Use after $footer.",
        |c| {
            Box::pin(async move {
                let url = c.arg(0).await?;
                Ok(json!({ "footer": { "iconURL": http_url(&url, "EMBED_URL")? } }))
            })
        },
    );

    d.add(
        "author",
        "embeds",
        1,
        3,
        "[name;url?;iconURL?]",
        "Embed author, with optional link and icon (HTTP/HTTPS).",
        |c| {
            Box::pin(async move {
                let mut author = json!({ "name": c.arg(0).await? });
                if c.arg_count() > 1 {
                    let url = c.arg(1).await?;
                    if !url.trim().is_empty() {
                        author["url"] = json!(http_url(&url, "EMBED_URL")?);
                    }
                }
                if c.arg_count() > 2 {
                    let icon = c.arg(2).await?;
                    if !icon.trim().is_empty() {
                        author["iconURL"] = json!(http_url(&icon, "EMBED_URL")?);
                    }
                }
                Ok(json!({ "author": author }))
            })
        },
    );

    for name in ["image", "thumbnail"] {
        d.add(
            name,
            "embeds",
            1,
            1,
            "[url]",
            "Embed image or thumbnail (HTTP/HTTPS).",
            move |c| {
                Box::pin(async move {
                    let url = c.arg(0).await?;
                    Ok(json!({ name: { "url": http_url(&url, "EMBED_URL")? } }))
                })
            },
        );
        d.add(
            &format!("{name}alt"),
            "embeds",
            1,
            1,
            "[description]",
            "Alternative text for the embed image or thumbnail.",
            move |c| {
                Box::pin(async move {
                    let desc = c.arg(0).await?;
                    Ok(json!({ name: { "description": desc } }))
                })
            },
        );
    }

    d.add(
        "color",
        "embeds",
        1,
        1,
        "[#RRGGBB|r,g,b|name]",
        "Embed color: #RRGGBB, r,g,b tuple (0 to 255) or simple name (red, blue, green...).",
        |c| {
            Box::pin(async move {
                let color = parse_color(&arg_value(c, 0).await?)?;
                Ok(json!({ "color": color }))
            })
        },
    );

    d.add(
        "field",
        "embeds",
        2,
        3,
        "[name;value;inline?]",
        "Embed field. The value may be empty when the field is inline.",
        |c| {
            Box::pin(async move {
                let inline = if c.arg_count() > 2 {
                    truthy(&arg_value(c, 2).await?)
                } else {
                    false
                };
                Ok(json!({ "fields": [{
                    "name": c.arg(0).await?,
                    "value": c.arg(1).await?,
                    "inline": inline,
                }] }))
            })
        },
    );

    d.add(
        "fields",
        "embeds",
        1,
        1,
        "[array]",
        "Embed fields from an array of {name, value, inline} objects or [name, value, inline?] arrays.",
        |c| {
            Box::pin(async move {
                let value = arg_value(c, 0).await?;
                let Value::Array(items) = value else {
                    return fail("EMBED_FIELD", "fields expects an array of fields.");
                };
                let mut fields = Vec::new();
                for item in items {
                    let record = if let Value::Array(tuple) = &item {
                        let mut map = Map::new();
                        map.insert("name".into(), tuple.first().cloned().unwrap_or(Value::Null));
                        map.insert("value".into(), tuple.get(1).cloned().unwrap_or(Value::Null));
                        if let Some(v) = tuple.get(2) {
                            map.insert("inline".into(), v.clone());
                        }
                        Value::Object(map)
                    } else {
                        item
                    };
                    fields.extend(normalize_fields(&Value::Array(vec![record]))?);
                }
                Ok(json!({ "fields": fields }))
            })
        },
    );

    d.add(
        "addfield",
        "embeds",
        3,
        4,
        "[embed;name;value;inline?]",
        "Returns a copy of the embed with a field appended.",
        |c| {
            Box::pin(async move {
                let base = normalize_embed(&arg_value(c, 0).await?)?;
                let field = json!({
                    "name": c.arg(1).await?,
                    "value": c.arg(2).await?,
                    "inline": c.arg_count() > 3 && truthy(&arg_value(c, 3).await?),
                });
                let fields = normalize_fields(&json!([field]))?;
                let mut base = base.as_object().cloned().unwrap_or_default();
                let mut all = base
                    .remove("fields")
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                all.extend(fields);
                base.insert("fields".into(), Value::Array(all));
                normalize_embed(&Value::Object(base))
            })
        },
    );

    d.add(
        "removefield",
        "embeds",
        2,
        2,
        "[embed;index]",
        "Returns a copy of the embed without the field at the given index, starting at 0.",
        |c| {
            Box::pin(async move {
                let base = normalize_embed(&arg_value(c, 0).await?)?;
                let mut map = base.as_object().cloned().unwrap_or_default();
                let mut fields = map
                    .remove("fields")
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                let index = field_index(&c.arg(1).await?, fields.len())?;
                fields.remove(index);
                let mut embed = map;
                if !fields.is_empty() {
                    embed.insert("fields".into(), Value::Array(fields));
                }
                normalize_embed(&Value::Object(embed))
            })
        },
    );

    d.add(
        "editfield",
        "embeds",
        4,
        5,
        "[embed;index;name;value;inline?]",
        "Returns a copy of the embed with the field at the index replaced. Without inline, keeps the previous value.",
        |c| {
            Box::pin(async move {
                let base = normalize_embed(&arg_value(c, 0).await?)?;
                let mut map = base.as_object().cloned().unwrap_or_default();
                let mut fields = map
                    .remove("fields")
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                let index = field_index(&c.arg(1).await?, fields.len())?;
                let current = &fields[index];
                let inline = if c.arg_count() > 4 {
                    truthy(&arg_value(c, 4).await?)
                } else {
                    current.get("inline").and_then(|v| v.as_bool()).unwrap_or(false)
                };
                fields[index] = json!({
                    "name": c.arg(2).await?,
                    "value": c.arg(3).await?,
                    "inline": inline,
                });
                map.insert("fields".into(), Value::Array(fields));
                normalize_embed(&Value::Object(map))
            })
        },
    );

    d.add(
        "embedfromobject",
        "embeds",
        1,
        1,
        "[object|json]",
        "Builds an embed from an object or JSON text, with the same rules as $embed. Fluxer response fields are dropped.",
        |c| {
            Box::pin(async move {
                let value = arg_value(c, 0).await?;
                let parsed = match &value {
                    Value::String(s) => match serde_json::from_str::<Value>(s) {
                        Ok(v) => v,
                        Err(_) => return fail("EMBED", "Invalid embed JSON."),
                    },
                    other => other.clone(),
                };
                normalize_embed(&parsed)
            })
        },
    );
}

fn _unused(_c: &ScriptCall<'_>) {}
