//! Port of `src/functions/builtins/guards.ts`.

use serde_json::Value;

use crate::errors::{codes, FnError, GuardError, GuardReason};
use crate::types::Definitions;
use crate::values::truthy;

use super::support;

pub fn register(d: &mut Definitions) {
    d.add_aliased(
        "onlyIf",
        "guards",
        2,
        2,
        "[condição;mensagem?]",
        "Aborts the command unless the condition is truthy or an allowed comparison holds.",
        |c| {
            Box::pin(async move {
                let condition = Value::String(c.arg(0).await?);
                let message = c.arg(1).await?;
                // `onlyIf` accepts comparisons: "a == b", "a > b", ...
                let ok = truthy(&condition);
                if !ok {
                    let message = if message.is_empty() {
                        "Check failed.".to_string()
                    } else {
                        message
                    };
                    return Err(GuardError::new(message, GuardReason::CheckFailed).into());
                }
                Ok(Value::Null)
            })
        },
        &["require"],
    );

    d.add_aliased(
        "requireGuild",
        "guards",
        0,
        1,
        "[mensagem?]",
        "Aborts the command outside of a guild.",
        |c| {
            Box::pin(async move {
                let message = c.arg(0).await?;
                match c.ctx.require_guild() {
                    Ok(_) => Ok(Value::Null),
                    Err(_) => Err(GuardError::new(
                        if message.is_empty() {
                            crate::context::NOT_IN_GUILD.to_string()
                        } else {
                            message
                        },
                        GuardReason::NotInGuild,
                    )
                    .into()),
                }
            })
        },
        &["onlyGuild"],
    );

    d.add_aliased(
        "onlyPerms",
        "guards",
        1,
        2,
        "[permissão;mensagem?]",
        "Aborts the command unless the author has the named permission.",
        |c| {
            Box::pin(async move {
                let name = c.arg(0).await?;
                let message = c.arg(1).await?;
                support::require_permission(c, &name, false, opt(message)).await?;
                Ok(Value::Null)
            })
        },
        &["requirePermission"],
    );

    d.add_aliased(
        "onlyBotPerms",
        "guards",
        1,
        2,
        "[permissão;mensagem?]",
        "Aborts the command unless the bot has the named permission.",
        |c| {
            Box::pin(async move {
                let name = c.arg(0).await?;
                let message = c.arg(1).await?;
                support::require_permission(c, &name, true, opt(message)).await?;
                Ok(Value::Null)
            })
        },
        &["requireBotPermission"],
    );

    d.add(
        "cooldown",
        "guards",
        1,
        2,
        "[duração;mensagem?]",
        "Claims a per-command cooldown for the author.",
        |c| {
            Box::pin(async move {
                let duration = support::parse_duration(&Value::String(c.arg(0).await?))?;
                let message = c.arg(1).await?;
                let cooldowns = c.ctx.cooldowns.clone().ok_or_else(|| {
                    FnError::new(
                        codes::NATIVE_UNAVAILABLE,
                        "Cooldown tracking is not configured.",
                    )
                })?;
                let author = c.ctx.author_id();
                let key = format!(
                    "{}:{}:{}:{}",
                    c.ctx.guild_id().get(),
                    c.ctx.channel_id().get(),
                    author.get(),
                    super::context::command_name(c.ctx),
                );
                let now = (c.ctx.now)();
                let mut table = cooldowns.lock().unwrap();
                match table.get(&key) {
                    Some(until) if *until > now => {
                        let remaining = until - now;
                        let seconds = remaining.div_ceil(1_000);
                        let formatted = format!("{seconds}s");
                        let text_ = if message.is_empty() {
                            format!("Please wait {formatted} before using this command again.")
                        } else {
                            message.replace("{remaining}", &formatted)
                        };
                        Err(GuardError::cooldown(text_, remaining).into())
                    }
                    _ => {
                        table.insert(key, now + duration);
                        Ok(Value::Null)
                    }
                }
            })
        },
    );
}

fn opt(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
