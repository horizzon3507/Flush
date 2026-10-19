# Changelog

We follow [Semantic Versioning](https://semver.org/) and [Keep a Changelog](https://keepachangelog.com/). Flush is a single lib surface for crates.io.

<details>
<summary>To see more about versioning, expand this.</summary>

Every version string starts with `v` (required), e.g. `v0.2.0`, `v0.1.0`.

As libraries, all crates release under one shared version — **Flush** (`flush-core` / `flush-gateway` / `flush-rest` / `flush-framework` / `flush-lib`). No per-surface sections and no `m` in the tag: just the version notes.

Each release heading is the version and date (`## v0.1.1 · 17/10/2026`); under it, a short summary ends with a plain sentence like: “This version was made for the Flush lib on 17/10/2026 (v0.1.1).”

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
