//! yuzuha `native-webhooks.ts`.

use serde_json::Value;

use flush_core::Permissions;
use flush_rest::resources::WebhookPatch;

use crate::errors::codes;
use crate::types::Definitions;

use super::common::{channel_id, err, require_perms, rest, serialize, snowflake, Res};

fn required(raw: &str, field: &str) -> Res<String> {
    if raw.trim().is_empty() {
        return Err(err(
            codes::INVALID_ARGUMENTS,
            format!("Missing the webhook {field}."),
        ));
    }
    Ok(raw.to_string())
}

pub fn register(d: &mut Definitions) {
    d.add(
        "createWebhook",
        "webhooks",
        2,
        2,
        "[canal;nome]",
        "Creates a webhook in the channel and returns its information.",
        |c| {
            Box::pin(async move {
                let channel = channel_id(&c.arg(0).await?)?;
                let name = required(&c.arg(1).await?, "name")?;
                let target = super::common::context_channel(c, Some(channel)).await?;
                require_perms(c, Some(&target), Permissions::MANAGE_WEBHOOKS).await?;
                let webhook =
                    rest(c, c.ctx.rest.create_webhook(channel, &name, None, None)).await?;
                Ok(super::common::camelize(&serialize(&webhook)))
            })
        },
    );

    d.add(
        "webhooks",
        "webhooks",
        0,
        1,
        "[canal?]",
        "Lists webhooks of the channel (default: current channel).",
        |c| {
            Box::pin(async move {
                let channel = if c.arg_count() > 0 {
                    channel_id(&c.arg(0).await?)?
                } else {
                    c.ctx.channel_id()
                };
                let target = super::common::context_channel(c, Some(channel)).await?;
                require_perms(c, Some(&target), Permissions::MANAGE_WEBHOOKS).await?;
                let webhooks = rest(c, c.ctx.rest.channel_webhooks(channel)).await?;
                Ok(Value::Array(
                    webhooks
                        .iter()
                        .map(|w| super::common::camelize(&serialize(w)))
                        .collect(),
                ))
            })
        },
    );

    d.add(
        "sendWebhook",
        "webhooks",
        3,
        3,
        "[id;token;texto]",
        "Sends through a webhook of the current guild with ManageWebhooks of author and bot; the token does not bypass authorization.",
        |c| {
            Box::pin(async move {
                let id = snowflake(&required(&c.arg(0).await?, "ID")?, "webhook")?;
                let token = required(&c.arg(1).await?, "token")?;
                let content = required(&c.arg(2).await?, "text")?;
                c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_WEBHOOKS).await?;
                let execute = flush_core::webhook::WebhookExecute {
                    content: Some(content),
                    ..Default::default()
                };
                let message =
                    rest(c, c.ctx.rest.execute_webhook(id, &token, &execute, true)).await?;
                Ok(match message {
                    Some(message) => Value::String(message.id.to_string()),
                    None => Value::String(String::new()),
                })
            })
        },
    );

    d.add(
        "editWebhook",
        "webhooks",
        2,
        3,
        "[id;nome;token?]",
        "Renames a webhook of the current guild with ManageWebhooks; without a token it uses the bot authentication.",
        |c| {
            Box::pin(async move {
                let id = snowflake(&required(&c.arg(0).await?, "ID")?, "webhook")?;
                let name = required(&c.arg(1).await?, "new name")?;
                let token = if c.arg_count() > 2 {
                    let t = c.arg(2).await?;
                    if t.trim().is_empty() {
                        None
                    } else {
                        Some(t)
                    }
                } else {
                    None
                };
                c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_WEBHOOKS).await?;
                let patch = WebhookPatch {
                    name: Some(name),
                    ..Default::default()
                };
                match &token {
                    Some(token) => {
                        let _ = rest(
                            c,
                            c.ctx.rest.edit_webhook_with_token(id, token, &patch),
                        )
                        .await?;
                    }
                    None => {
                        let _ = rest(c, c.ctx.rest.edit_webhook(id, &patch, None)).await?;
                    }
                }
                Ok(Value::Null)
            })
        },
    );

    d.add(
        "deleteWebhook",
        "webhooks",
        1,
        2,
        "[id;token?]",
        "Deletes a webhook of the current guild with ManageWebhooks; the token does not bypass authorization.",
        |c| {
            Box::pin(async move {
                let id = snowflake(&required(&c.arg(0).await?, "ID")?, "webhook")?;
                let token = if c.arg_count() > 1 {
                    let t = c.arg(1).await?;
                    if t.trim().is_empty() {
                        None
                    } else {
                        Some(t)
                    }
                } else {
                    None
                };
                c.ctx.require_guild()?;
                require_perms(c, None, Permissions::MANAGE_WEBHOOKS).await?;
                match &token {
                    Some(token) => {
                        rest(c, c.ctx.rest.delete_webhook_with_token(id, token)).await?
                    }
                    None => rest(c, c.ctx.rest.delete_webhook(id, None)).await?,
                }
                Ok(Value::Null)
            })
        },
    );
}
