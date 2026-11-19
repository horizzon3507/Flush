# Changelog

We follow [Semantic Versioning](https://semver.org/) and [Keep a Changelog](https://keepachangelog.com/). Flush is a single lib surface for crates.io.

<details>
<summary>To see more about versioning, expand this.</summary>

Every version string starts with `v` (required), e.g. `v0.2.0`, `v0.1.0`.

As libraries, all crates release under one shared version — **Flush** (`flush-core` / `flush-gateway` / `flush-rest` / `flush-framework` / `flush-lib`). No per-surface sections and no `m` in the tag: just the version notes.

Each release heading is the version and date (`## v0.1.1 · 17/10/2026`); under it, a short summary ends with a plain sentence like: “This version was made for the Flush lib on 17/10/2026 (v0.1.1).”

</details>

## v0.2.0 · 19/11/2026

First parity release against `@fluxerjs` 3.3.0 and Yuzuha 0.8.0, and the whole project is now in English. This version was made for the Flush lib on 19/11/2026 (v0.2.0).

- **English everywhere**: docs, comments, log lines, error messages and every string the framework sends to users (cooldown, permission, hierarchy, help and mention replies) are now in English. Bots relying on the Portuguese built-in replies will see the new wording.
- **REST — full route table**: `Routes` now has all 92 builders of `@fluxerjs/types` 3.3.0, and `RestClient` gained 69 typed methods: webhooks (bot- and token-authenticated, execute, edit/delete message), emojis and stickers (single, bulk, clone, metadata), audit logs, threads (start, active/archived listings, search, members, archive/lock), forum tags, permission overwrites, channel/role positions, member search, guild edit, vanity URL, current-user guilds, leave guild, user profile, application info and message search. `RestClient::raw` reaches anything without a typed method.
- **Gateway — events**: `DISPATCH_EVENTS` names all 80 dispatch events and 23 more decode into typed `Event` variants (threads, emojis, stickers, invites, webhooks, voice state/server, audit log entries, member chunks, bulk role/channel updates, pins, recipients, reaction clears, user update). `Event::wire_name()` returns the wire name even for `Unknown`.
- **Core — entities**: `Webhook`, `Emoji`, `Sticker`, `AuditLog`, `VoiceState`, `VoiceServerUpdate`, `Application`, thread metadata/members and forum tags; `Channel` carries thread, forum and voice fields; `ChannelType` and `MessageType` gained the SDK's missing values; CDN helpers for banners, member assets, guild assets, emojis and stickers with the SDK size ladder.
- **Framework — typed arguments**: `Args` parses user, role and channel mentions, integers, booleans and durations (`1h30m`) with `ArgError` describing what was expected.
- **Framework — middleware**: `Bot::middleware()` runs `Middleware` before each command; `Guard` and `guild_only` cover the common cases.
- **Framework — collectors and scheduler**: `Bot::collectors()` awaits messages or reactions with filters and timeouts; `Bot::scheduler()` runs one-shot and interval jobs with cancellation.
- **Framework — hooks**: `HookEvent::Raw` delivers every gateway event; exhaustive matches on `HookEvent` need a new arm. Thread channels are cached like regular channels.
- **Breaking**: `MessageType` 99 now decodes as `ClientSystem` instead of `Unknown`; forum and media channels are no longer `is_text_bearing()`.
- `docs/PARITY.md` lists what matches and what is deferred to follow-up releases (script language, CLI, SQLite store, sharding, voice, music, canvas, attachment uploads).

## v0.1.4 · 19/10/2026

Fixes `INVALID_FORM_BODY` on command replies in DMs (reported by Sourbot). This version was made for the Flush lib on 19/10/2026 (v0.1.4).

- **Core — DM fix**: `MessageReference` no longer serializes `guild_id: null` or `type: 0`. Adam rejects the form when the reference carries a null `guild_id` and a Default `type` (`400 INVALID_FORM_BODY`), which broke every command `reply` in DMs; the fields are now omitted when absent, mirroring `toMessageReference` in the `@fluxerjs` SDK. Guild payloads are unchanged.

## v0.1.3 · 18/10/2026

Critical connection fix for the `wss` gateway. This version was made for the Flush lib on 18/10/2026 (v0.1.3).

- **Gateway — handshake HTTP 400 fix**: `gateway_url_with_params` now guarantees a trailing `/` on the host before appending `?v=1&encoding=json`. With `tungstenite` ≥ 0.30 serializing `path_and_query` raw, a URL without a path (`wss://gateway.fluxer.app?v=1…`) went out as `GET ?v=1&encoding=json HTTP/1.1` and the gateway answered `HTTP 400 Bad Request`, so no bot on the framework could connect. The function also no longer duplicates the slash or params when the URL is already complete.
- New regression test in `flush-gateway` covering 4 URL shapes (no slash, with slash, full query, partial query); the existing expectation in `protocol.rs` was updated.

## v0.1.1 · 17/10/2026`); under it, a short summary ends with a plain sentence like: “This version was made for the Flush lib on 17/10/2026 (v0.1.1).”

</details>

## v0.1.4 · 19/10/2026

Fix de `INVALID_FORM_BODY` em respostas de comando via DM (report do Sourbot). This version was made for the Flush lib on 19/10/2026 (v0.1.4).

- **Core — fix de DM**: `MessageReference` não serializa mais `guild_id: null` nem `type: 0`. O Adam rejeita o form quando a referência carrega `guild_id` nulo e `type` Default (`400 INVALID_FORM_BODY`), o que quebrava todo `reply` de comando em DM; os campos agora são omitidos quando ausentes, espelhando o `toMessageReference` do SDK `@fluxerjs`. Em guild o payload não muda.

## v0.1.3 · 18/10/2026

Fix crítico de conexão no gateway `wss`. This version was made for the Flush lib on 18/10/2026 (v0.1.3).

- **Gateway — fix do HTTP 400 no handshake**: `gateway_url_with_params` agora garante `/` no fim do host antes de anexar `?v=1&encoding=json`. Com o `tungstenite` ≥ 0.30 serializando `path_and_query` cru, a URL sem path (`wss://gateway.fluxer.app?v=1…`) saía como `GET ?v=1&encoding=json HTTP/1.1` e o gateway respondia `HTTP 400 Bad Request` — nenhum bot do framework conseguia conectar. A função também não duplica barra nem params se a URL já vier completa.
- Teste de regressão novo no `flush-gateway` cobrindo 4 formatos de URL (sem barra, com barra, query completa, query parcial); expectativa do teste existente em `protocol.rs` atualizada.

## v0.1.1 · 17/10/2026

Reactions, pins, invites, roles CRUD, per-command permission gates, hooks, embedded help and a mod log. This version was made for the Flush lib on 17/10/2026 (v0.1.1).

- **REST — reactions**: `add_reaction`, `remove_own_reaction`, `remove_reaction` (member, needs Manage Messages), `clear_reactions` and `reaction_users`, with `ReactionEmoji::encoded()` (unicode percent-encoding, custom as `name:id`).
- **REST — pins**: `pin_message`, `unpin_message` and `pinned_messages` via `GET /channels/{c}/messages/pins`.
- **REST — invites**: `channel_invites`, `create_invite` (max age, max uses, temporary, unique), `guild_invites`, `invite(code)` and `delete_invite`, with a null-tolerant `Invite` type in `flush-core`.
- **REST — roles & channels**: `create_role` / `edit_role` / `delete_role` via `RolePatch`, `create_channel` (`ChannelCreate`), `delete_channel`, plus `create_dm` and `set_nick`.
- **Framework — permission gates**: `Command::required_permission()` is enforced by the router before execution — it checks both the executor and the bot (server owner bypasses) and reads channel overwrites.
- **Framework — embedded help**: `Bot::enable_help()` registers a `help` command auto-generated from every registered command (name, description, usage, aliases, required permission), synced on `run()`.
- **Framework — event hooks**: `Bot::hooks()` receives `HookEvent::Ready` / `GuildAdd` / `MemberJoin` / `MemberLeave` / `Message` dispatches.
- **Framework — mod log**: `ModAction.log_channel` mirrors each moderation success to a log channel without ping, resolved from `GuildStore.mod_log_channel_id`.
- Command helpers gained `CommandCtx::target_member()` (mention/id resolution via REST, 404 → `Ok(None)`) and `MessageSend::no_ping_text()`.
- Various other small tweaks

## v0.1.0 · 16/10/2026

First public cut of the Fluxer bot framework. This version was made for the Flush lib on 16/10/2026 (v0.1.0).

- Five-crate workspace: `flush-core` (types + null-tolerant wire), `flush-gateway` (WS v1 Hello/Identify/Resume/Heartbeat with 60s resume retention and backoff), `flush-rest` (discovery via `GET /.well-known/fluxer` with fallback, `Bot` auth, 429 retry), `flush-framework` (prefix router) and `flush-lib` (facade + `ping` example).
- Prefix router mirroring real bots: ignores bots/self, per-guild prefix, aliases, `message.id` dedup, cooldown replies, mention-reply with prefix, rotating presence every 45s.
- Moderation suite: permission+hierarchy checks, `with_mod_action` via `ModAction`, `GuildStore`/`CaseStore` with atomic JSON writes.
- REST core: messages (reply with `message_reference`, edit, bulk-delete 1–100), members (pagination by `after`), bans, timeouts, roles lookup.
- Various other small tweaks and 26 workspace tests
