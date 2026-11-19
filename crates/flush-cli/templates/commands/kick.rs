use std::future::Future;
use std::pin::Pin;

use flush::{
    with_mod_action, CaseStore, CaseType, Command, CommandCtx, CommandResult, ModAction,
    PermissionAction,
};

use crate::util::{mod_log_channel, reason_from};

pub struct Kick;

impl Command for Kick {
    fn name(&self) -> &str {
        "kick"
    }
    fn description(&self) -> &str {
        "Kicks a member and records a case."
    }
    fn usage(&self) -> Option<&str> {
        Some("kick <member> [reason]")
    }
    fn required_permission(&self) -> Option<PermissionAction> {
        Some(PermissionAction::Kick)
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
            let Some(raw) = ctx.args.first() else {
                ctx.reply_no_ping(format!("Usage: `{}kick <member> [reason]`", ctx.prefix))
                    .await?;
                return Ok(());
            };
            let Some(target) = ctx.target_member(raw).await? else {
                ctx.reply_no_ping("Member not found.").await?;
                return Ok(());
            };
            let reason = reason_from(&ctx.args, 1);
            let user = target.user.id;
            // The case is only recorded once the action went through.
            let mut cases = CaseStore::with_default_path();
            let case_id = cases.next_id(&guild.to_string());
            let rest = ctx.rest;
            let audit = reason.clone();
            let done = with_mod_action(ModAction {
                ctx: &ctx,
                executor: ctx.member.as_ref(),
                action: PermissionAction::Kick,
                target: Some(&target),
                reason: &reason,
                success_icon: "✅",
                success_description: &format!("**{}** was kicked.", target.user.username),
                case_id,
                extra: None,
                log_channel: mod_log_channel(guild),
                run: || async move {
                    rest.kick(guild, user, Some(&audit))
                        .await
                        .map_err(|e| e.to_string())
                },
            })
            .await?;
            if done {
                cases.add(
                    &guild.to_string(),
                    CaseType::Kick,
                    &user.to_string(),
                    &ctx.message.author.id.to_string(),
                    &reason,
                );
            }
            Ok(())
        })
    }
}
