//! `messages.ts`: `$reply`, `$sendMessage`, `$editMessage`, `$deleteMessage`,
//! `$react`.

use serde_json::Value;

use flush_core::ReactionEmoji;
use flush_rest::EditMessageBody;

use crate::errors::ScriptFailure;
use crate::types::Definitions;

use super::payload::{default_mentions, payload};
use super::{arg_value, snowflake};

/// `<a?:name:id>` or `name:id` or a bare unicode emoji → `ReactionEmoji`.
fn parse_emoji(raw: &str) -> ReactionEmoji {
    let trimmed = raw.trim();
    let inner = trimmed
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .map(|s| s.trim_start_matches('a').trim_start_matches(':'))
        .unwrap_or(trimmed);
    if let Some((name, id)) = inner.rsplit_once(':') {
        if let Ok(id) = id.parse() {
            return ReactionEmoji {
                id: Some(id),
                name: name.to_string(),
            };
        }
    }
    ReactionEmoji {
        id: None,
        name: trimmed.to_string(),
    }
}

pub fn register(d: &mut Definitions) {
    d.add(
        "reply",
        "messages",
        1,
        1,
        "[text or embed]",
        "Replies without pings and without producing extra text.",
        |c| {
            Box::pin(async move {
                let mut send = payload(c, &arg_value(c, 0).await?).await?;
                default_mentions(&mut send);
                let rest = c.ctx.rest.clone();
                let channel = c.ctx.channel_id();
                let message = c.ctx.message.clone();
                c.wait(async move {
                    rest.reply(channel, &message, send, false)
                        .await
                        .map_err(ScriptFailure::from)
                })
                .await?;
                Ok(Value::String(String::new()))
            })
        },
    );

    d.add(
        "sendmessage",
        "messages",
        2,
        2,
        "[channelID;text or embed]",
        "Sends and returns the message ID.",
        |c| {
            Box::pin(async move {
                let channel = snowflake(&c.arg(0).await?)?;
                let mut send = payload(c, &arg_value(c, 1).await?).await?;
                default_mentions(&mut send);
                let rest = c.ctx.rest.clone();
                let sent = c
                    .wait(async move {
                        rest.send_message(channel, &send)
                            .await
                            .map_err(ScriptFailure::from)
                    })
                    .await?;
                Ok(Value::String(sent.id.to_string()))
            })
        },
    );

    d.add(
        "editmessage",
        "messages",
        3,
        3,
        "[channelID;messageID;text or embed]",
        "Edits a message.",
        |c| {
            Box::pin(async move {
                let channel = snowflake(&c.arg(0).await?)?;
                let message = snowflake(&c.arg(1).await?)?;
                let value = arg_value(c, 2).await?;
                let has_embeds = matches!(&value, Value::Object(o) if o.contains_key("embeds"))
                    || matches!(&value, Value::Array(_));
                let send = payload(c, &value).await?;
                let body = EditMessageBody {
                    content: send.content,
                    embeds: has_embeds.then_some(send.embeds),
                };
                let rest = c.ctx.rest.clone();
                c.wait(async move {
                    rest.edit_message(channel, message, &body)
                        .await
                        .map_err(ScriptFailure::from)
                })
                .await?;
                Ok(Value::String(String::new()))
            })
        },
    );

    d.add(
        "deletemessage",
        "messages",
        2,
        2,
        "[channelID;messageID]",
        "Deletes a message.",
        |c| {
            Box::pin(async move {
                let channel = snowflake(&c.arg(0).await?)?;
                let message = snowflake(&c.arg(1).await?)?;
                let rest = c.ctx.rest.clone();
                c.wait(async move {
                    rest.delete_message(channel, message)
                        .await
                        .map_err(ScriptFailure::from)
                })
                .await?;
                Ok(Value::String(String::new()))
            })
        },
    );

    d.add(
        "react",
        "messages",
        1,
        1,
        "[emoji]",
        "Reacts to the message that ran the command.",
        |c| {
            Box::pin(async move {
                let raw = c.arg(0).await?;
                let emoji = parse_emoji(&raw);
                let rest = c.ctx.rest.clone();
                let channel = c.ctx.channel_id();
                let message = c.ctx.message_id();
                c.wait(async move {
                    rest.add_reaction(channel, message, &emoji, None)
                        .await
                        .map_err(ScriptFailure::from)
                })
                .await?;
                Ok(Value::String(String::new()))
            })
        },
    );
}
