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
| Help | `Bot::enable_help()` |
| Event hooks | `Hooks`, including `HookEvent::Raw` for every gateway event |
| Typed arguments | `Args` (user, role, channel, integer, boolean, duration, rest) |
| Middleware | `Middleware`, `Guard`, `guild_only` |
| Collectors | `Collectors` (messages and reactions) |
| Scheduler | `Scheduler` (`after`, `every`, cancellation) |

### Deferred

Each of these is a follow-up of its own:

1. **Function-mode scripting language** and its ~459 built-in functions.
2. **CLI** (project scaffolding, config loading, run/reload lifecycle).
3. **SQLite store** (Flush persists `GuildStore`/`CaseStore` as JSON only),
   plus Yuzuha's variable service on top of it.
4. **Sharding and cluster runtime** (Flush opens a single gateway connection).
5. **Voice runtime** (Flush decodes voice state/server events but opens no
   voice connection).
6. **Music / Lavalink** integration.
7. **Canvas / image** tooling.
8. **Command hot reload** (`load`/`reload` of command directories) and
   cron-expression schedules (the scheduler has delays and intervals only).
