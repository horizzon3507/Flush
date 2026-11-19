use std::future::Future;
use std::pin::Pin;

use flush::{Command, CommandCtx, CommandResult};

pub struct About;

impl Command for About {
    fn name(&self) -> &str {
        "about"
    }
    fn description(&self) -> &str {
        "What this bot is and how to list its commands."
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            ctx.reply_no_ping(format!(
                "I am a Fluxer bot built with Flush. Use `{}help` to see my commands.",
                ctx.prefix
            ))
            .await?;
            Ok(())
        })
    }
}
