use std::future::Future;
use std::pin::Pin;

use flush::rest::EditMessageBody;
use flush::{Command, CommandCtx, CommandResult};

pub struct Ping;

impl Command for Ping {
    fn name(&self) -> &str {
        "ping"
    }
    fn description(&self) -> &str {
        "Bot latency."
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let sent = ctx.reply_no_ping("Pong!").await?;
            // Snowflakes carry a millisecond timestamp in their upper bits.
            let ms = (sent.id.get() >> 22).saturating_sub(ctx.message.id.get() >> 22);
            let body = EditMessageBody {
                content: Some(format!("Pong! {ms} ms")),
                embeds: None,
            };
            ctx.rest
                .edit_message(ctx.channel_id(), sent.id, &body)
                .await?;
            Ok(())
        })
    }
}
