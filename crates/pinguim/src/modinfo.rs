use std::future::Future;
use std::pin::Pin;

use flush_core::{Guild, Snowflake};
use flush_framework::{
    with_mod_action, CaseStore, CaseType, Command, CommandCtx, CommandError, CommandResult,
    GuildStore, ModAction, Moderation, PermissionAction,
};
use flush_rest::BanOptions;

use crate::basic::{first_arg_target, target_id};
use crate::dates::{from_snowflake, short_opt};

async fn guild_id_or_reply(ctx: &CommandCtx<'_>) -> Result<Option<Snowflake>, CommandError> {
    if let Some(g) = ctx.guild_id() {
        return Ok(Some(g));
    }
    ctx.reply_no_ping("Esse comando só funciona em servidor.")
        .await?;
    Ok(None)
}

async fn guild_or_reply(
    ctx: &CommandCtx<'_>,
    guild_id: Snowflake,
    missing: &str,
) -> Result<Option<Guild>, CommandError> {
    if let Some(g) = ctx.guild.clone() {
        return Ok(Some(g));
    }
    match ctx.rest.guild(guild_id).await {
        Ok(g) => Ok(Some(g)),
        Err(_) => {
            ctx.reply_no_ping(missing).await?;
            Ok(None)
        }
    }
}

async fn allowed_or_reply(
    ctx: &CommandCtx<'_>,
    guild: &Guild,
    action: PermissionAction,
) -> Result<bool, CommandError> {
    let member = match ctx.member.clone() {
        Some(m) => Some(m),
        None => ctx.rest.member(guild.id, ctx.message.author.id).await.ok(),
    };
    let Some(m) = member else {
        ctx.reply_no_ping(format!(
            "Você não tem permissão para isso (precisa de {}).",
            action.label()
        ))
        .await?;
        return Ok(false);
    };
    let is_owner = guild.owner_id == m.user.id;
    if Moderation::check_member(guild, Some(&m), action, ctx.channel.as_ref(), is_owner).is_some() {
        ctx.reply_no_ping(format!(
            "Você não tem permissão para isso (precisa de {}).",
            action.label()
        ))
        .await?;
        return Ok(false);
    }
    Ok(true)
}

async fn target_or_reply(
    ctx: &CommandCtx<'_>,
    usage: &str,
) -> Result<Option<(Snowflake, Option<String>)>, CommandError> {
    let id = first_arg_target(&ctx.args).or_else(|| ctx.message.mentions.first().map(|u| u.id));
    let Some(id) = id else {
        ctx.reply_no_ping(format!("Marque um usuário. Uso: {usage}"))
            .await?;
        return Ok(None);
    };
    let joined = ctx
        .args
        .iter()
        .skip(1)
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = joined.trim().to_owned();
    Ok(Some((
        id,
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        },
    )))
}

async fn target_name(ctx: &CommandCtx<'_>, id: Snowflake) -> String {
    let guild_id = match ctx.guild_id() {
        Some(g) => g,
        None => return format!("<@{id}>"),
    };
    match ctx.rest.member(guild_id, id).await {
        Ok(m) => m.display_name().to_owned(),
        Err(_) => format!("<@{id}>"),
    }
}

pub struct Ban;

impl Command for Ban {
    fn name(&self) -> &str {
        "ban"
    }
    fn description(&self) -> &str {
        "Bane um usuário do servidor. Requer Banir Membros."
    }
    fn usage(&self) -> Option<&str> {
        Some(".ban @usuário [motivo]")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let Some(guild_id) = guild_id_or_reply(&ctx).await? else {
                return Ok(());
            };
            let Some(guild) =
                guild_or_reply(&ctx, guild_id, "Esse comando só funciona em servidor.").await?
            else {
                return Ok(());
            };
            if !allowed_or_reply(&ctx, &guild, PermissionAction::Ban).await? {
                return Ok(());
            }
            let Some((target, reason)) = target_or_reply(&ctx, ".ban @usuário [motivo]").await?
            else {
                return Ok(());
            };
            let opts = BanOptions {
                reason: reason.clone(),
                ..Default::default()
            };
            if ctx.rest.ban(guild_id, target, &opts, None).await.is_err() {
                ctx.reply_no_ping(
                    "Não consegui banir (confira minha permissão e a hierarquia de cargos).",
                )
                .await?;
                return Ok(());
            }
            let display = target_name(&ctx, target).await;
            let suffix = reason.map(|r| format!(" Motivo: {r}.")).unwrap_or_default();
            ctx.reply_text(format!("🔨 {display} foi banido.{suffix}"))
                .await?;
            Ok(())
        })
    }
}

pub struct Kick;

impl Command for Kick {
    fn name(&self) -> &str {
        "kick"
    }
    fn description(&self) -> &str {
        "Expulsa um usuário do servidor. Requer Expulsar Membros."
    }
    fn usage(&self) -> Option<&str> {
        Some(".kick @usuário [motivo]")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let Some(guild_id) = guild_id_or_reply(&ctx).await? else {
                return Ok(());
            };
            let Some(guild) =
                guild_or_reply(&ctx, guild_id, "Esse comando só funciona em servidor.").await?
            else {
                return Ok(());
            };
            if !allowed_or_reply(&ctx, &guild, PermissionAction::Kick).await? {
                return Ok(());
            }
            let Some((target, reason)) = target_or_reply(&ctx, ".kick @usuário [motivo]").await?
            else {
                return Ok(());
            };
            if ctx
                .rest
                .kick(guild_id, target, reason.as_deref())
                .await
                .is_err()
            {
                ctx.reply_no_ping(
                    "Não consegui expulsar (confira minha permissão e a hierarquia de cargos).",
                )
                .await?;
                return Ok(());
            }
            let display = target_name(&ctx, target).await;
            let suffix = reason.map(|r| format!(" Motivo: {r}.")).unwrap_or_default();
            ctx.reply_text(format!("👢 {display} foi expulso.{suffix}"))
                .await?;
            Ok(())
        })
    }
}

pub struct Limpar;

impl Command for Limpar {
    fn name(&self) -> &str {
        "limpar"
    }
    fn description(&self) -> &str {
        "Apaga N mensagens recentes do canal. Requer Gerenciar Mensagens."
    }
    fn usage(&self) -> Option<&str> {
        Some(".limpar <1-100>")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            if guild_id_or_reply(&ctx).await?.is_none() {
                return Ok(());
            }
            let guild_id = ctx.guild_id().expect("guild confirmada acima");
            let Some(guild) =
                guild_or_reply(&ctx, guild_id, "Esse comando só funciona em servidor.").await?
            else {
                return Ok(());
            };
            if !allowed_or_reply(&ctx, &guild, PermissionAction::Messages).await? {
                return Ok(());
            }
            let count: u32 = match ctx.args.first().map(|s| s.parse::<i64>()) {
                Some(Ok(v)) if (1..=100).contains(&v) => v as u32,
                _ => {
                    ctx.reply_no_ping("Informe um número de 1 a 100. Ex.: `.limpar 10`.")
                        .await?;
                    return Ok(());
                }
            };
            match ctx.rest.bulk_delete_recent(ctx.channel_id(), count).await {
                Ok(ids) => {
                    ctx.reply_text(format!("🧹 Apaguei {} mensagem(ns).", ids.len()))
                        .await?;
                    Ok(())
                }
                Err(_) => {
                    ctx.reply_no_ping(
                        "Não consegui apagar as mensagens (confira minha permissão).",
                    )
                    .await?;
                    Ok(())
                }
            }
        })
    }
}

pub struct Slowmode;

impl Command for Slowmode {
    fn name(&self) -> &str {
        "slowmode"
    }
    fn description(&self) -> &str {
        "Define o modo lento do canal. Requer Gerenciar Canais."
    }
    fn usage(&self) -> Option<&str> {
        Some(".slowmode <segundos|off>")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            if guild_id_or_reply(&ctx).await?.is_none() {
                return Ok(());
            }
            let guild_id = ctx.guild_id().expect("guild confirmada acima");
            let Some(guild) =
                guild_or_reply(&ctx, guild_id, "Esse comando só funciona em servidor.").await?
            else {
                return Ok(());
            };
            if !allowed_or_reply(&ctx, &guild, PermissionAction::Channel).await? {
                return Ok(());
            }
            let Some(raw) = ctx.args.first().cloned() else {
                ctx.reply_no_ping(
                    "Informe a duração. Ex.: `.slowmode 10s`, `.slowmode 1m`, `.slowmode 1h` ou `.slowmode off`.",
                )
                .await?;
                return Ok(());
            };
            let invalid = || {
                format!(
                    "Duração inválida. Use 0–{} segundos (ex.: `5`, `10s`, `1m`, `1h`) ou `off`.",
                    Slowmode::MAX_SECONDS
                )
            };
            let Some(seconds) = parse_duration(&raw) else {
                ctx.reply_no_ping(invalid()).await?;
                return Ok(());
            };
            if seconds > Slowmode::MAX_SECONDS {
                ctx.reply_no_ping(invalid()).await?;
                return Ok(());
            }
            if ctx
                .rest
                .set_slowmode(ctx.channel_id(), seconds, None)
                .await
                .is_err()
            {
                ctx.reply_no_ping("Não consegui definir o modo lento (confira minha permissão).")
                    .await?;
                return Ok(());
            }
            if seconds == 0 {
                ctx.reply_text("⏱️ Modo lento desativado.").await?;
            } else {
                ctx.reply_text(format!(
                    "⏱️ Modo lento ativado: {} ({seconds} segundos).",
                    format_seconds(seconds)
                ))
                .await?;
            }
            Ok(())
        })
    }
}

impl Slowmode {
    pub const MAX_SECONDS: u64 = 21600;
}

pub fn parse_duration(raw: &str) -> Option<u64> {
    let t = raw.trim().to_lowercase();
    if t == "off" || t == "0" {
        return Some(0);
    }
    let split = t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    if num.is_empty() || num.len() > 6 {
        return None;
    }
    let value: u64 = num.parse().ok()?;
    let unit = unit.trim();
    if unit.is_empty() || unit == "s" {
        return Some(value);
    }
    for suffix in ["segundos", "segundo", "segs", "seg"] {
        if unit == suffix {
            return Some(value);
        }
    }
    for suffix in ["minutos", "minuto", "mins", "min", "m"] {
        if unit == suffix {
            return value.checked_mul(60);
        }
    }
    for suffix in ["horas", "hora", "hs", "h"] {
        if unit == suffix {
            return value.checked_mul(3600);
        }
    }
    None
}

pub fn format_seconds(total: u64) -> String {
    if total >= 3600 {
        let h = total / 3600;
        let m = (total % 3600) / 60;
        return if m > 0 {
            format!("{h}h{m}min")
        } else {
            format!("{h}h")
        };
    }
    if total >= 60 {
        let m = total / 60;
        let s = total % 60;
        return if s > 0 {
            format!("{m}min{s}s")
        } else {
            format!("{m}min")
        };
    }
    format!("{total}s")
}

pub struct Userinfo;

impl Command for Userinfo {
    fn name(&self) -> &str {
        "userinfo"
    }
    fn description(&self) -> &str {
        "Mostra informações de um usuário."
    }
    fn usage(&self) -> Option<&str> {
        Some(".userinfo [@usuário]")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let target = target_id(ctx.message, &ctx.args);
            let mut line = String::new();
            let mut name: Option<String> = None;
            if let Some(guild_id) = ctx.guild_id() {
                match ctx.rest.member(guild_id, target).await {
                    Ok(m) => {
                        let joined = short_opt(m.joined_at.as_deref());
                        let nick = m.nick.as_deref().unwrap_or("—");
                        line = format!("\n🕰️ Entrou no servidor em: {joined}\n🏷️ Apelido: {nick}");
                        name = Some(m.display_name().to_owned());
                    }
                    Err(_) => line = "\n(usuário fora do servidor)".into(),
                }
            }
            if name.is_none() {
                if target == ctx.message.author.id {
                    let a = &ctx.message.author;
                    name = Some(a.global_name.clone().unwrap_or_else(|| a.username.clone()));
                } else if let Ok(u) = ctx.rest.user(target).await {
                    name = Some(u.display_name().to_owned());
                }
            }
            let created = from_snowflake(target);
            let name = name.unwrap_or_else(|| format!("<@{target}>"));
            ctx.reply_text(format!(
                "👤 **{name}**\n🆔 ID: `{target}`\n📅 Conta criada em: {created}{line}"
            ))
            .await?;
            Ok(())
        })
    }
}

pub struct Serverinfo;

impl Command for Serverinfo {
    fn name(&self) -> &str {
        "serverinfo"
    }
    fn description(&self) -> &str {
        "Mostra informações do servidor."
    }
    fn usage(&self) -> Option<&str> {
        Some(".serverinfo")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let Some(guild_id) = guild_id_or_reply(&ctx).await? else {
                return Ok(());
            };
            let Some(guild) =
                guild_or_reply(&ctx, guild_id, "Não consegui resolver esse servidor.").await?
            else {
                return Ok(());
            };
            let roles = ctx.rest.roles(guild_id).await.unwrap_or_default();
            let role_count = if roles.is_empty() {
                guild.roles.len()
            } else {
                roles.len()
            };
            let channels = ctx
                .rest
                .guild_channels(guild_id)
                .await
                .map(|c| c.len().to_string())
                .unwrap_or_else(|_| "?".into());
            let members = guild
                .member_count
                .map(|n| n.to_string())
                .unwrap_or_else(|| "?".into());
            let created = from_snowflake(guild.id);
            ctx.reply_text(format!(
                "🏰 **{name}**\n🆔 ID: `{id}`\n👥 Membros: {members}\n🎭 Cargos: {role_count}\n📺 Canais: {channels}\n📅 Criado em: {created}",
                name = guild.name,
                id = guild.id,
            ))
            .await?;
            Ok(())
        })
    }
}

pub struct Avatar;

impl Command for Avatar {
    fn name(&self) -> &str {
        "avatar"
    }
    fn description(&self) -> &str {
        "Mostra o avatar de um usuário."
    }
    fn usage(&self) -> Option<&str> {
        Some(".avatar [@usuário]")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let target = target_id(ctx.message, &ctx.args);
            let (name, hash) = if target == ctx.message.author.id {
                let a = &ctx.message.author;
                (
                    a.global_name.clone().unwrap_or_else(|| a.username.clone()),
                    a.avatar.clone(),
                )
            } else if let Some(guild_id) = ctx.guild_id() {
                match ctx.rest.member(guild_id, target).await {
                    Ok(m) => (m.display_name().to_owned(), m.user.avatar.clone()),
                    Err(_) => match ctx.rest.user(target).await {
                        Ok(u) => (u.display_name().to_owned(), u.avatar.clone()),
                        Err(_) => (format!("<@{target}>"), None),
                    },
                }
            } else {
                match ctx.rest.user(target).await {
                    Ok(u) => (u.display_name().to_owned(), u.avatar.clone()),
                    Err(_) => (format!("<@{target}>"), None),
                }
            };
            let url = flush_core::display_avatar_url(target, hash.as_deref(), Some(512));
            ctx.reply_text(format!("🖼️ Avatar de **{name}**:\n{url}"))
                .await?;
            Ok(())
        })
    }
}

pub struct Warn;

impl Command for Warn {
    fn name(&self) -> &str {
        "warn"
    }
    fn description(&self) -> &str {
        "Dá um warning a um membro (fica no histórico de casos)."
    }
    fn usage(&self) -> Option<&str> {
        Some(".warn @usuário [motivo]")
    }
    fn required_permission(&self) -> Option<PermissionAction> {
        Some(PermissionAction::Moderate)
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let Some(guild_id) = guild_id_or_reply(&ctx).await? else {
                return Ok(());
            };
            let Some((target, reason)) = target_or_reply(&ctx, ".warn @usuário [motivo]").await?
            else {
                return Ok(());
            };
            let reason = reason.unwrap_or_else(|| "sem motivo".into());
            let display = target_name(&ctx, target).await;
            let mut store = CaseStore::with_default_path();
            let case = store.add(
                &guild_id.to_string(),
                CaseType::Warn,
                &target.to_string(),
                &ctx.message.author.id.to_string(),
                &reason,
            );
            let guild_log = GuildStore::with_default_path()
                .get(&guild_id.to_string())
                .mod_log_channel_id
                .and_then(|s| s.parse::<Snowflake>().ok());
            with_mod_action(ModAction {
                ctx: &ctx,
                executor: None, // o gate de `required_permission` já checou
                action: PermissionAction::Moderate,
                target: None,
                reason: &reason,
                success_icon: "⚠️",
                success_description: &format!("{display} recebeu um warning."),
                case_id: case.id,
                extra: Some(&format!(
                    "Total: {} ativos",
                    store.active_warns(&guild_id.to_string(), &target.to_string())
                )),
                log_channel: guild_log,
                run: || async move { Ok::<(), String>(()) },
            })
            .await?;
            Ok(())
        })
    }
}

pub struct Unwarn;

impl Command for Unwarn {
    fn name(&self) -> &str {
        "unwarn"
    }
    fn description(&self) -> &str {
        "Remove um warning: unwarn <@usuário> <case#>"
    }
    fn usage(&self) -> Option<&str> {
        Some(".unwarn @usuário <case#>")
    }
    fn required_permission(&self) -> Option<PermissionAction> {
        Some(PermissionAction::Moderate)
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let Some(guild_id) = guild_id_or_reply(&ctx).await? else {
                return Ok(());
            };
            let Some((target, extra)) = target_or_reply(&ctx, ".unwarn @usuário <case#>").await?
            else {
                return Ok(());
            };
            let Some(case_raw) = extra else {
                ctx.reply_no_ping("❌ Use: `.unwarn @usuário <case#>`")
                    .await?;
                return Ok(());
            };
            let Ok(case_id) = case_raw.trim().trim_start_matches('#').parse::<u64>() else {
                ctx.reply_no_ping("❌ Numero de case inválido.").await?;
                return Ok(());
            };
            let mut store = CaseStore::with_default_path();
            match store.revert_warn(&guild_id.to_string(), &target.to_string(), case_id) {
                Some(_) => {
                    ctx.reply_text(format!(
                        "⭕ Warning do case `#{case_id}` removido de <@{target}>."
                    ))
                    .await?;
                }
                None => {
                    ctx.reply_no_ping(format!("❌ Case `#{case_id}` não é um warning ativo."))
                        .await?;
                }
            }
            Ok(())
        })
    }
}

pub struct Caso;

impl Command for Caso {
    fn name(&self) -> &str {
        "caso"
    }
    fn description(&self) -> &str {
        "Mostra um caso ou o histórico do membro: caso [case#|@usuário]"
    }
    fn usage(&self) -> Option<&str> {
        Some(".caso [@usuário|#case]")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let Some(guild_id) = guild_id_or_reply(&ctx).await? else {
                return Ok(());
            };
            let mut store = CaseStore::with_default_path();
            if let Some(first) = ctx.args.first() {
                if let Ok(case_id) = first.trim().trim_start_matches('#').parse::<u64>() {
                    match store.get(&guild_id.to_string(), case_id) {
                        Some(c) => {
                            let status = if c.reverted { " (revertido)" } else { "" };
                            ctx.reply_text(format!(
                                "🗂️ Case `#{}` — {:?}\n-# ╰› Alvo: <@{}>\n-# ╰› Por: <@{}>\n-# ╰› Motivo: {}{status}",
                                c.id, c.kind, c.target_id, c.moderator_id, c.reason
                            ))
                            .await?;
                        }
                        None => {
                            ctx.reply_no_ping(format!("❌ Case `#{case_id}` não existe."))
                                .await?;
                        }
                    }
                    return Ok(());
                }
            }
            let target = target_id(ctx.message, &ctx.args);
            let cases = store.for_user(&guild_id.to_string(), &target.to_string(), 10);
            if cases.is_empty() {
                ctx.reply_no_ping("Sem casos neste servidor.").await?;
                return Ok(());
            }
            let lines: Vec<String> = cases
                .iter()
                .map(|c| {
                    format!(
                        "`#{}` {:?} — {}{}",
                        c.id,
                        c.kind,
                        if c.reason.is_empty() {
                            "sem motivo"
                        } else {
                            &c.reason
                        },
                        if c.reverted { " (revertido)" } else { "" }
                    )
                })
                .collect();
            let joined = lines.join("\n");
            ctx.reply_text(format!("🗂️ Casos de <@{target}>:\n{joined}"))
                .await?;
            Ok(())
        })
    }
}

pub struct Convites;

impl Command for Convites {
    fn name(&self) -> &str {
        "convites"
    }
    fn description(&self) -> &str {
        "Lista os convites ativos do servidor. Requer Gerenciar Servidor."
    }
    fn usage(&self) -> Option<&str> {
        Some(".convites")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let Some(guild_id) = guild_id_or_reply(&ctx).await? else {
                return Ok(());
            };
            let invites = ctx.rest.guild_invites(guild_id).await.unwrap_or_default();
            if invites.is_empty() {
                ctx.reply_no_ping("Nenhum convite ativo.").await?;
                return Ok(());
            }
            let lines: Vec<String> = invites
                .iter()
                .take(10)
                .map(|i| {
                    let inviter = i
                        .inviter
                        .as_ref()
                        .map(|u| format!("<@{}>", u.id))
                        .unwrap_or_else(|| "?".into());
                    format!(
                        "`{code}` — {uses} usos · por {inviter}",
                        code = i.code,
                        uses = i.uses
                    )
                })
                .collect();
            let joined = lines.join("\n");
            ctx.reply_text(format!("🔗 Convites ativos:\n{joined}"))
                .await?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{format_seconds, parse_duration, Slowmode};

    #[test]
    fn durations_match_pinguim() {
        assert_eq!(parse_duration("off"), Some(0));
        assert_eq!(parse_duration("0"), Some(0));
        assert_eq!(parse_duration("10"), Some(10));
        assert_eq!(parse_duration("10s"), Some(10));
        assert_eq!(parse_duration("1m"), Some(60));
        assert_eq!(parse_duration("1h"), Some(3600));
        assert_eq!(parse_duration("2min"), Some(120));
        assert_eq!(parse_duration("3hs"), Some(10800));
        assert_eq!(parse_duration("abc"), None);
        assert_eq!(parse_duration(""), None);
        assert_eq!(Slowmode::MAX_SECONDS, 21600);
    }

    #[test]
    fn formats_match_pinguim() {
        assert_eq!(format_seconds(0), "0s");
        assert_eq!(format_seconds(45), "45s");
        assert_eq!(format_seconds(60), "1min");
        assert_eq!(format_seconds(90), "1min30s");
        assert_eq!(format_seconds(3600), "1h");
        assert_eq!(format_seconds(3900), "1h5min");
    }
}
