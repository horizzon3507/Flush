use std::future::Future;
use std::pin::Pin;

use flush::framework::parse_user;
use flush::{CaseStore, CaseType, Command, CommandCtx, CommandResult, PermissionAction};

use crate::util::reason_from;

pub struct Unban;

impl Command for Unban {
    fn name(&self) -> &str {
        "unban"
    }
    fn description(&self) -> &str {
        "Removes a ban by user id and records a case."
    }
    fn usage(&self) -> Option<&str> {
        Some("unban <user id> [reason]")
    }
    fn required_permission(&self) -> Option<PermissionAction> {
        Some(PermissionAction::Ban)
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let Some(guild) = ctx.guild_id() else {
                ctx.reply_no_ping("This command only works in servers.")
                    .await?;
                return Ok(());
            };
            let user = ctx.args.first().and_then(|raw| parse_user(raw));
            let Some(user) = user else {
                ctx.reply_no_ping(format!("Usage: `{}unban <user id> [reason]`", ctx.prefix))
                    .await?;
                return Ok(());
            };
            let reason = reason_from(&ctx.args, 1);
            ctx.rest.unban(guild, user, Some(&reason)).await?;
            let saved = CaseStore::with_default_path().add(
                &guild.to_string(),
                CaseType::Unban,
                &user.to_string(),
                &ctx.message.author.id.to_string(),
                &reason,
            );
            let reply = match saved {
                Ok(case) => format!("✅ Ban removed. Case #{}.", case.id),
                Err(_) => "✅ Ban removed, but the case could not be saved.".to_owned(),
            };
            ctx.reply_no_ping(reply).await?;
            Ok(())
        })
    }
}
