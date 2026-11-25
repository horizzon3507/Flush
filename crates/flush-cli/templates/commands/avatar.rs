use std::future::Future;
use std::pin::Pin;

use flush::core::display_avatar_url;
use flush::{Command, CommandCtx, CommandResult};

pub struct Avatar;

impl Command for Avatar {
    fn name(&self) -> &str {
        "avatar"
    }
    fn description(&self) -> &str {
        "Shows a member's avatar, or yours."
    }
    fn usage(&self) -> Option<&str> {
        Some("avatar [@member]")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let user = match ctx.args.first() {
                Some(raw) => match ctx.target_member(raw).await? {
                    Some(member) => member.user,
                    None => {
                        ctx.reply_no_ping("Member not found.").await?;
                        return Ok(());
                    }
                },
                None => ctx.message.author.clone(),
            };
            let url = display_avatar_url(user.id, user.avatar.as_deref(), Some(512));
            ctx.reply_no_ping(format!("Avatar of **{}**: {url}", user.username))
                .await?;
            Ok(())
        })
    }
}
