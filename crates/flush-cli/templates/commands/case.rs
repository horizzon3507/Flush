use std::future::Future;
use std::pin::Pin;

use flush::{CaseStore, Command, CommandCtx, CommandResult, PermissionAction};

pub struct Case;

impl Command for Case {
    fn name(&self) -> &str {
        "case"
    }
    fn description(&self) -> &str {
        "Shows a moderation case by number."
    }
    fn usage(&self) -> Option<&str> {
        Some("case <number>")
    }
    fn required_permission(&self) -> Option<PermissionAction> {
        Some(PermissionAction::Guild)
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
            let Some(id) = ctx.args.first().and_then(|raw| raw.parse::<u64>().ok()) else {
                ctx.reply_no_ping(format!("Usage: `{}case <number>`", ctx.prefix))
                    .await?;
                return Ok(());
            };
            let reply = match CaseStore::with_default_path().get(&guild.to_string(), id) {
                Some(case) => format!(
                    "**Case #{}** ({:?})\n-# Target: {}\n-# Moderator: {}\n-# Reason: {}",
                    case.id, case.kind, case.target_id, case.moderator_id, case.reason
                ),
                None => format!("Case #{id} not found."),
            };
            ctx.reply_no_ping(reply).await?;
            Ok(())
        })
    }
}
