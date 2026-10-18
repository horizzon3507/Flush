use flush_core::Snowflake;
use flush_framework::{Bot, BotConfig};

mod adw;
mod basic;
mod dates;
mod fun;
mod modinfo;
mod rng;

use adw::Adw;
use basic::{Help, Info, Ping};
use fun::{Ball8, Dado, Moeda};
use modinfo::{
    Avatar, Ban, Caso, Convites, Kick, Limpar, Serverinfo, Slowmode, Unwarn, Userinfo, Warn,
};

const BOT_NAME: &str = "PinguimBot";

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn load_owner() -> Option<Snowflake> {
    env("BOT_OWNER_ID").and_then(|raw| raw.trim().parse().ok())
}

#[async_std::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token = env("FLUXER_TOKEN").ok_or(
        "Faltando a variável de ambiente FLUXER_TOKEN. Copie o .env.example para .env e configure o FLUXER_TOKEN.",
    )?;
    let owner = load_owner();
    let prefix = env("PREFIX").unwrap_or_else(|| ".".into());

    let names = [
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
        "warn",
        "unwarn",
        "caso",
        "convites",
        "adw",
        "help",
    ];
    println!("╭─ {BOT_NAME} ─────────────────────");
    println!("│ {BOT_NAME} (bot de Fluxer)");
    println!("│ Prefixo:       {prefix}");
    println!("│ Comandos ({}): {}", names.len(), names.join(", "));
    println!("│ FLUXER_TOKEN:  configurado");
    match owner {
        Some(_) => println!("│ BOT_OWNER_ID:  configurado"),
        None => println!("│ BOT_OWNER_ID:  não configurado — comandos do dono vão recusar"),
    }
    println!("╰──────────────────────────────────");

    let mut cfg = BotConfig::new(token);
    cfg.default_prefix = prefix;
    cfg.default_cooldown_ms = 0;
    let mut bot = Bot::new(cfg)?
        .command(Ping)
        .command(Info)
        .command(Dado)
        .command(Moeda)
        .command(Ball8)
        .command(Userinfo)
        .command(Serverinfo)
        .command(Avatar)
        .command(Ban)
        .command(Kick)
        .command(Limpar)
        .command(Slowmode)
        .command(Warn)
        .command(Unwarn)
        .command(Caso)
        .command(Convites)
        .command(Adw { owner_id: owner })
        .command(Help {
            entries: Help::entries(),
        });
    bot.run().await.map_err(|e| e.to_string())?;
    Ok(())
}
