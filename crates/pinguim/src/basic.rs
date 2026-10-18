use std::future::Future;
use std::pin::Pin;

use flush_core::Snowflake;
use flush_framework::{Command, CommandCtx, CommandResult};

pub struct Ping;

impl Command for Ping {
    fn name(&self) -> &str {
        "ping"
    }
    fn description(&self) -> &str {
        "Responde com pong."
    }
    fn usage(&self) -> Option<&str> {
        Some(".ping")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            ctx.reply_text("pong").await?;
            Ok(())
        })
    }
}

pub struct Info;

impl Command for Info {
    fn name(&self) -> &str {
        "info"
    }
    fn description(&self) -> &str {
        "Mostra informações do bot."
    }
    fn usage(&self) -> Option<&str> {
        Some(".info")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            ctx.reply_text(format!(
                "Sou o PinguimBot 🐧 — prefixo `{}`. Use `{}help` para ver os comandos.",
                ctx.prefix, ctx.prefix
            ))
            .await?;
            Ok(())
        })
    }
}

pub struct Help {
    pub entries: Vec<(&'static str, &'static str, &'static str, bool)>,
}

impl Help {
    pub fn entries() -> Vec<(&'static str, &'static str, &'static str, bool)> {
        vec![
            ("ping", "Responde com pong.", ".ping", false),
            ("info", "Mostra informações do bot.", ".info", false),
            (
                "dado",
                "Rola um dado de N lados (padrão: 6).",
                ".dado [lados]",
                false,
            ),
            ("moeda", "Joga uma moeda: cara ou coroa.", ".moeda", false),
            (
                "8ball",
                "Responde sua pergunta como uma bola 8 mágica.",
                ".8ball <pergunta>",
                false,
            ),
            (
                "userinfo",
                "Mostra informações de um usuário.",
                ".userinfo [@usuário]",
                false,
            ),
            (
                "serverinfo",
                "Mostra informações do servidor.",
                ".serverinfo",
                false,
            ),
            (
                "avatar",
                "Mostra o avatar de um usuário.",
                ".avatar [@usuário]",
                false,
            ),
            (
                "ban",
                "Bane um usuário do servidor. Requer Banir Membros.",
                ".ban @usuário [motivo]",
                false,
            ),
            (
                "kick",
                "Expulsa um usuário do servidor. Requer Expulsar Membros.",
                ".kick @usuário [motivo]",
                false,
            ),
            (
                "limpar",
                "Apaga N mensagens recentes do canal. Requer Gerenciar Mensagens.",
                ".limpar <1-100>",
                false,
            ),
            (
                "slowmode",
                "Define o modo lento do canal. Requer Gerenciar Canais.",
                ".slowmode <segundos|off> (ex.: .slowmode 10s, .slowmode 1m, .slowmode off)",
                false,
            ),
            (
                "adw",
                "Dá o cargo ADW para todos os membros do servidor.",
                ".adw",
                true,
            ),
            ("help", "Lista os comandos disponíveis.", ".help", false),
        ]
    }
}

impl Command for Help {
    fn name(&self) -> &str {
        "help"
    }
    fn description(&self) -> &str {
        "Lista os comandos disponíveis."
    }
    fn usage(&self) -> Option<&str> {
        Some(".help")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let lines: Vec<String> = self
                .entries
                .iter()
                .map(|(name, desc, usage, owner)| {
                    let tag = if *owner { " [só dono]" } else { "" };
                    format!(
                        "{prefix}{name}{tag}\n  {desc}\n  Uso: {usage}",
                        prefix = ctx.prefix
                    )
                })
                .collect();
            ctx.reply_text(format!("Comandos disponíveis:\n{}", lines.join("\n")))
                .await?;
            Ok(())
        })
    }
}

pub fn target_id(message: &flush_core::Message, args: &[String]) -> Snowflake {
    first_arg_target(args)
        .or_else(|| message.mentions.first().map(|u| u.id))
        .unwrap_or(message.author.id)
}

pub fn first_arg_target(args: &[String]) -> Option<Snowflake> {
    let raw = args.first()?;
    let id = flush_framework::parse_mention_or_id(raw)?;
    id.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::Help;

    #[test]
    fn help_lists_pinguim_commands() {
        let names: Vec<&str> = Help::entries().iter().map(|e| e.0).collect();
        for want in [
            "ping",
            "info",
            "dado",
            "moeda",
            "8ball",
            "userinfo",
            "serverinfo",
            "avatar",
            "ban",
            "kick",
            "limpar",
            "slowmode",
            "adw",
            "help",
        ] {
            assert!(names.contains(&want), "faltando {want}");
        }
    }
}
