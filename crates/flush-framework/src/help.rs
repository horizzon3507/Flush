//! Comando `help` embutido (auto-lido da lista de comandos do `Bot`).
//!
//! `Bot::enable_help()` registra o comando e `Bot::refresh_help()` sincroniza
//! o snapshot (nome, descrição, usage, aliases) antes do loop principal.

use std::sync::Mutex;

use crate::command::{info_block, plain_error, Command, CommandCtx, CommandResult};
use crate::moderation::PermissionAction;

#[derive(Debug, Clone)]
pub struct CommandInfo {
    pub name: String,
    pub description: String,
    pub usage: Option<String>,
    pub aliases: Vec<String>,
    pub hidden: bool,
    pub required_permission: Option<PermissionAction>,
}

pub struct Help {
    snapshot: Mutex<Vec<CommandInfo>>,
}

impl Help {
    pub fn new() -> Self {
        Self {
            snapshot: Mutex::new(Vec::new()),
        }
    }

    pub fn refresh(&self, commands: &[Box<dyn Command>]) {
        let list: Vec<CommandInfo> = commands
            .iter()
            .map(|c| CommandInfo {
                name: c.name().to_owned(),
                description: c.description().to_owned(),
                usage: c.usage().map(str::to_owned),
                aliases: c.aliases().to_vec(),
                hidden: c.hidden(),
                required_permission: c.required_permission(),
            })
            .collect();
        if let Ok(mut g) = self.snapshot.lock() {
            *g = list;
        }
    }

    fn detail(list: &[CommandInfo], query: &str) -> Option<String> {
        let lower = query.to_lowercase();
        let c = list.iter().find(|c| {
            !c.hidden && (c.name == lower || c.aliases.iter().any(|a| a.to_lowercase() == lower))
        })?;
        let mut lines = vec![format!("`{}`", c.description)];
        if let Some(u) = &c.usage {
            lines.push(format!("Uso: `{u}`"));
        }
        if !c.aliases.is_empty() {
            lines.push(format!("Aliases: `{}`", c.aliases.join(", ")));
        }
        if let Some(p) = c.required_permission {
            lines.push(format!("Requer permissão: **{}**", p.label()));
        }
        Some(info_block(
            format!("❓ {name}", name = c.name).as_str(),
            &lines,
        ))
    }

    fn overview(list: &[CommandInfo]) -> String {
        let mut visible: Vec<&CommandInfo> = list.iter().filter(|c| !c.hidden).collect();
        visible.sort_by(|a, b| a.name.cmp(&b.name));
        let lines: Vec<String> = visible
            .iter()
            .map(|c| {
                let aliases = if c.aliases.is_empty() {
                    String::new()
                } else {
                    format!(" (aliases: {})", c.aliases.join(", "))
                };
                format!("**{}**{} — {}", c.name, aliases, c.description)
            })
            .collect();
        if lines.is_empty() {
            return plain_error("Nenhum comando registrado.");
        }
        info_block("❓ Meus comandos", &lines)
    }
}

impl Command for Help {
    fn name(&self) -> &str {
        "help"
    }

    fn description(&self) -> &str {
        "Mostra a lista de comandos ou o detalhe de um: help [nome]"
    }

    fn usage(&self) -> Option<&str> {
        Some("help [nome]")
    }

    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let text = {
                let list = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
                if ctx.args.is_empty() {
                    Self::overview(&list)
                } else {
                    match Self::detail(&list, &ctx.args[0]) {
                        Some(t) => t,
                        None => plain_error(format!(
                            "Comando `{}` não encontrado. Use `{p}help`.",
                            ctx.args[0],
                            p = ctx.prefix
                        )),
                    }
                }
            };
            ctx.reply_no_ping(text).await?;
            Ok(())
        })
    }
}

impl Default for Help {
    fn default() -> Self {
        Self::new()
    }
}

/// [`Command`] que desacopla o `help` compartilhado entre o registro do
/// `Bot::enable_help()` e o snapshot sincronizado no `run()`.
pub struct HelpCmd(pub std::sync::Arc<Help>);

impl Command for HelpCmd {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn description(&self) -> &str {
        self.0.description()
    }
    fn usage(&self) -> Option<&str> {
        self.0.usage()
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = CommandResult> + Send + 'a>> {
        self.0.execute(ctx)
    }
}

impl std::fmt::Debug for Help {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Help").finish()
    }
}
