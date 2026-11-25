use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};

use flush::framework::parse_duration;
use flush::{
    with_mod_action, CaseStore, CaseType, Command, CommandCtx, CommandResult, ModAction,
    PermissionAction,
};

use crate::util::{mod_log_channel, reason_from};

const MAX_TIMEOUT_SECS: u64 = 28 * 24 * 60 * 60;

pub struct Timeout;

impl Command for Timeout {
    fn name(&self) -> &str {
        "timeout"
    }
    fn description(&self) -> &str {
        "Times a member out (1 second to 28 days) and records a case."
    }
    fn usage(&self) -> Option<&str> {
        Some("timeout <member> <duration> [reason]")
    }
    fn required_permission(&self) -> Option<PermissionAction> {
        Some(PermissionAction::Timeout)
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
                ctx.reply_no_ping(format!(
                    "Usage: `{}timeout <member> <duration> [reason]`",
                    ctx.prefix
                ))
                .await?;
                return Ok(());
            };
            let Some(target) = ctx.target_member(raw).await? else {
                ctx.reply_no_ping("Member not found.").await?;
                return Ok(());
            };
            let duration = ctx.args.get(1).and_then(|raw| parse_duration(raw));
            let Some(duration) = duration.filter(|d| (1..=MAX_TIMEOUT_SECS).contains(&d.as_secs()))
            else {
                ctx.reply_no_ping("Duration must be between 1 second and 28 days, e.g. `10m`.")
                    .await?;
                return Ok(());
            };
            let until = crate::util::iso_utc(crate::util::unix_now() + duration.as_secs());
            let reason = reason_from(&ctx.args, 2);
            let user = target.user.id;
            // The case is recorded whenever the action went through, even if
            // the confirmation reply then fails.
            let mut cases = CaseStore::with_default_path();
            let case_id = cases.next_id(&guild.to_string());
            let rest = ctx.rest;
            let audit = reason.clone();
            let acted = AtomicBool::new(false);
            let went_through = &acted;
            let outcome = with_mod_action(ModAction {
                ctx: &ctx,
                executor: ctx.member.as_ref(),
                action: PermissionAction::Timeout,
                target: Some(&target),
                reason: &reason,
                success_icon: "✅",
                success_description: &format!("**{}** was timed out.", target.user.username),
                case_id,
                extra: None,
                log_channel: mod_log_channel(guild),
                run: || async move {
                    let result = rest
                        .set_timeout(guild, user, Some(until), Some(&audit), Some(&audit))
                        .await
                        .map(|_| ())
                        .map_err(|e| e.to_string());
                    went_through.store(result.is_ok(), Ordering::Relaxed);
                    result
                },
            })
            .await;
            if acted.load(Ordering::Relaxed) {
                let saved = cases.add(
                    &guild.to_string(),
                    CaseType::Timeout,
                    &user.to_string(),
                    &ctx.message.author.id.to_string(),
                    &reason,
                );
                if saved.is_err() {
                    ctx.reply_no_ping(format!("⚠️ Case #{case_id} could not be saved."))
                        .await?;
                }
            }
            outcome.map(|_| ())
        })
    }
}
