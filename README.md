# Flush: a Rust framework/library for Fluxer bots

Bots for [Fluxer](https://fluxer.app/) on `async-std`, tracking
[`@fluxerjs`](https://fluxer.js.org/) 3.3 and
[Yuzuha](https://github.com/horizzon3507/yuzuha) 0.8.0. It mirrors the real
flows of the `jully-fluxer` (raw WS + `Bot <token>`) and `Sour Fluxer` (prefix,
reply, moderation with Case #, per-server config) bots.

What matches the two references, and what is still deferred, is tracked in
[docs/PARITY.md](docs/PARITY.md).

## Crates

| Crate | Role |
|---|---|
| `flush-core` | Types: `Snowflake` (decimal string on the wire), permissions (`1<<n`), users, guilds, channels and threads, messages (`MessageSend::no_ping_text`), embeds (`EmbedBuilder` with the 256/4096/25/6000 limits), invites, webhooks, emojis, stickers, audit logs, voice state, presence, CDN URLs, typed events (`Event::decode`) |
| `flush-gateway` | Gateway WS v1: Hello op 10 → Identify op 2 → Heartbeat op 1 (~41s) → Dispatch op 0; Resume op 6 (`session_id`/`seq`, 60s retention); Reconnect op 7, Invalid Session op 9; close codes 4000–4012 |
| `flush-rest` | REST `/v1` on surf: discovery `GET /.well-known/fluxer` → `api_public` (fallback `https://api.fluxer.app`), `Bot <token>` auth, 429 retry via `Retry-After` |
| `flush-framework` | Router + features: per-guild prefix, aliases, dedup, cooldown, typed arguments, middleware, collectors, scheduler, rotating presence every 45s, per-command permission gate, generated help, event hooks, mod log, atomic JSON `GuildStore`/`CaseStore`, `with_mod_action` |
| `flush-lib` | Facade: re-exports the crates + the `ping` example |

## REST (/v1)

`Routes` has every builder of `Routes` in `@fluxerjs/types` 3.3.0. Typed methods:

- **Users/gateway**: `/users/@me`, `/users/{id}`, `/gateway/bot`, `user_profile`, `edit_current_user`, `current_user_guilds`, `leave_guild`; DMs via `POST /users/@me/channels` (`create_dm`).
- **Messages**: send/reply/edit/delete, get, list, bulk-delete (1 id = DELETE, 2–100 = POST), typing, `search_messages`, `delete_attachment`.
- **Reactions**: `add`/`remove` (own or a member's, `MANAGE_MESSAGES`), `clear_reactions`, `clear_reaction`, `reaction_users`; emoji escaped as `name:id` / unicode via `ReactionEmoji::encoded()`.
- **Pins**: `pin_message` / `unpin_message` / `pinned_messages` (`/channels/{c}/messages/pins`).
- **Invites**: `channel_invites`, `create_invite` (`max_age_seconds`, `max_uses`, `temporary`, `unique`), `guild_invites`, `invite(code)`, `delete_invite`.
- **Roles**: list, `fetch_role`, `create_role` / `edit_role` / `delete_role` (`RolePatch`), `set_role_positions`.
- **Members**: get/paginated list (`after`)/all, `search_members`, `patch_member`, `set_nick` (None clears), timeout, add/remove role, kick.
- **Bans**: list, `put ban` with `delete_message_days`/`ban_duration_seconds`, unban.
- **Channels**: get, `edit_channel`, `patch_channel`, `set_slowmode`, `create_channel` (`ChannelCreate`), `delete_channel`, `set_channel_positions`, permission overwrites, `rtc_regions`.
- **Threads and forums**: `start_thread`, `start_thread_from_message`, `active_threads`, `archived_threads`, `search_threads`, thread members (list/join/leave/add/remove), `set_thread_state`, forum tags (`create_tag`/`edit_tag`/`delete_tag`).
- **Webhooks**: list (channel/guild), create, get/edit/delete with the bot token or the webhook token, `execute_webhook`, edit/delete webhook messages. Token-scoped calls never send the bot token.
- **Emojis and stickers**: list, get, create, bulk create, clone, rename/edit, delete, metadata.
- **Guild**: get, `edit_guild`, `audit_logs` (`AuditLogQuery`), `vanity_url` / `set_vanity_url`, `guild_webhooks`.
- **Applications**: `application`, `oauth2_application`.
- **Anything else**: `RestClient::raw(method, path, body, query, audit)`.
- Every mutating method takes `audit: Option<&str>` → `X-Audit-Log-Reason` header.

## Gateway events

`flush_core::DISPATCH_EVENTS` names all 80 dispatch events of the SDK. 47 decode
into typed `Event` variants (guilds, channels, threads, messages, reactions,
members, roles, bans, emojis, stickers, invites, webhooks, voice state, audit
log entries, presence, typing); the rest arrive as `Event::Unknown` with the
raw payload, and `Event::wire_name()` tells them apart.

## Framework

- Object-safe `Command`: `name`, `description`, `aliases`, `usage`, `hidden`, `cooldown_ms`, **`required_permission`** (automatic gate in the router: checks the executor *and* the bot, the server owner passes), `execute` with a boxed future.
- `CommandCtx`: `reply_text` / `reply_no_ping` / `reply_embed`, **`target_member`** (mention/`id` via REST, 404 → `Ok(None)`).
- **`Args`**: typed argument cursor: `next_user`, `next_role`, `next_channel`, `next_int`, `next_bool`, `next_duration` (`10m`, `1h30m`), `optional`, `rest`.
- **`Bot::middleware(impl Middleware)`**: runs before each command and can stop it with an optional reply; `Guard` wraps a closure, `guild_only` is built in.
- **`Bot::collectors()`**: await messages or reactions with a filter and a timeout (`messages_from`, `reactions_on`). Await them from a spawned task, not inside `execute`.
- **`Bot::scheduler()`**: `after(delay, job)` and `every(period, job)` with cancellable handles.
- **`Bot::enable_help()`**: built-in `help` command listing the others (name, description, usage, aliases, required permission) from a snapshot synced in `run()`.
- **`Bot::hooks(impl Hooks)`**: `HookEvent` with `Ready`, `GuildAdd`, `MemberJoin`, `MemberLeave`, `Message` and `Raw` (every gateway event).
- **Mod log**: `ModAction.log_channel` receives each moderation success without a ping (errors swallowed), with the channel coming from `GuildStore.mod_log_channel_id` (`MOD_LOG_CHANNEL_ID`/env per guild).
- **`StoreLock`**: `GuildStore` and `CaseStore` hold a shared advisory lock on `<store>.lock` while they are open. A tool that replaces a store file calls `StoreLock::try_exclusive` first and gets `None` while a bot has the store open.
- **`VariableService`**: JSON variables in `global`, `guild` and `user` scopes with TTL (`set_with_ttl`, `expire`, `ttl`), `update`/`increment`, prefix queries with a cursor and atomic `transaction`s; runs over `MemoryVariableStore` or `SqliteStore`.
- **`SqliteStore`** (feature `sqlite`, on by default; bundles SQLite): one WAL database for variables, cases (`add_case`, `cases_for_user`, `active_warns`, `revert_warn`) and guild configuration, plus `import_guilds_json`/`import_cases_json` to move off the JSON stores. Build with `default-features = false` to leave SQLite out.
- Sour-style reply writers: `plain_error`, `info_block`, `mod_success`.

## Quickstart

```bash
FLUXER_BOT_TOKEN=... cargo run -p flush-lib --example ping
```

```rust
use flush::{Bot, BotConfig, Command, CommandCtx, CommandError, CommandResult};
use std::future::Future;
use std::pin::Pin;

struct Ping;
impl Command for Ping {
    fn name(&self) -> &str { "ping" }
    fn description(&self) -> &str { "Latency" }
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

Env: `FLUXER_BOT_TOKEN` (required), `PREFIX` (default `!`),
`FLUSH_DATA_DIR` (default `./data`), `MOD_LOG_CHANNEL_ID`, `BOT_COLOR`.

## Wire contracts (do not break)

- Snowflake = decimal string; `> i64::MAX` is rejected; timestamp via `>> 22 + epoch`.
- Enums (`ChannelType`, `MessageType`) = integers; unknown → `Unknown`, never an error.
- Lists accept `null` as empty (the Gateway sends `null`).
- Permissions = decimal string; ADMIN implies everything; overwrite order: everyone → roles (union) → member.
- `GUILD_CREATE`/READY nest the guild under `properties` or send it flat; both decode.
- Reply = `POST /channels/{id}/messages` with
  `message_reference {channel_id, message_id, guild_id?}` +
  `allowed_mentions.replied_user: false` when `ping: false`.
- Ban = `PUT /guilds/{g}/bans/{u}` (`reason`, `delete_message_days` 0–7);
  timeout = `PATCH` member (`communication_disabled_until`, `timeout_reason`);
  bulk-delete: 1 id = `DELETE`, 2–100 = `POST bulk-delete {message_ids}`.
- Typing never fails a command (error swallowed, as in Jully/Sour).

## Tests

```bash
cargo test --workspace   # 57 tests: tolerant wire, routes, protocol, framework
cargo clippy --workspace --all-targets  # zero warnings
```

There is no test bot wired up: the `ping` example needs a real token and
connects to the production gateway/REST.
