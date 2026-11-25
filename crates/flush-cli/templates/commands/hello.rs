use std::future::Future;
use std::pin::Pin;

use flush::{Command, CommandCtx, CommandResult};

pub struct Hello;

pub fn greeting(name: Option<&str>) -> String {
    format!("Hello, {}!", name.unwrap_or("world"))
}

impl Command for Hello {
    fn name(&self) -> &str {
        "hello"
    }
    fn description(&self) -> &str {
        "Example command with an optional argument."
    }
    fn usage(&self) -> Option<&str> {
        Some("hello [name]")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let name = ctx.args.first().map(String::as_str);
            ctx.reply_no_ping(greeting(name)).await?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::greeting;

    #[test]
    fn greets_the_given_name_or_the_world() {
        assert_eq!(greeting(Some("Flush")), "Hello, Flush!");
        assert_eq!(greeting(None), "Hello, world!");
    }
}
