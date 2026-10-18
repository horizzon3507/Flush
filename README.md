# Flush — framework/lib em Rust para bots do Fluxer

Bots para [Fluxer](https://fluxer.app/) sobre `async-std`, com paridade prática
ao [`@fluxerjs/core`](https://fluxer.js.org/) 2.x. Espelha os fluxos reais dos
bots `jully-fluxer` (WS raw + `Bot <token>`) e `Sour Fluxer` (prefixo, reply,
moderação com Case#, config por servidor).

## Crates

| Crate | Papel |
|---|---|
| `flush-core` | Tipos: `Snowflake` (string decimal no wire), permissões (`1<<n`), usuários, guilds, canais, mensagens (`MessageSend::no_ping_text`), embeds (`EmbedBuilder` com limites 256/4096/25/6000), invites, presença, eventos tipados (`Event::decode`) |
| `flush-gateway` | Gateway WS v1: Hello op 10 → Identify op 2 → Heartbeat op 1 (~41s) → Dispatch op 0; Resume op 6 (`session_id`/`seq`, retenção 60s); Reconect op 7, Invalid Session op 9; close codes 4000–4012 |
| `flush-rest` | REST `/v1` sobre surf: discovery `GET /.well-known/fluxer` → `api_public` (fallback `https://api.fluxer.app`), auth `Bot <token>`, retry 429 via `Retry-After` |
| `flush-framework` | Roteador + features: prefixo por guild, aliases, dedup, cooldown, presença rotativa 45s, gate de permissão por comando, help auto-gerado, hooks de eventos, mod-log, `GuildStore`/`CaseStore` JSON atômico, `with_mod_action` |
| `flush` | Fachada: re-exporta os crates + exemplo `ping` |

## REST (/v1)

Rotas cobertas (espelho de `Routes` do `@fluxerjs/types`):

- **Usuários/gateway**: `/users/@me`, `/users/{id}`, `/gateway/bot`; DM via `POST /users/@me/channels` (`create_dm`).
- **Mensagens**: send/reply/edit/delete, lista, bulk-delete (1 id = DELETE, 2–100 = POST), typing.
- **Reactions**: `add`/`remove` (própria ou de membro, `MANAGE_MESSAGES`), `clear_reactions`, `reaction_users` — emoji escape `nome:id` / unicode via `ReactionEmoji::encoded()`.
- **Pins**: `pin_message` / `unpin_message` / `pinned_messages` (`/channels/{c}/messages/pins`).
- **Invites**: `channel_invites`, `create_invite` (`max_age_seconds`, `max_uses`, `temporary`, `unique`), `guild_invites`, `invite(code)`, `delete_invite`.
- **Roles**: lista (sem GET individual), `create_role` / `edit_role` / `delete_role` (`RolePatch`).
- **Membros**: get/list paginada (`after`)/all, `patch_member`, `set_nick` (None remove), timeout, add/remove role, kick.
- **Bans**: lista, `put ban` com `delete_message_days`/`ban_duration_seconds`, unban.
- **Canais**: get, `edit_channel`, `set_slowmode`, `create_channel` (`ChannelCreate`), `delete_channel`.
- Todos os métodos mutantes aceitam `audit: Option<&str>` → header `X-Audit-Log-Reason`.

## Framework

- `Command` object-safe: `name`, `description`, `aliases`, `usage`, `hidden`, `cooldown_ms`, **`required_permission`** (gate automático no router: checa executor *e* o bot, dono do servidor passa), `execute` com boxed future.
- `CommandCtx`: `reply_text` / `reply_no_ping` / `reply_embed`, **`target_member`** (menção/`id` via REST, 404 → `Ok(None)`).
- **`Bot::enable_help()`**: comando `help` embutido que lista os demais (nome, descrição, usage, aliases, permissão exigida) a partir de snapshot sincronizado no `run()`.
- **`Bot::hooks(impl Hooks)`**: `HookEvent` com `Ready`, `GuildAdd`, `MemberJoin`, `MemberLeave`, `Message`.
- **Mod-log**: `ModAction.log_channel` recebe o sucesso de moderação sem ping (erros engolidos), com canal vindo de `GuildStore.mod_log_channel_id` (`MOD_LOG_CHANNEL_ID`/env por guild).
- Escrita de resposta estilo Sour: `plain_error`, `info_block`, `mod_success`.

## Quickstart

```bash
FLUXER_BOT_TOKEN=... cargo run -p flush --example ping
```

```rust
use flush::{Bot, BotConfig, Command, CommandCtx, CommandError, CommandResult};
use std::future::Future;
use std::pin::Pin;

struct Ping;
impl Command for Ping {
    fn name(&self) -> &str { "ping" }
    fn description(&self) -> &str { "Latência" }
    fn execute<'a>(&'a self, ctx: CommandCtx<'a>)
        -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            ctx.reply_text("🏓 Pong!").await?;
            Ok(())
        })
    }
}

let mut bot = Bot::new(BotConfig::new(token))?.command(Ping);
bot.run().await?;
```

Env: `FLUXER_BOT_TOKEN` (obrigatório), `PREFIX` (default `!`),
`FLUSH_DATA_DIR` (default `./data`), `MOD_LOG_CHANNEL_ID`, `BOT_COLOR`.

## Contratos do wire (não quebrar)

- Snowflake = string decimal; `> i64::MAX` rejeita; `verity` via `>> 22 + epoch`.
- Enums (`ChannelType`, `MessageType`) = inteiros; desconhecido → `Unknown`, nunca erro.
- Listas aceitam `null` como vazio (Gateway manda `null`).
- Permissões = string decimal; ADMIN implica tudo; overwrite ordem: everyone → roles (união) → member.
- `GUILD_CREATE`/READY aninham guild em `properties` ou mandam flat — ambos decodificam.
- Reply = `POST /channels/{id}/messages` com
  `message_reference {channel_id, message_id, guild_id?}` +
  `allowed_mentions.replied_user: false` quando `ping: false`.
- Ban = `PUT /guilds/{g}/bans/{u}` (`reason`, `delete_message_days` 0–7);
  timeout = `PATCH` member (`communication_disabled_until`, `timeout_reason`);
  bulk-delete: 1 id = `DELETE`, 2–100 = `POST bulk-delete {message_ids}`.
- Sem GET de role individual: `GET /guilds/{g}/roles` + `find` pelo ID.
- Typing nunca falha comando (erro engolido, como Jully/Sour).

## Testes

```bash
cargo test --workspace   # 45 testes: wire tolerante, rotas, protocolo, router
cargo clippy --workspace --all-targets  # zero warnings
```

Sem bot de teste ligado: o exemplo `ping` exige token real e conecta no
gateway/REST de produção.
