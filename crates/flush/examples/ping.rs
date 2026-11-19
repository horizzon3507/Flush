//! Example: a minimal `!ping` bot on the Flush framework.
//!
//! Shows these framework features:
//! - `enable_help()`: `help` generated from the registered commands
//! - per-command permission gate (`required_permission`)
//! - event hooks (`Hooks`): logs member joins
//! - mod-log via `ModAction.log_channel`
//!
//! ```bash
//! FLUXER_BOT_TOKEN=... cargo run -p flush-lib --example ping
//! ```

use std::future::Future;
use std::pin::Pin;

use flush_core::{EmbedBuilder, MessageSend, Snowflake};
use flush_framework::{
    with_mod_action, Bot, BotConfig, CaseStore, CaseType, Command, CommandCtx, CommandError,
    CommandResult, HookEvent, Hooks, ModAction, PermissionAction,
};

struct Ping;

impl Command for Ping {
    fn name(&self) -> &str {
        "ping"
    }
    fn description(&self) -> &str {
        "Latency (round-trip), as in Sour"
    }
    fn cooldown_ms(&self) -> u64 {
        3_000
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let sent = ctx
                .rest
                .reply(
                    ctx.channel_id(),
                    ctx.message,
                    MessageSend::text("🏓 Pong! (editing with the round-trip…)"),
                    false,
                )
                .await?;
            // Round-trip: snowflake difference ≈ ms since the epoch.
            let dt = sent.id.get().saturating_sub(ctx.message.id.get()) >> 22;
            // Every registered command shows up in the built-in `help`.
            let embed = EmbedBuilder::new()
                .title("🏓 Pong!")
                .description(format!(
                    "**Latency**\n-# ╰› Round-trip: **{dt}ms**\n-# ╰› Use `{p}help` to see my commands.",
                    p = ctx.prefix
                ))
                .color(0xFCE300)
                .footer("Flush")
                .build()
                .map_err(|e| CommandError::Internal(e.to_string()))?;
            ctx.rest
                .edit_message(
                    ctx.channel_id(),
                    sent.id,
                    &flush_rest::EditMessageBody {
                        content: None,
                        embeds: Some(vec![embed]),
                    },
                )
                .await?;
            // Lists the message's reactions (REST reactions feature).
            for r in &sent.reactions {
                let _ = ctx
                    .rest
                    .reaction_users(ctx.channel_id(), sent.id, &r.emoji, 5)
                    .await;
            }
            Ok(())
        })
    }
}

/// `!warn <target> [reason]`: the standard moderation flow with a permission gate.
struct Warn;

impl Command for Warn {
    fn name(&self) -> &str {
        "warn"
    }
    fn description(&self) -> &str {
        "Warns a member: warn <target> [reason]"
    }
    fn usage(&self) -> Option<&str> {
        Some("warn <target> [reason]")
    }
    fn required_permission(&self) -> Option<PermissionAction> {
        Some(PermissionAction::Moderate)
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let Some(guild) = ctx.guild_id() else {
                ctx.reply_no_ping("❌ This command only works in servers.")
                    .await?;
                return Ok(());
            };
            let Some(mention) = ctx.args.first() else {
                ctx.reply_no_ping("❌ Usage: `warn <target> [reason]`")
                    .await?;
                return Ok(());
            };
            let reason = ctx
                .args
                .iter()
                .skip(1)
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            let reason = if reason.is_empty() {
                "no reason".to_owned()
            } else {
                reason
            };
            let target = ctx.target_member(mention).await?;
            let mut store = CaseStore::with_default_path();
            let case = store.add(
                &guild.to_string(),
                CaseType::Warn,
                &target
                    .as_ref()
                    .map(|t| t.user.id.to_string())
                    .unwrap_or_default(),
                &ctx.message.author.id.to_string(),
                &reason,
            );
            // Log channel configured in guilds.json (MOD_LOG_CHANNEL_ID).
            let log_channel = flush_framework::GuildStore::with_default_path()
                .get(&guild.to_string())
                .mod_log_channel_id
                .and_then(|s| s.parse::<Snowflake>().ok());
            with_mod_action(ModAction {
                ctx: &ctx,
                executor: ctx.member.as_ref(),
                action: PermissionAction::Moderate,
                target: target.as_ref(),
                reason: &reason,
                success_icon: "⚠️",
                success_description: &format!("**{mention}** received a warning."),
                case_id: case.id,
                extra: None,
                log_channel,
                run: || async move { Ok::<(), String>(()) },
            })
            .await?;
            Ok(())
        })
    }
}

/// Example hooks: reports member joins on stdout.
struct JoinLogger;

impl Hooks for JoinLogger {
    fn dispatch<'a>(&'a self, ev: HookEvent<'a>) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            match ev {
                HookEvent::MemberJoin { guild, member } => {
                    println!(
                        "👤 {} ({}) joined {} ({})",
                        member.user.username, member.user.id, guild.name, guild.id
                    );
                }
                HookEvent::Ready { .. }
                | HookEvent::GuildAdd(_)
                | HookEvent::MemberLeave { .. }
                | HookEvent::Message(_)
                | HookEvent::ShardStatus { .. }
                | HookEvent::Raw(_) => {}
            }
        })
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("FLUXER_BOT_TOKEN").is_err() {
        return Err("FLUXER_BOT_TOKEN is not set".into());
    }
    async_std::task::block_on(run())
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let token = std::env::var("FLUXER_BOT_TOKEN").unwrap();
    let mut cfg = BotConfig::new(token);
    cfg.presence_lines = vec!["flush/ping".into(), "flush framework".into()];
    let mut bot = Bot::new(cfg)?
        .command(Ping)
        .command(Warn)
        .hooks(JoinLogger)
        .enable_help();
    bot.run().await.map_err(|e| e.to_string())?;
    Ok(())
}
