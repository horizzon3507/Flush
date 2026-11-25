//! Port of `src/functions/builtins/events.ts`.

use serde_json::Value;

use crate::errors::{GuardError, GuardReason, ScriptFailure};
use crate::types::Definitions;

use super::support;

const NO_EVENT: &str = "This command only runs inside an event.";

fn event_data<'a>(
    c: &'a crate::call::ScriptCall<'a>,
) -> Result<&'a (String, Value), ScriptFailure> {
    c.ctx
        .event
        .as_ref()
        .ok_or_else(|| GuardError::new(NO_EVENT, GuardReason::Generic).into())
}

macro_rules! event_fn {
    ($d:ident, $name:literal, $key:literal, $desc:literal) => {
        $d.add($name, "events", 0, 0, "", $desc, move |c| {
            let key = $key.to_string();
            Box::pin(async move {
                match c.ctx.event.as_ref() {
                    Some((_, data)) => Ok(data.get(&key).cloned().unwrap_or(Value::Null)),
                    None => Err(GuardError::new(NO_EVENT, GuardReason::Generic).into()),
                }
            })
        });
    };
}

pub fn register(d: &mut Definitions) {
    d.add(
        "eventName",
        "events",
        0,
        0,
        "",
        "The name of the current event.",
        |c| {
            Box::pin(async move {
                match c.ctx.event.as_ref() {
                    Some((name, _)) => Ok(Value::String(name.clone())),
                    None => Err(GuardError::new(NO_EVENT, GuardReason::Generic).into()),
                }
            })
        },
    );

    event_fn!(
        d,
        "eventUserID",
        "userID",
        "The user id carried by the event."
    );
    event_fn!(
        d,
        "eventGuildID",
        "guildID",
        "The guild id carried by the event."
    );
    event_fn!(
        d,
        "eventChannelID",
        "channelID",
        "The channel id carried by the event."
    );
    event_fn!(
        d,
        "eventMessageID",
        "messageID",
        "The message id carried by the event."
    );
    event_fn!(
        d,
        "eventContent",
        "content",
        "The new content carried by the event."
    );
    event_fn!(
        d,
        "eventOldContent",
        "oldContent",
        "The previous content carried by the event."
    );
    event_fn!(d, "eventEmoji", "emoji", "The emoji carried by the event.");
    event_fn!(
        d,
        "eventErrorCode",
        "errorCode",
        "The error code carried by the event."
    );
    event_fn!(
        d,
        "eventErrorMessage",
        "errorMessage",
        "The error message carried by the event."
    );

    d.add(
        "eventData",
        "events",
        0,
        2,
        "[caminho?;padrão?]",
        "Reads the event payload, optionally at a nested path.",
        |c| {
            Box::pin(async move {
                let (_, data) = event_data(c)?;
                if c.arg_count() == 0 {
                    return Ok(data.clone());
                }
                let path = c.arg(0).await?;
                let default = if c.arg_count() > 1 {
                    support::json_arg(&c.arg(1).await?)
                } else {
                    Value::Null
                };
                Ok(support::read_path(data, &path).cloned().unwrap_or(default))
            })
        },
    );
}
