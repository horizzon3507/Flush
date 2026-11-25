use std::future::Future;
use std::pin::Pin;

use flush::{Command, CommandCtx, CommandResult};

use crate::util::random_below;

pub struct Choose;

/// Splits `a | b | c` into 2 to 20 non-empty options of up to 100 characters.
pub fn options(input: &str) -> Option<Vec<&str>> {
    let options: Vec<&str> = input.split('|').map(str::trim).collect();
    let valid = (2..=20).contains(&options.len())
        && options
            .iter()
            .all(|option| !option.is_empty() && option.chars().count() <= 100);
    valid.then_some(options)
}

impl Command for Choose {
    fn name(&self) -> &str {
        "choose"
    }
    fn description(&self) -> &str {
        "Picks one of 2 to 20 options separated by |."
    }
    fn usage(&self) -> Option<&str> {
        Some("choose <a | b | ...>")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let input = ctx.args.join(" ");
            let Some(options) = options(&input) else {
                ctx.reply_no_ping(
                    "Give 2 to 20 options separated by `|`, up to 100 characters each.",
                )
                .await?;
                return Ok(());
            };
            let pick = options[random_below(options.len() as u64) as usize];
            ctx.reply_no_ping(format!("I choose: {pick}")).await?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::options;

    #[test]
    fn splits_and_validates_options() {
        assert_eq!(options("pizza | sushi"), Some(vec!["pizza", "sushi"]));
        assert_eq!(options("only one"), None);
        assert_eq!(options("a | | c"), None);
    }
}
