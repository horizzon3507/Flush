use std::future::Future;
use std::pin::Pin;

use flush::{Command, CommandCtx, CommandResult};

use crate::util::random_below;

pub struct Dice;

/// Number of sides: 6 by default, 2 to 100 otherwise.
pub fn sides(raw: Option<&str>) -> Option<u64> {
    match raw {
        None => Some(6),
        Some(raw) => raw.parse().ok().filter(|n| (2..=100).contains(n)),
    }
}

impl Command for Dice {
    fn name(&self) -> &str {
        "dice"
    }
    fn description(&self) -> &str {
        "Rolls a die with 2 to 100 sides (6 by default)."
    }
    fn usage(&self) -> Option<&str> {
        Some("dice [sides]")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let Some(sides) = sides(ctx.args.first().map(String::as_str)) else {
                ctx.reply_no_ping("Sides must be a number between 2 and 100.")
                    .await?;
                return Ok(());
            };
            let roll = random_below(sides) + 1;
            ctx.reply_no_ping(format!("{sides}-sided die: {roll}."))
                .await?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::sides;

    #[test]
    fn validates_the_number_of_sides() {
        assert_eq!(sides(None), Some(6));
        assert_eq!(sides(Some("20")), Some(20));
        assert_eq!(sides(Some("1")), None);
        assert_eq!(sides(Some("101")), None);
        assert_eq!(sides(Some("many")), None);
    }
}
