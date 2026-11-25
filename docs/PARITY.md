# Parity

Flush targets full parity with [`@fluxerjs`](https://fluxer.js.org/) 3.3.0 and
[Yuzuha](https://github.com/horizzon3507/yuzuha) 0.8.0. This file tracks what
matches today and what is still to come. Nothing under **Deferred** has been
dropped: each item is planned as its own follow-up release.

## @fluxerjs 3.3.0

| Area | Status |
|---|---|
| `Routes` (`@fluxerjs/types`) | All 92 route builders exist in `flush_rest::Routes` |
| REST methods | 153 typed methods on `RestClient`, covering all 92 routes; `RestClient::raw` remains as an escape hatch |
| Gateway dispatch events | All 80 names in `DISPATCH_EVENTS` decode into typed `Event` variants; a payload that does not match its type arrives as `Event::Unknown` with the raw payload |
| Attachments | Multipart sending (`send_message_with_files`) and the presigned upload plan (`upload_attachments`, single- and multipart) |
| Entities | User, guild, member, role, channel (incl. threads, forum, media, announcement), message, embed, invite, webhook, emoji, sticker, audit log, voice state, application |
| CDN helpers (`Cdn.ts`) | Avatar, default avatar, banner, member avatar/banner, guild icon/banner/splash, emoji, sticker, with the SDK size ladder |
| Gateway lifecycle (`@fluxerjs/ws`) | Identify, resume, heartbeat, reconnect, close codes, presence update, member chunk requests; `ShardManager` runs one connection per shard (`shard_id_for_guild`, `ShardPlan`, per-shard status, reconnect) |
| Builders (`@fluxerjs/builders`) | `EmbedBuilder` with SDK limits; `MessageSend`, `WebhookExecute`, `StartThread` payloads |

### REST methods by resource

Messages, reactions, pins, invites, roles, members, bans, channels, DMs
(since 0.1.x); webhooks (bot- and token-authenticated, execute, edit/delete
message), emojis and stickers (single, bulk, clone, metadata), audit logs,
threads (start, list active/archived, search, members, archive/lock), forum
tags, permission overwrites, channel/role positions, member search, guild
edit, vanity URL, current-user guilds, leave guild, user profile,
application info, message search (since 0.2.0).

Since 0.2.x, in `flush_rest::account` and `flush_rest::uploads`:

- Attachment uploads: `send_message_with_files` (multipart `payload_json` +
  `files[n]`), `request_attachment_uploads`, `complete_attachment_uploads`
  and `upload_attachments`, which runs the whole presigned plan.
- Guild lifecycle: create, delete, transfer ownership, discovery
  status/apply/edit/withdraw, role hoist positions (set and reset).
- Bulk and read state: bulk-delete own messages (channel and guild), purge,
  clear read state, acknowledge pins, bulk message fetch, both preload routes.
- Group DMs and user state: add/remove recipient, pin/unpin DM, thread member
  settings, username tag check.
- Applications: edit bot user, reset bot token.
- Other: full instance discovery document, stream preview (get and upload).

Notes on these:

- Many are session-token helpers in the SDK and the API rejects them for bot
  tokens; they are typed for parity.
- `@fluxerjs/core` ships no caller for `create_guild`, `check_username_tag`,
  `edit_application_bot`, `reset_application_bot_token` and the stream
  preview routes, so their bodies and query parameters are not confirmed by
  the SDK. They take or return loosely typed JSON where the shape is unknown.
- Upload and account routes are tested against a loopback HTTP server, not
  the live API.

### Dispatch events

The session, read-state and bulk events (calls, favorite memes, relationships,
saved messages, user settings, member list updates, counts, `VOICE_STATE_ACK`
and the rest) have payload types in `flush_core::gateway_events`. Where the
SDK types a payload as an open map, known fields are typed and the remainder
is kept in `extra`. Most are user-account events a bot token does not receive.

## Yuzuha 0.8.0

| Yuzuha feature | Flush |
|---|---|
| Prefix commands, aliases, cooldowns, per-guild prefix | `Bot`, `Command`, `Cooldowns`, `GuildStore` |
| Permission and hierarchy guards | `Command::required_permission`, `Moderation` |
| Moderation with cases and mod log | `with_mod_action`, `CaseStore`, `ModAction.log_channel` |
| Store in use by a running bot is detectable | `StoreLock` (shared while `GuildStore`/`CaseStore` are open, `try_exclusive` for whole-file replacement) |
| Help | `Bot::enable_help()` |
| Event hooks | `Hooks`, including `HookEvent::Raw` for every gateway event |
| Typed arguments | `Args` (user, role, channel, integer, boolean, duration, rest) |
| Middleware | `Middleware`, `Guard`, `guild_only` |
| Collectors | `Collectors` (messages and reactions) |
| Scheduler | `Scheduler` (`after`, `every`, cancellation, paused on shard processes that should not run jobs) |
| CLI: `init`, `templates`, `check`, `doctor`, `data`, `run`, `dev` | `flush` binary (`flush-cli`), see [CLI.md](CLI.md) |
| Config and `.env` loading | `ProjectConfig` (`flush.toml`), `bot_from_project`, `load_env` |
| SQLite store (WAL, `BEGIN IMMEDIATE`, normalized tables, private file mode, schema guard) | `SqliteStore` (feature `sqlite`, on by default): variables, cases, guild configuration, import from the JSON stores |
| Variable service: `global`/`guild`/`user` scopes, TTL, `update`, `expire`, `ttl`, prefix queries with a cursor, atomic transactions, data operations (max 100) | `VariableService`, `VariableTransaction`, `execute_operations`, over `SqliteStore` or `MemoryVariableStore` |
| Sharding | `BotConfig::sharding` (`ShardingOptions`: ids, count or auto, leader shard, `ShardSchedulerMode`), `FLUXER_SHARD_IDS` / `FLUXER_SHARD_COUNT` / `FLUXER_SHARD_PROCESS_ID`, `BotShard` (`shard_id_for_guild`, `is_leader`, `should_start_jobs`, statuses), `HookEvent::ShardStatus` |
| Cluster | `Cluster` (`add`, `add_runner`, `restart`, `remove`, `destroy`, entries with status / last error / metadata, lifecycle events) |
| Function-mode scripting (`$fn[args]` language) | `flush-functions` crate: parser, evaluator, registry and `ScriptContext`; builtins in `builtins::language` (locals/control) and `builtins::entities` — the entity and native groups (users, messages, channels, expressions, threads, roles, webhooks, invites, collectors, guild/info/members), ~119 functions total so far |

### Deferred

Each of these is a follow-up of its own:

1. **Function-mode builtins leftovers**: `flush-functions` ships the
   evaluator plus the `language` and `entities` builtin groups; the
   remaining yuzuha builtin files (math, text, arrays, objects, dates,
   utility, moderation, functions-mode meta) still wait on their own
   porting pass.
2. **CLI leftovers**. The `flush` CLI covers scaffolding, config loading and
   the run/reload lifecycle ([CLI.md](CLI.md)). What is still missing waits
   on other items of this list: `functions`, `lint`, `test`, `editor` and
   `init --functions` (item 1); `run --shards` (item 4); plus `data cleanup`,
   `data` on SQLite stores, the `community`, `economy`, `utility` and
   `multifunctional` templates, and the health server behind `--health-port`.
   `check` reports the config only: the command catalog lives inside the
   compiled bot.
3. **SQLite leftovers**: persisted schedules and schedule history (Flush
   scheduler jobs are in-memory closures, so there is nothing to store yet),
   and `Bot` itself still reads the per-guild prefix from the JSON
   `GuildStore`; `SqliteStore` is used from commands.
4. **Cross-process shard supervisor**: spawning one child process per shard
   group, and `broadcastEval` / `fetchClientValues` / `respawn` across
   processes. A process started with the `FLUXER_SHARD_*` variables runs its
   shards; `BotShard::broadcast_eval` covers the shards of one process only.
   `ShardCount::Auto` sizes from the first page of the bot's guild list.
5. **Voice runtime** (Flush decodes voice state/server events but opens no
   voice connection).
6. **Music / Lavalink** integration.
7. **Canvas / image** tooling.
8. **Command hot reload** (`load`/`reload` of command directories) and
   cron-expression schedules (the scheduler has delays and intervals only).
