//! yuzuha `native-collectors.ts`.

use std::time::Duration;

use serde_json::Value;

use flush_core::message::MessageSend;
use flush_framework::collector::Collectors;

use crate::errors::codes;
use crate::types::Definitions;

use super::common::{
    channel_id, err, opt, parse_duration, reaction_emoji, rest, snowflake, user_id, Res,
};

/// Duration in ms: plain digits mean seconds; anything else goes through
/// `parseDuration` (`500ms`, `5s`, ...).
fn duration_ms(raw: &str) -> Res<u64> {
    let value = raw.trim();
    if value.chars().all(|c| c.is_ascii_digit() || c == '.') && !value.is_empty() {
        let secs: f64 = value
            .parse()
            .map_err(|_| err(codes::INVALID_ARGUMENTS, "Invalid duration."))?;
        return Ok((secs * 1000.0) as u64);
    }
    parse_duration(value)
}

async fn opt_arg(c: &crate::call::ScriptCall<'_>, i: usize) -> Res<Option<String>> {
    if i >= c.arg_count() {
        return Ok(None);
    }
    Ok(opt(c.arg(i).await?))
}

fn collectors(c: &crate::call::ScriptCall<'_>) -> Res<std::sync::Arc<Collectors>> {
    c.ctx.collectors.clone().ok_or_else(|| {
        err(
            codes::NATIVE_UNAVAILABLE,
            "Collectors are not available here.",
        )
    })
}

pub fn register(d: &mut Definitions) {
    d.add(
        "awaitMessage",
        "collectors",
        1,
        3,
        "[segundos;usuario?;canal?]",
        "Awaits a message and returns its content or empty on timeout.",
        |c| {
            Box::pin(async move {
                let ms = duration_ms(&c.arg(0).await?)?;
                let user = opt_arg(c, 1).await?.map(|s| user_id(&s)).transpose()?;
                let channel = opt_arg(c, 2)
                    .await?
                    .map(|s| channel_id(&s))
                    .transpose()?
                    .unwrap_or(c.ctx.channel_id());
                let collectors = collectors(c)?;
                let collector = collectors.messages(move |m| {
                    m.channel_id == channel && user.map(|u| m.author.id == u).unwrap_or(true)
                });
                let timeout = Duration::from_millis(ms);
                let collected = c
                    .wait(async move { Ok(collector.next(timeout).await) })
                    .await?;
                Ok(collected
                    .map(|m| Value::String(m.content))
                    .unwrap_or(Value::String(String::new())))
            })
        },
    );

    d.add(
        "awaitReaction",
        "collectors",
        1,
        3,
        "[segundos;emoji?;mensagem?]",
        "Awaits a reaction and returns the user ID or empty on timeout.",
        |c| {
            Box::pin(async move {
                let ms = duration_ms(&c.arg(0).await?)?;
                let emoji = opt_arg(c, 1)
                    .await?
                    .map(|s| reaction_emoji(&s))
                    .unwrap_or_else(|| "✅".to_string());
                let message = opt_arg(c, 2)
                    .await?
                    .map(|s| snowflake(&s, "message"))
                    .transpose()?
                    .unwrap_or(c.ctx.message_id());
                let collectors = collectors(c)?;
                let collector = collectors.reactions(move |r| {
                    r.message_id == message
                        && (r.emoji.name == emoji
                            || format!(
                                "{}:{}",
                                r.emoji.name,
                                r.emoji.id.map(|i| i.to_string()).unwrap_or_default()
                            ) == emoji)
                });
                let timeout = Duration::from_millis(ms);
                let selected = c
                    .wait(async move { Ok(collector.next(timeout).await) })
                    .await?;
                Ok(selected
                    .map(|r| Value::String(r.user_id.to_string()))
                    .unwrap_or(Value::String(String::new())))
            })
        },
    );

    d.add(
        "confirm",
        "collectors",
        1,
        2,
        "[segundos;texto?]",
        "Asks for confirmation with ✅/❌ and returns a boolean.",
        |c| {
            Box::pin(async move {
                let ms = duration_ms(&c.arg(0).await?)?;
                let text = opt_arg(c, 1)
                    .await?
                    .unwrap_or_else(|| "Confirme reagindo abaixo.".to_string());
                let channel = c.ctx.channel_id();
                let author = c.ctx.author_id();
                let prompt = rest(
                    c,
                    c.ctx.rest.send_message(
                        channel,
                        &MessageSend {
                            content: Some(text),
                            ..Default::default()
                        },
                    ),
                )
                .await?;
                let ok = flush_core::ReactionEmoji {
                    id: None,
                    name: "✅".into(),
                };
                let no = flush_core::ReactionEmoji {
                    id: None,
                    name: "❌".into(),
                };
                let _ = rest(c, c.ctx.rest.add_reaction(channel, prompt.id, &ok, None)).await;
                let _ = rest(c, c.ctx.rest.add_reaction(channel, prompt.id, &no, None)).await;
                let collectors = collectors(c)?;
                let message_id = prompt.id;
                let collector = collectors.reactions(move |r| {
                    r.message_id == message_id
                        && r.user_id == author
                        && (r.emoji.name == "✅" || r.emoji.name == "❌")
                });
                let timeout = Duration::from_millis(ms);
                let selected = c
                    .wait(async move { Ok(collector.next(timeout).await) })
                    .await?;
                let _ = rest(c, c.ctx.rest.clear_reactions(channel, message_id, None)).await;
                Ok(Value::Bool(
                    selected.map(|r| r.emoji.name == "✅").unwrap_or(false),
                ))
            })
        },
    );
}
