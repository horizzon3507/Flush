//! Port of `src/functions/builtins/context.ts`.

use serde_json::Value;

use crate::types::Definitions;

use super::support;

/// The command name invoked (first token of the message minus the prefix).
pub fn command_name(ctx: &crate::context::ScriptContext) -> String {
    let content = ctx.message.content.trim();
    let rest = content.strip_prefix(ctx.prefix.as_str()).unwrap_or(content);
    rest.split_whitespace()
        .next()
        .unwrap_or_default()
        .to_lowercase()
}

pub fn register(d: &mut Definitions) {
    d.add(
        "authorID",
        "context",
        0,
        0,
        "",
        "The id of the command author.",
        |c| Box::pin(async move { Ok(Value::String(c.ctx.author_id().to_string())) }),
    );

    d.add_aliased(
        "authorName",
        "context",
        0,
        0,
        "",
        "The username of the command author.",
        |c| Box::pin(async move { Ok(Value::String(c.ctx.author_name())) }),
        &["username"],
    );

    d.add("botID", "context", 0, 0, "", "The bot's user id.", |c| {
        Box::pin(async move {
            Ok(Value::String(
                c.ctx
                    .bot_id
                    .map(|id| id.get().to_string())
                    .unwrap_or_default(),
            ))
        })
    });

    d.add(
        "guildID",
        "context",
        0,
        0,
        "",
        "The current guild id.",
        |c| Box::pin(async move { Ok(Value::String(support::id_text(c.ctx.guild_id().get()))) }),
    );

    d.add(
        "channelID",
        "context",
        0,
        0,
        "",
        "The current channel id.",
        |c| Box::pin(async move { Ok(Value::String(c.ctx.channel_id().to_string())) }),
    );

    d.add(
        "messageID",
        "context",
        0,
        0,
        "",
        "The triggering message id.",
        |c| Box::pin(async move { Ok(Value::String(c.ctx.message_id().to_string())) }),
    );

    d.add(
        "ping",
        "context",
        0,
        0,
        "",
        "The bot's gateway latency in milliseconds.",
        |_| Box::pin(async move { Ok(Value::Null) }),
    );

    d.add(
        "caseID",
        "context",
        0,
        0,
        "",
        "The id of the last recorded moderation case.",
        |c| {
            Box::pin(async move {
                let id = c
                    .ctx
                    .last_case
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|case| case.id.to_string())
                    .unwrap_or_default();
                Ok(Value::String(id))
            })
        },
    );

    d.add(
        "help",
        "context",
        0,
        0,
        "",
        "The help text of the invoked command.",
        |_| Box::pin(async move { Ok(Value::Null) }),
    );

    d.add(
        "commandName",
        "context",
        0,
        0,
        "",
        "The invoked command name.",
        |c| Box::pin(async move { Ok(Value::String(command_name(c.ctx))) }),
    );

    d.add(
        "mentioned",
        "context",
        1,
        1,
        "[posição]",
        "The mentioned user at a 1-based position.",
        |c| {
            Box::pin(async move {
                let position = support::index(&Value::String(c.arg(0).await?), 1)?;
                let user = c
                    .ctx
                    .message
                    .mentions
                    .get(position - 1)
                    .map(|u| u.id.to_string())
                    .unwrap_or_default();
                Ok(Value::String(user))
            })
        },
    );

    d.add(
        "message",
        "context",
        0,
        1,
        "[posição?]",
        "The full input text or the token at a 1-based position.",
        |c| {
            Box::pin(async move {
                if c.arg_count() == 0 {
                    return Ok(Value::String(c.ctx.input.clone()));
                }
                let position = support::index(&Value::String(c.arg(0).await?), 1)?;
                let tokens = support::tokenize(&c.ctx.input)?;
                Ok(Value::String(
                    tokens.get(position - 1).cloned().unwrap_or_default(),
                ))
            })
        },
    );

    d.add(
        "messageSlice",
        "context",
        1,
        2,
        "[início;fim?]",
        "A slice of the input tokens joined by spaces.",
        |c| {
            Box::pin(async move {
                let start = support::index(&Value::String(c.arg(0).await?), 0)?;
                let tokens = support::tokenize(&c.ctx.input)?;
                let end = if c.arg_count() > 1 {
                    support::index(&Value::String(c.arg(1).await?), 0)?
                } else {
                    tokens.len()
                };
                let from = start.min(tokens.len());
                let slice = &tokens[from..end.min(tokens.len()).max(from)];
                Ok(Value::String(slice.join(" ")))
            })
        },
    );

    d.add(
        "arg",
        "context",
        1,
        1,
        "[nome]",
        "The value of a named command argument.",
        |_| Box::pin(async move { Ok(Value::Null) }),
    );

    d.add(
        "tokens",
        "context",
        0,
        0,
        "",
        "The input split into tokens.",
        |c| {
            Box::pin(async move {
                Ok(Value::Array(
                    support::tokenize(&c.ctx.input)?
                        .into_iter()
                        .map(Value::String)
                        .collect(),
                ))
            })
        },
    );
}
