# Porting yuzuha builtins to flush-functions

This crate is a Rust port of `yuzuha/src/functions/` (v0.8.0). The runtime
(`ast`, `parser`, `evaluator`, `call`, `runtime`, `registry`, `types`,
`values`, `limits`, `errors`, `context`) is done; builtins are ported in
groups, one module per group, registered from `builtins/mod.rs`.

## The pattern

Every builtin is registered inside a `pub fn register(d: &mut Definitions)`
in `src/builtins/<group>/<file>.rs`, wired through `<group>/mod.rs` and
`builtins::register_builtins`:

```rust
d.add(
    "name",            // without the $
    "category",        // yuzuha's category string, verbatim
    1,                 // min_args
    2,                 // max_args (usize::MAX for variadic)
    "[a;b?]",          // usage tail — "$name" is prepended automatically
    "Description.",    // English
    |c| Box::pin(async move {
        let first = c.arg(0).await?;      // lazy, cached
        let again = c.fresh(0).await?;    // lazy, re-evaluated
        let rest = c.text(1, None).await?; // args 1.. joined by " "
        Ok(serde_json::Value::String(first))
    }),
);
d.add_aliased(..., &["alias1", "alias2"]);
```

`execute` returns `FnOutput = Result<serde_json::Value, ScriptFailure>`;
the evaluator stringifies the Value (`values::text`). Return `Value::Null`
for `''`.

## The `c: &ScriptCall` surface

- `c.ctx` — `&Arc<ScriptContext>`: `.message`, `.guild_id()`,
  `.author_id()`, `.channel_id()`, `.message_id()`, `.rest`,
  `.vars`-helpers (`var_get/var_set/var_delete`), `.loop_frame()`/
  `.set_loop_frame()`, `.aborted`, `.execution_id()`, `.last_case`,
  `.moderation_applied`, `.emoji`, `.owners`, `.dev_guilds`,
  `.is_owner()`, `.is_dev_guild()`, `.require_guild()`, `.assert()`,
  `.variables` (Option<Arc<VariableService>> — `variables.get/set/
  increment/query/ttl/expire/delete`, scope `VariableScope::{Guild,User,
  Global}`), `.cases`, `.scheduler`, `.collectors`, `.guild`, `.channel`,
  `.member`, `.bot_member`, `.bot_id`, `.prefix`, `.input`, `.event`,
  `.actor`, `.member_permission_bits(guild, channel, roles, member_id)`,
  `.now` (ms clock fn).
- `c.evaluator` — `.limits()`, `.remaining_ms()`,
  `.evaluate(ctx, &node, depth)` / `.evaluate_many(ctx, &[node], depth)`,
  `.wrap(future)` (wrapError semantics), `.check_output(prev, current)`.
- `c.arg(i)`, `c.fresh(i)`, `c.text(i, sep)`, `c.all(sep)`, `c.wait(fut)`
  (pauses the deadline for external awaits — always wrap REST/HTTP calls),
  `c.check_output(prev, current)`, `c.node`, `c.name`, `c.depth`,
  `c.arg_count()`.

## Errors (`crate::errors`)

- `FnError::new(code, msg)` — code from `codes::*`
  (`INVALID_ARGUMENTS`, `FUNCTION_ERROR`, `API_ERROR`,
  `NATIVE_UNAVAILABLE`, ...). Internal kinds live in `ScriptFailure`
  variants: `Guard(GuardError::new(msg, GuardReason::X))`,
  `Stop(StopExecution)`, `Loop(LoopControl { frame, action })`,
  `PartialModeration(PartialModerationError {...})`.
- Guards, stops, loop control, partial moderation and the limit codes
  (`EXECUTION_LIMIT`, `OUTPUT_LIMIT`, `EXECUTION_ABORTED`) are never
  swallowed — `is_uncatchable` is enforced by `$try` and `evaluator.wrap`.
- REST errors convert via `?` (`From<RestError>` → `API_ERROR`).

## Value helpers (`crate::values`)

`text(&Value) -> String` (JS `String(v)`), `truthy(&Value) -> bool`
(JS truthiness + `==`/`!=`/`>=`/`<=`/`>`/`<` string comparisons),
`js_number(&str) -> Option<f64>`, `typed_scalar(&Value) -> Value`
(`"42"` → `42`, `"true"` → `true`, ...), `deep_equal`, `to_json`
(non-finite scrubbed). Args arrive as `String`; wrap in
`Value::String(...)` then `typed_scalar` when the yuzuha code does
`typedScalar`.

## Semantics that must match yuzuha exactly

- Args evaluate lazily; `arg(i)` caches, `fresh(i)` re-evaluates.
- Loops: each loop gets a frame id (monotonic counter);
  `ctx.set_loop_frame(Some(id))` while the body runs, restore after;
  `LoopControl` with a foreign frame id must be rethrown.
  `withLoopState` snapshots `__loopIndex/__loopValue/__loopAccumulator`
  vars and clears the loop frame for array callbacks. LOOP_MAX = 1000.
- `$try`/`wrap`: rethrow uncatchable; else FUNCTION_ERROR.
- `payload` normalizes to `Value` message options; `tts` requires
  SendTtsMessages, mentions require MentionEveryone.
- Voice/music/canvas builtins register normally and return
  `FnError(codes::NATIVE_UNAVAILABLE, "...")` — the native services are
  deferred parity items.

## Tests

One `tests/<group>.rs` per group. Build a ctx like `tests/runtime.rs`
(fake `RestClient` at `api_base http://127.0.0.1:9`, `now: Box::new(|| 0)`).
No outside network. For REST-touching builtins, either test argument/guard
paths only, or run a loopback `surf` server in the test and point
`api_base` at it.

## Commands (must all be clean before a PR)

```bash
cargo check -p flush-functions --all-targets
cargo test -p flush-functions
cargo clippy -p flush-functions --all-targets   # zero warnings
cargo fmt
```

## Repo rules that apply

Everything in English. No generic AI comments; comment to explain the
code, not the edit. Match ergonomics (no `ref`). Futures via
`Pin<Box<dyn Future + Send>>` (`BoxFnFuture`), never inline
`std::future::Future`. `#[allow(clippy::too_many_arguments)]` only on the
`Definitions::add*` helpers.
