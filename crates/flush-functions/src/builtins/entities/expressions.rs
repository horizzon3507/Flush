//! yuzuha `native-expressions.ts` — emojis and stickers.

use serde_json::{json, Value};

use flush_core::Permissions;
use flush_rest::resources::StickerCreate;

use crate::errors::codes;
use crate::types::Definitions;

use super::common::{arg_value, entity_property, err, opt, require_perms, rest, serialize, Res};

const EMOJI: &[&str] = &[
    "id",
    "name",
    "animated",
    "url",
    "mention",
    "guildID",
    "identifier",
    "createdTimestamp",
    "createdAt",
];
const STICKER: &[&str] = &[
    "id",
    "name",
    "description",
    "tags",
    "guildID",
    "animated",
    "nsfw",
    "url",
    "createdTimestamp",
    "createdAt",
];

async fn opt_arg(c: &crate::call::ScriptCall<'_>, i: usize) -> Res<Option<String>> {
    if i >= c.arg_count() {
        return Ok(None);
    }
    Ok(opt(c.arg(i).await?))
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn valid_emoji_name(name: &str) -> Res<String> {
    let n = name.trim();
    if n.len() < 2 || n.len() > 32 || n.chars().any(|c| c.is_whitespace()) {
        return Err(err(
            codes::FUNCTION_ERROR,
            "Emoji name must be 2 to 32 characters without spaces.",
        ));
    }
    Ok(n.to_string())
}

/// Downloads an http(s) image and returns a `data:` URI.
async fn fetch_image(c: &crate::call::ScriptCall<'_>, url: &str) -> Res<String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(err(
            codes::FUNCTION_ERROR,
            "The image URL must be HTTP/HTTPS.",
        ));
    }
    let url = url.to_string();
    c.wait(async move {
        let mut res = surf::get(&url)
            .await
            .map_err(|e| err(codes::API_ERROR, e.to_string()))?;
        if !res.status().is_success() {
            return Err(err(
                codes::API_ERROR,
                format!("Could not download the image: {}.", res.status()),
            ));
        }
        let mime = res
            .header("content-type")
            .map(|v| {
                v.as_str()
                    .split(';')
                    .next()
                    .unwrap_or("image/png")
                    .to_string()
            })
            .unwrap_or_else(|| "image/png".into());
        let bytes = res
            .body_bytes()
            .await
            .map_err(|e| err(codes::API_ERROR, e.to_string()))?;
        Ok(format!("data:{mime};base64,{}", base64_encode(&bytes)))
    })
    .await
}

/// Raw base64 → `data:image/png;base64,...` (yuzuha requires no `data:` prefix).
fn sticker_image(raw: &str) -> Res<String> {
    if raw.starts_with("data:")
        || !raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
    {
        return Err(err(
            codes::FUNCTION_ERROR,
            "The sticker image must be valid base64, without a data: prefix.",
        ));
    }
    Ok(format!("data:image/png;base64,{raw}"))
}

/// Resolves an emoji by ID, name or `<a?:name:id>` mention in the guild.
async fn find_emoji(
    c: &crate::call::ScriptCall<'_>,
    guild: flush_core::Snowflake,
    query: &str,
) -> Res<flush_core::Emoji> {
    let q = query.trim();
    if q.is_empty() {
        return Err(err(
            codes::INVALID_ARGUMENTS,
            "Missing the emoji name, ID or mention.",
        ));
    }
    let emojis = rest(c, c.ctx.rest.guild_emojis(guild)).await?;
    let mention = q.strip_prefix("<").and_then(|s| s.strip_suffix('>'));
    let mention_id = mention.and_then(|m| m.rsplit(':').next());
    emojis
        .into_iter()
        .find(|e| {
            mention_id.map(|id| e.id.to_string() == id).unwrap_or(false)
                || e.id.to_string() == q
                || e.name == q
        })
        .ok_or_else(|| err(codes::FUNCTION_ERROR, format!("Emoji not found: {q}.")))
}

/// Resolves a sticker by ID or name in the guild.
async fn find_sticker(
    c: &crate::call::ScriptCall<'_>,
    guild: flush_core::Snowflake,
    query: &str,
) -> Res<flush_core::Sticker> {
    let q = query.trim();
    if q.is_empty() {
        return Err(err(
            codes::INVALID_ARGUMENTS,
            "Missing the sticker name or ID.",
        ));
    }
    let stickers = rest(c, c.ctx.rest.guild_stickers(guild)).await?;
    stickers
        .into_iter()
        .find(|s| s.id.to_string() == q || s.name == q)
        .ok_or_else(|| err(codes::FUNCTION_ERROR, format!("Sticker not found: {q}.")))
}

fn emoji_info(emoji: &flush_core::Emoji, guild: flush_core::Snowflake) -> Value {
    let mut info = super::common::camelize(&serialize(emoji));
    if let Some(map) = info.as_object_mut() {
        map.insert("guildID".into(), json!(guild.to_string()));
        map.insert(
            "url".into(),
            json!(flush_core::cdn::emoji_url(emoji.id, emoji.animated, None)),
        );
        map.insert(
            "mention".into(),
            json!(format!(
                "<{}:{}:{}>",
                if emoji.animated { "a" } else { "" },
                emoji.name,
                emoji.id
            )),
        );
        map.insert(
            "identifier".into(),
            json!(format!("{}:{}", emoji.name, emoji.id)),
        );
    }
    info
}

fn sticker_info(sticker: &flush_core::Sticker, guild: flush_core::Snowflake) -> Value {
    let mut info = super::common::camelize(&serialize(sticker));
    if let Some(map) = info.as_object_mut() {
        map.insert("guildID".into(), json!(guild.to_string()));
        map.insert(
            "url".into(),
            json!(flush_core::cdn::sticker_url(
                sticker.id,
                sticker.animated,
                None
            )),
        );
    }
    info
}

pub fn register(d: &mut Definitions) {
    d.add(
        "createEmojisBulk",
        "expressions",
        1,
        1,
        "[emojis]",
        "Creates 1 to 50 emojis with name and image in base64; returns success and failed without hiding partial failures.",
        |c| {
            Box::pin(async move {
                let value = arg_value(c, 0).await?;
                let Some(list) = value.as_array() else {
                    return Err(err(codes::INVALID_ARGUMENTS, "Expected an emoji array."));
                };
                if list.is_empty() || list.len() > 50 {
                    return Err(err(codes::INVALID_ARGUMENTS, "Expected 1 to 50 emojis."));
                }
                let guild = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_EXPRESSIONS).await?;
                let mut items = Vec::new();
                for item in list {
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| err(codes::INVALID_ARGUMENTS, "Missing name."))?;
                    let image = item
                        .get("image")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| err(codes::INVALID_ARGUMENTS, "Missing image."))?;
                    items.push((valid_emoji_name(name)?, sticker_image(image)?));
                }
                let result =
                    rest(c, c.ctx.rest.create_emojis_bulk(guild, &items, None)).await?;
                Ok(json!({
                    "success": result.success.iter().map(|e| e.id.to_string()).collect::<Vec<_>>(),
                    "failed": result.failed.iter().map(|f| json!({"name": f.name, "error": f.error})).collect::<Vec<_>>(),
                }))
            })
        },
    );

    d.add(
        "editEmoji",
        "expressions",
        2,
        3,
        "[nomeOuID;novoNome;motivo?]",
        "Renames an emoji of the guild and returns its information.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                let emoji = find_emoji(c, guild, &c.arg(0).await?).await?;
                let name = valid_emoji_name(&c.arg(1).await?)?;
                let reason = opt_arg(c, 2).await?;
                require_perms(c, None, Permissions::MANAGE_EXPRESSIONS).await?;
                let updated = rest(
                    c,
                    c.ctx
                        .rest
                        .rename_emoji(guild, emoji.id, &name, reason.as_deref()),
                )
                .await?;
                Ok(emoji_info(&updated, guild))
            })
        },
    );

    d.add(
        "createSticker",
        "expressions",
        2,
        5,
        "[nome;imagemBase64;descrição?;tags?;motivo?]",
        "Creates a sticker with a base64 image; comma-separated tags.",
        |c| {
            Box::pin(async move {
                let name = valid_emoji_name(&c.arg(0).await?)?;
                let image = sticker_image(&c.arg(1).await?)?;
                let description = opt_arg(c, 2).await?.unwrap_or_default();
                let tags = opt_arg(c, 3).await?.unwrap_or_default();
                let reason = opt_arg(c, 4).await?;
                let guild = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_EXPRESSIONS).await?;
                let create = StickerCreate {
                    name,
                    image,
                    description: Some(description).filter(|d| !d.is_empty()),
                    tags: tags
                        .split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect(),
                };
                let sticker = rest(
                    c,
                    c.ctx.rest.create_sticker(guild, &create, reason.as_deref()),
                )
                .await?;
                Ok(sticker_info(&sticker, guild))
            })
        },
    );

    d.add(
        "editSticker",
        "expressions",
        1,
        4,
        "[nomeOuID;novoNome?;descrição?;motivo?]",
        "Edits a sticker; an empty name keeps it and an empty description clears it.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                let sticker = find_sticker(c, guild, &c.arg(0).await?).await?;
                let name = opt_arg(c, 1)
                    .await?
                    .map(|s| valid_emoji_name(&s))
                    .transpose()?;
                let description = opt_arg(c, 2).await?;
                let reason = opt_arg(c, 3).await?;
                require_perms(c, None, Permissions::MANAGE_EXPRESSIONS).await?;
                let patch = flush_rest::resources::StickerPatch {
                    name,
                    description: description.map(Some),
                    tags: None,
                };
                let updated = rest(
                    c,
                    c.ctx
                        .rest
                        .edit_sticker(guild, sticker.id, &patch, reason.as_deref()),
                )
                .await?;
                Ok(sticker_info(&updated, guild))
            })
        },
    );

    d.add(
        "deleteSticker",
        "expressions",
        1,
        2,
        "[nomeOuID;motivo?]",
        "Deletes a sticker of the guild.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                let sticker = find_sticker(c, guild, &c.arg(0).await?).await?;
                let reason = opt_arg(c, 1).await?;
                require_perms(c, None, Permissions::MANAGE_EXPRESSIONS).await?;
                rest(
                    c,
                    c.ctx
                        .rest
                        .delete_sticker(guild, sticker.id, reason.as_deref()),
                )
                .await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "emoji",
        "expressions",
        1,
        2,
        "[nomeOuID;propriedade?]",
        "Fetches a guild emoji by ID, name or mention.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                let emoji = find_emoji(c, guild, &c.arg(0).await?).await?;
                let info = emoji_info(&emoji, guild);
                if c.arg_count() < 2 {
                    return Ok(info);
                }
                let raw = c.arg(1).await?;
                entity_property(&info, raw.trim(), EMOJI)
            })
        },
    );

    d.add(
        "emojis",
        "expressions",
        0,
        0,
        "",
        "Lists emojis of the current guild.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                let emojis = rest(c, c.ctx.rest.guild_emojis(guild)).await?;
                Ok(Value::Array(
                    emojis.iter().map(|e| emoji_info(e, guild)).collect(),
                ))
            })
        },
    );

    d.add(
        "createEmoji",
        "expressions",
        2,
        2,
        "[nome;url]",
        "Requires ManageExpressions of author and bot before downloading the image over HTTP, converting it to base64 and creating the emoji in the guild.",
        |c| {
            Box::pin(async move {
                let name = valid_emoji_name(&c.arg(0).await?)?;
                let url = c.arg(1).await?;
                let guild = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_EXPRESSIONS).await?;
                let image = fetch_image(c, &url).await?;
                let emoji =
                    rest(c, c.ctx.rest.create_emoji(guild, &name, &image, None)).await?;
                Ok(emoji_info(&emoji, guild))
            })
        },
    );

    d.add(
        "deleteEmoji",
        "expressions",
        1,
        1,
        "[nomeOuID]",
        "Deletes a guild emoji.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                let emoji = find_emoji(c, guild, &c.arg(0).await?).await?;
                require_perms(c, None, Permissions::MANAGE_EXPRESSIONS).await?;
                rest(c, c.ctx.rest.delete_emoji(guild, emoji.id, None)).await?;
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "sticker",
        "expressions",
        1,
        2,
        "[nomeOuID;propriedade?]",
        "Fetches a guild sticker by ID or name.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                let sticker = find_sticker(c, guild, &c.arg(0).await?).await?;
                let info = sticker_info(&sticker, guild);
                if c.arg_count() < 2 {
                    return Ok(info);
                }
                let raw = c.arg(1).await?;
                entity_property(&info, raw.trim(), STICKER)
            })
        },
    );

    d.add(
        "stickers",
        "expressions",
        0,
        0,
        "",
        "Lists stickers of the current guild.",
        |c| {
            Box::pin(async move {
                let guild = c.ctx.require_guild()?;
                let stickers = rest(c, c.ctx.rest.guild_stickers(guild)).await?;
                Ok(Value::Array(
                    stickers.iter().map(|s| sticker_info(s, guild)).collect(),
                ))
            })
        },
    );

    d.add(
        "createStickersBulk",
        "expressions",
        1,
        1,
        "[figurinhas]",
        "Creates 1 to 50 base64 stickers, up to 10 MB; validates the whole batch, requires ManageExpressions and preserves success/failed.",
        |c| {
            Box::pin(async move {
                let value = arg_value(c, 0).await?;
                let Some(list) = value.as_array() else {
                    return Err(err(codes::INVALID_ARGUMENTS, "Expected a sticker array."));
                };
                if list.is_empty() || list.len() > 50 {
                    return Err(err(codes::INVALID_ARGUMENTS, "Expected 1 to 50 stickers."));
                }
                let guild = c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_EXPRESSIONS).await?;
                let mut items = Vec::new();
                for item in list {
                    let get = |key: &str| -> Option<String> {
                        item.get(key).and_then(|v| v.as_str()).map(str::to_string)
                    };
                    let name = get("name")
                        .ok_or_else(|| err(codes::INVALID_ARGUMENTS, "Missing name."))?;
                    let image = sticker_image(&get("image").unwrap_or_default())?;
                    items.push(StickerCreate {
                        name: valid_emoji_name(&name)?,
                        image,
                        description: get("description"),
                        tags: get("tags")
                            .unwrap_or_default()
                            .split(',')
                            .map(|t| t.trim().to_string())
                            .filter(|t| !t.is_empty())
                            .collect(),
                    });
                }
                let result = rest(
                    c,
                    c.ctx.rest.create_stickers_bulk(guild, &items, None),
                )
                .await?;
                Ok(json!({
                    "success": result.success.iter().map(|s| s.id.to_string()).collect::<Vec<_>>(),
                    "failed": result.failed.iter().map(|f| json!({"name": f.name, "error": f.error})).collect::<Vec<_>>(),
                }))
            })
        },
    );
}
