# Flush (bot framework Rust)

> Framework/lib Rust para bots do **Fluxer** (`async-std`), paridade prática
> com `@fluxerjs/core` 2.x. Espelha `jully-fluxer` e `Sour! (Fluxer)`.

## Comandos

```bash
cargo check --workspace --all-targets
cargo test --workspace          # 26 testes, sem rede
cargo clippy --workspace --all-targets   # deve ficar zerado
FLUXER_BOT_TOKEN=... cargo run -p flush-lib --example ping   # conecta de verdade
```

NUNCA rode o exemplo sem permissão explícita (abre WS + REST de produção).
Para validar sem rede: `cargo test`, `cargo check`, `cargo clippy`.
Sem token dummy com timeout aqui — o exemplo não aceita modo offline.

## Estrutura

```
crates/flush-core/       tipos: Snowflake, permissões, entidades, Event::decode
crates/flush-gateway/    WS v1: Hello/Identify/Resume/Heartbeat + GatewayClient
crates/flush-rest/       REST /v1: discovery, Bot <token>, mensagens/membros/bans
crates/flush-framework/  router: prefixo, cooldown, GuildStore, CaseStore, moderação
crates/flush/            fachada + examples/ping.rs (!ping round-trip, !help)
```

## Regras

- Wire: Snowflake sempre string decimal; enums numéricos com fallback `Unknown`;
  listas aceitam `null`; permissões string decimal; guild aninhada em
  `properties` ou flat. Testes em `flush-core/tests/wire.rs` travam isso.
- Reply: `message_reference` + `replied_user: false` quando sem ping.
- Ban `PUT .../bans/...`, timeout `PATCH` member, bulk-delete 1=`DELETE`/2–100=`POST`.
- Roles: sem GET individual — lista + `find`.
- Typing nunca falha comando.
- Trait `Command::execute` retorna `Pin<Box<dyn Future>>` (object-safe);
  `Future` via prelude/import, nunca `std::future::Future` inline;
  match ergonomics (sem `ref`); sem comentário genérico de IA.
- `with_mod_action` recebe `ModAction { ... }` (struct, não 10 args).
- Nunca edite `.commandcode/taste/`.
