use std::future::Future;
use std::pin::Pin;

use flush::{Command, CommandCtx, CommandResult};

use crate::util::random_below;

pub struct Coin {
    aliases: Vec<String>,
}

impl Default for Coin {
    fn default() -> Self {
        Self {
            aliases: vec!["coinflip".to_owned()],
        }
    }
}

impl Command for Coin {
    fn name(&self) -> &str {
        "coin"
    }
    fn description(&self) -> &str {
        "Flips a coin: heads or tails."
    }
    fn aliases(&self) -> &[String] {
        &self.aliases
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let side = if random_below(2) == 0 {
                "Heads!"
            } else {
                "Tails!"
            };
            ctx.reply_no_ping(side).await?;
            Ok(())
        })
    }
}
