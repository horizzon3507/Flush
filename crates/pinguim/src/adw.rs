use std::future::Future;
use std::pin::Pin;

use flush_core::Snowflake;
use flush_framework::{Command, CommandCtx, CommandResult};

const ROLE_ID: u64 = 1473381708310396982;
const ADD_DELAY_MS: u64 = 300;
const PROGRESS_EVERY: usize = 50;

pub struct Adw {
    pub owner_id: Option<Snowflake>,
}

impl Command for Adw {
    fn name(&self) -> &str {
        "adw"
    }
    fn description(&self) -> &str {
        "Dá o cargo ADW para todos os membros do servidor."
    }
    fn usage(&self) -> Option<&str> {
        Some(".adw")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let Some(owner) = self.owner_id else {
                ctx.reply_no_ping("O dono do bot não está configurado (BOT_OWNER_ID).")
                    .await?;
                return Ok(());
            };
            if ctx.message.author.id != owner {
                ctx.reply_no_ping("Só o dono do bot pode usar esse comando.")
                    .await?;
                return Ok(());
            }
            let Some(guild_id) = ctx.guild_id() else {
                ctx.reply_no_ping("Esse comando só funciona em servidor.")
                    .await?;
                return Ok(());
            };
            let role_id = Snowflake::new(ROLE_ID);
            if ctx
                .rest
                .role(guild_id, role_id)
                .await
                .ok()
                .flatten()
                .is_none()
            {
                ctx.reply_no_ping(
                    "Cargo não encontrado nesse servidor (confira se o bot enxerga o cargo e tem permissão de gerenciar cargos).",
                )
                .await?;
                return Ok(());
            }
            let members = match ctx.rest.all_members(guild_id).await {
                Ok(m) => m,
                Err(_) => {
                    ctx.reply_no_ping("Falha ao buscar a lista de membros.")
                        .await?;
                    return Ok(());
                }
            };
            if members.len() >= PROGRESS_EVERY {
                ctx.reply_text(format!(
                    "Dando o cargo para {} membros, pode demorar um pouco...",
                    members.len()
                ))
                .await?;
            }
            let mut granted = 0usize;
            let mut skipped = 0usize;
            let mut failed = 0usize;
            for m in &members {
                let fresh = ctx.rest.member(guild_id, m.user.id).await.ok();
                if fresh.as_ref().is_some_and(|f| f.roles.contains(&role_id)) {
                    skipped += 1;
                } else {
                    match ctx.rest.add_role(guild_id, m.user.id, role_id, None).await {
                        Ok(()) => granted += 1,
                        Err(_) => failed += 1,
                    }
                }
                let processed = granted + skipped + failed;
                if processed % PROGRESS_EVERY == 0 {
                    ctx.reply_text(format!(
                        "Progresso: {processed}/{} (concedidos {granted}, já tinham {skipped}, falharam {failed})",
                        members.len()
                    ))
                    .await?;
                }
                async_std::task::sleep(std::time::Duration::from_millis(ADD_DELAY_MS)).await;
            }
            let summary = format!(
                "concedidos {granted}, já tinham {skipped}, falharam {failed} (de {})",
                members.len()
            );
            ctx.reply_text(format!("Pronto: {summary}.")).await?;
            Ok(())
        })
    }
}
