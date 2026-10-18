//! Exemplo: bot mínimo com `!ping` sobre o Flush (framework).
//!
//! Demostração das features de framework:
//! - `enable_help()`: `help` auto-gerado da lista de comandos registrados
//! - gate de permissão por comando (`required_permission`)
//! - hooks de eventos (`Hooks`) — log de entrada de membro
//! - mod-log via `ModAction.log_channel`
//!
//! ```bash
//! FLUXER_BOT_TOKEN=... cargo run -p flush --example ping
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
        "Latência (round-trip) — espelho do Sour"
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
                    MessageSend::text("🏓 Pong! (editando com round-trip…)"),
                    false,
                )
                .await?;
            // Round-trip: diferença de snowflakes ≈ ms desde o epoch.
            let dt = sent.id.get().saturating_sub(ctx.message.id.get()) >> 22;
            // Todo comando registrado aparece no `help` embutido.
            let embed = EmbedBuilder::new()
                .title("🏓 Pong!")
                .description(format!(
                    "**Latência**\n-# ╰› Round-trip: **{dt}ms**\n-# ╰› Use `{p}help` para ver meus comandos.",
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
            // Lista as reações da mensagem (feature reactions do REST).
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

/// `!warn <alvo> [motivo]` — fluxo padrão de moderação com gate de permissão.
struct Warn;

impl Command for Warn {
    fn name(&self) -> &str {
        "warn"
    }
    fn description(&self) -> &str {
        "Dá um warning a um membro: warn <alvo> [motivo]"
    }
    fn usage(&self) -> Option<&str> {
        Some("warn <alvo> [motivo]")
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
                ctx.reply_no_ping("❌ Este comando só funciona em servidores.")
                    .await?;
                return Ok(());
            };
            let Some(mention) = ctx.args.first() else {
                ctx.reply_no_ping("❌ Uso: `warn <alvo> [motivo]`").await?;
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
                "sem motivo".to_owned()
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
            // Canal de log configurado no guilds.json (MOD_LOG_CHANNEL_ID).
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
                success_description: &format!("**{mention}** recebeu um warning."),
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

/// Hooks de exemplo: relata entradas de membro no stdout.
struct JoinLogger;

impl Hooks for JoinLogger {
    fn dispatch<'a>(&'a self, ev: HookEvent<'a>) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            match ev {
                HookEvent::MemberJoin { guild, member } => {
                    println!(
                        "👤 {} ({}) entrou em {} ({})",
                        member.user.username, member.user.id, guild.name, guild.id
                    );
                }
                HookEvent::Ready { .. }
                | HookEvent::GuildAdd(_)
                | HookEvent::MemberLeave { .. }
                | HookEvent::Message(_) => {}
            }
        })
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("FLUXER_BOT_TOKEN").is_err() {
        return Err("FLUXER_BOT_TOKEN não definido".into());
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
