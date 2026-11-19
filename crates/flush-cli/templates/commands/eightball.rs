use std::future::Future;
use std::pin::Pin;

use flush::{Command, CommandCtx, CommandResult};

use crate::util::random_below;

const ANSWERS: [&str; 8] = [
    "Yes.",
    "No.",
    "Maybe.",
    "Definitely.",
    "I would not count on it.",
    "Ask again later.",
    "Signs point to yes.",
    "Very doubtful.",
];

pub struct EightBall {
    aliases: Vec<String>,
}

impl Default for EightBall {
    fn default() -> Self {
        Self {
            aliases: vec!["8ball".to_owned()],
        }
    }
}

impl Command for EightBall {
    fn name(&self) -> &str {
        "eightball"
    }
    fn description(&self) -> &str {
        "Answers a question at random, just for fun."
    }
    fn usage(&self) -> Option<&str> {
        Some("eightball <question>")
    }
    fn aliases(&self) -> &[String] {
        &self.aliases
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            if ctx.args.is_empty() {
                ctx.reply_no_ping(format!("Usage: `{}eightball <question>`", ctx.prefix))
                    .await?;
                return Ok(());
            }
            let answer = ANSWERS[random_below(ANSWERS.len() as u64) as usize];
            ctx.reply_no_ping(answer).await?;
            Ok(())
        })
    }
}
