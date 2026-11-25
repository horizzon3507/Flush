# Flush (Rust bot framework)

> Rust framework/library for **Fluxer** bots (`async-std`), tracking
> `@fluxerjs` 3.3 and Yuzuha 0.8.0. Mirrors `jully-fluxer` and `Sour! (Fluxer)`.
> Everything in this repository is written in English.

## Commands

```bash
cargo check --workspace --all-targets
cargo test --workspace          # no network
cargo test -p flush-cli --test cli -- --ignored   # after touching crates/flush-cli/templates
cargo clippy --workspace --all-targets   # must stay at zero warnings
FLUXER_BOT_TOKEN=... cargo run -p flush-lib --example ping   # connects for real
```

NEVER run the example without explicit permission (it opens production WS + REST).
To validate without the network: `cargo test`, `cargo check`, `cargo clippy`.
There is no dummy token with a timeout here: the example has no offline mode.

## Layout

```
crates/flush-core/       types: Snowflake, permissions, entities, Event::decode
crates/flush-gateway/    WS v1: Hello/Identify/Resume/Heartbeat + GatewayClient
crates/flush-rest/       REST /v1: discovery, Bot <token>, client.rs + resources.rs
crates/flush-framework/  router: prefix, cooldown, args, middleware, collectors,
                         scheduler, GuildStore, CaseStore, moderation,
                         VariableService, SqliteStore (feature `sqlite`)
crates/flush/            facade + examples/ping.rs (!ping round-trip, !help)
crates/flush-cli/        `flush` binary: init/templates/check/doctor/data/run/dev;
                         templates/ holds the generated command sources
docs/CLI.md              CLI reference
docs/PARITY.md           what matches @fluxerjs / Yuzuha and what is deferred
```

## Rules

- Wire: Snowflake is always a decimal string; numeric enums fall back to `Unknown`;
  lists accept `null`; permissions are decimal strings; guilds arrive nested under
  `properties` or flat. Tests in `flush-core/tests/wire.rs` lock this in.
- Reply: `message_reference` + `replied_user: false` when not pinging.
- Ban `PUT .../bans/...`, timeout `PATCH` member, bulk-delete 1=`DELETE`/2–100=`POST`.
- Roles: `role()` lists + `find`s; `fetch_role()` uses the single-role route.
- Typing never fails a command.
- Token-scoped webhook routes go through `request_with(.., authed = false)`:
  never send the bot token to them.
- New gateway events: add the name to `DISPATCH_EVENTS`, a variant, its `kind()`
  arm and a `decode_extended` arm; malformed payloads must fall back to `Unknown`.
- `Command::execute` returns `Pin<Box<dyn Future>>` (object-safe);
  `Future` via prelude/import, never inline `std::future::Future`;
  match ergonomics (no `ref`); no generic AI comments.
- Collectors must be awaited from a spawned task, never inside `execute`
  (commands run inline on the event loop).
- `with_mod_action` takes `ModAction { ... }` (a struct, not 10 args).
- Update `docs/PARITY.md` when coverage changes.
- CLI: every failure has a stable code (`CliError`); files under
  `crates/flush-cli/templates/` are generated verbatim and must stay
  rustfmt- and clippy-clean (the ignored `cli` test checks it).
- Never edit `.commandcode/taste/`.
