# Parity

Flush targets full parity with [`@fluxerjs`](https://fluxer.js.org/) 3.3.0 and
[Yuzuha](https://github.com/horizzon3507/yuzuha) 0.8.0. This file tracks what
matches today and what is still to come. Nothing under **Deferred** has been
dropped: each item is planned as its own follow-up release.

## @fluxerjs 3.3.0

| Area | Status |
|---|---|
| `Routes` (`@fluxerjs/types`) | All 92 route builders exist in `flush_rest::Routes` |
| REST methods | 121 typed methods on `RestClient`; 23 routes have no typed method yet (see below) and are reachable through `RestClient::raw` |
| Gateway dispatch events | All 80 names in `DISPATCH_EVENTS`; 47 decode into typed `Event` variants, the other 33 arrive as `Event::Unknown` with the raw payload |
| Entities | User, guild, member, role, channel (incl. threads, forum, media, announcement), message, embed, invite, webhook, emoji, sticker, audit log, voice state, application |
| CDN helpers (`Cdn.ts`) | Avatar, default avatar, banner, member avatar/banner, guild icon/banner/splash, emoji, sticker, with the SDK size ladder |
| Gateway lifecycle (`@fluxerjs/ws`) | Identify, resume, heartbeat, reconnect, close codes, presence update, member chunk requests; one connection, no sharding |
| Builders (`@fluxerjs/builders`) | `EmbedBuilder` with SDK limits; `MessageSend`, `WebhookExecute`, `StartThread` payloads |

### REST methods by resource

Messages, reactions, pins, invites, roles, members, bans, channels, DMs
(since 0.1.x); webhooks (bot- and token-authenticated, execute, edit/delete
message), emojis and stickers (single, bulk, clone, metadata), audit logs,
threads (start, list active/archived, search, members, archive/lock), forum
tags, permission overwrites, channel/role positions, member search, guild
edit, vanity URL, current-user guilds, leave guild, user profile,
application info, message search (since 0.2.0).

### Routes without a typed method yet

Deferred, reachable through `RestClient::raw`:

- Attachment uploads: `channel_attachments`, `channel_attachments_complete`
  (multipart/file sending is not implemented).
- Guild lifecycle: `guilds` (create), `guild_delete`,
  `guild_transfer_ownership`, `guild_discovery`, `guild_roles_hoist_positions`.
- Bulk and read state: `channel_bulk_delete_mine`, `guild_bulk_delete_mine`,
  `channel_messages_purge`, `channel_messages_ack`, `channel_pins_ack`,
  `channels_messages_bulk`, `preload_messages`, `preload_messages_alt`.
- Group DMs and user state: `channel_recipient`, `user_me_channel_pin`,
  `channel_thread_member_me_settings`, `check_username_tag`.
- Applications: `oauth2_application_bot`, `oauth2_application_bot_reset_token`.
- Other: `instance_discovery` (used internally by discovery), `stream_preview`.

### Dispatch events without a typed variant yet

Deferred; mostly user-account events a bot token does not receive:
`AUTH_SESSION_CHANGE`, `CALL_CREATE`, `CALL_UPDATE`, `CALL_DELETE`,
`CHANNEL_MEMBER_COUNTS_UPDATE`, `CHANNEL_PINS_ACK`, `ENTRANCE_SOUND_PLAY`,
`FAVORITE_MEME_CREATE`, `FAVORITE_MEME_UPDATE`, `FAVORITE_MEME_DELETE`,
`FORUM_UNREADS`, `GUILD_COUNTS_UPDATE`, `GUILD_MEMBER_LIST_UPDATE`,
`GUILD_SYNC`, `MESSAGE_ACK`, `MESSAGE_REACTION_ADD_MANY`, `PASSIVE_UPDATES`,
`PRESENCE_UPDATE_BULK`, `RECENT_MENTION_DELETE`, `RELATIONSHIP_ADD`,
`RELATIONSHIP_UPDATE`, `RELATIONSHIP_REMOVE`, `SAVED_MESSAGE_CREATE`,
`SAVED_MESSAGE_DELETE`, `SESSIONS_REPLACE`, `THREAD_MEMBER_LIST_UPDATE`,
`USER_CONNECTIONS_UPDATE`, `USER_GUILD_SETTINGS_UPDATE`, `USER_NOTE_UPDATE`,
`USER_PINNED_DMS_UPDATE`, `USER_SETTINGS_UPDATE`, `VOICE_STATE_ACK`,
`WEBAUTHN_CREDENTIALS_UPDATE`.

## Yuzuha 0.8.0

| Yuzuha feature | Flush |
|---|---|
| Prefix commands, aliases, cooldowns, per-guild prefix | `Bot`, `Command`, `Cooldowns`, `GuildStore` |
| Permission and hierarchy guards | `Command::required_permission`, `Moderation` |
| Moderation with cases and mod log | `with_mod_action`, `CaseStore`, `ModAction.log_channel` |
| Help | `Bot::enable_help()` |
| Event hooks | `Hooks`, including `HookEvent::Raw` for every gateway event |
| Typed arguments | `Args` (user, role, channel, integer, boolean, duration, rest) |
| Middleware | `Middleware`, `Guard`, `guild_only` |
| Collectors | `Collectors` (messages and reactions) |
| Scheduler | `Scheduler` (`after`, `every`, cancellation) |
| CLI: `init`, `templates`, `check`, `doctor`, `data`, `run`, `dev` | `flush` binary (`flush-cli`), see [CLI.md](CLI.md) |
| Config and `.env` loading | `ProjectConfig` (`flush.toml`), `bot_from_project`, `load_env` |

### Deferred

Each of these is a follow-up of its own:

1. **Function-mode scripting language** and its ~459 built-in functions.
2. **CLI leftovers**. The `flush` CLI covers scaffolding, config loading and
   the run/reload lifecycle ([CLI.md](CLI.md)). What is still missing waits
   on other items of this list: `functions`, `lint`, `test`, `editor` and
   `init --functions` (item 1); `data cleanup`, SQLite stores and the
   `community`, `economy`, `utility` and `multifunctional` templates (item 3);
   `run --shards` (item 4); plus the health server behind `--health-port`.
   `check` reports the config only: the command catalog lives inside the
   compiled bot.
3. **SQLite store** (Flush persists `GuildStore`/`CaseStore` as JSON only),
   plus Yuzuha's variable service on top of it.
4. **Sharding and cluster runtime** (Flush opens a single gateway connection).
5. **Voice runtime** (Flush decodes voice state/server events but opens no
   voice connection).
6. **Music / Lavalink** integration.
7. **Canvas / image** tooling.
8. **Attachment uploads** and the remaining typed REST methods and dispatch
   events listed above.
9. **Command hot reload** (`load`/`reload` of command directories) and
   cron-expression schedules (the scheduler has delays and intervals only).
