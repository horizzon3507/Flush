//! Command middleware: checks that run after a command is resolved and
//! before its permission gate and `execute`.

use std::future::Future;
use std::pin::Pin;

use crate::command::CommandCtx;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flow {
    Continue,
    /// Stops the command. `Some(text)` is sent as a no-ping reply.
    Stop(Option<String>),
}

pub trait Middleware: Send + Sync {
    fn before<'a>(
        &'a self,
        command: &'a str,
        ctx: &'a CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = Flow> + Send + 'a>>;
}

/// Middleware from a synchronous predicate over `(command, ctx)`.
pub struct Guard<F>(pub F);

impl<F> Middleware for Guard<F>
where
    F: Fn(&str, &CommandCtx<'_>) -> Flow + Send + Sync,
{
    fn before<'a>(
        &'a self,
        command: &'a str,
        ctx: &'a CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = Flow> + Send + 'a>> {
        let flow = (self.0)(command, ctx);
        Box::pin(async move { flow })
    }
}

/// Blocks commands outside guilds.
pub fn guild_only(message: impl Into<String>) -> impl Middleware {
    let message = message.into();
    Guard(move |_: &str, ctx: &CommandCtx<'_>| {
        if ctx.guild_id().is_some() {
            Flow::Continue
        } else {
            Flow::Stop(Some(message.clone()))
        }
    })
}
