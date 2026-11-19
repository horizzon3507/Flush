# CLI reference

`flush` is the command line of the framework, ported from Yuzuha 0.8.0's CLI.
Install it from a checkout with `cargo install --path crates/flush-cli`, or run
it in place with `cargo run -p flush-cli -- <command>`.

A Flush bot is a compiled Rust crate, where a Yuzuha bot is a directory of
scripts loaded at runtime. That changes three things, and nothing else:

- The config is `flush.toml` (static data) instead of `yuzuha.config.ts`.
- `check` validates the config; `check --build` also compiles the commands.
- `dev` rebuilds and restarts the bot when a source changes, instead of
  re-importing command files into a running process.

## Commands

| Command | Behavior |
|---|---|
| `flush`, `flush help` or `flush --help` | Shows help |
| `flush --version` | Shows the version |
| `flush init <directory> [--template name] [--local]` | Creates a project in a new directory |
| `flush templates [--json]` | Lists template names, descriptions and commands |
| `flush check [config or directory] [--json] [--build]` | Validates the config without logging in |
| `flush doctor [config or directory] [--json]` | Offline diagnosis, never prints the token |
| `flush data <stats\|backup\|restore> <store.json> [options]` | Maintenance and backup of a JSON store, no config needed |
| `flush run [config or directory]` | Loads `.env` and the config, builds in release mode and runs the bot |
| `flush dev [config or directory]` | Runs a debug build and restarts it when sources change |

Unknown options, repeated options and extra arguments are errors. `init` never
installs or builds anything.

Failures exit with code `1`. Every failure has a stable code (`USAGE`,
`UNSUPPORTED`, `CONFIG_MISSING`, `CONFIG_INVALID`, `TOKEN_MISSING`,
`PROJECT_MISSING`, `BUILD_FAILED`, `INIT_NAME`, `INIT_TEMPLATE`,
`INIT_DIRECTORY`, `DATA_FORMAT`, `STORE_CORRUPT`, `BACKUP_INVALID`, ...). With
`--json`, the failure is written to stderr as:

```json
{ "valid": false, "code": "CONFIG_INVALID", "message": "..." }
```

Read `valid` and `code`; the wording of `message` may change.

## Config discovery

Without an explicit path the CLI reads `flush.toml` in the current directory.
A directory argument means `flush.toml` inside it; any other file name has to
be passed explicitly:

```sh
flush check ./config/bot.toml
flush run ./my-bot
```

```toml
name = "my-bot"            # required
prefix = "!"               # default prefix; the per-guild value wins
data_dir = "data"          # JSON stores, relative to this file
presence = ["!help"]       # rotated every 45s; empty = no rotation
default_cooldown_ms = 0    # per-command default, 0 = off
# origin = "https://fluxer.app"        # discovery origin
# api_base = "https://api.fluxer.app"  # set with gateway_url to skip discovery
# gateway_url = "wss://gateway.example.com"
```

Unknown keys are rejected. The same loader is public API:
`flush::bot_from_project(None)` finds the config (`FLUSH_CONFIG`, then
`flush.toml` in the current directory), loads `.env`, points
`FLUSH_DATA_DIR` at `data_dir` and builds the `Bot` with the token from
`FLUXER_BOT_TOKEN`; `flush::ProjectConfig` exposes the pieces.

`run` and `dev` read the `.env` next to the config. Values already present in
the process environment win. `check` and `doctor` never load it into the
environment.

## Validation and JSON

```sh
flush check --json
flush check ./my-bot --build
```

```json
{
  "valid": true,
  "version": "0.2.0",
  "built": false,
  "config": { "name": "my-bot", "prefix": "!", "data_dir": "data", "presence": [], "default_cooldown_ms": 0 }
}
```

`check` does not log in and does not prove the token or remote permissions.
`--build` runs `cargo check` in the project, which is what validates the
command code. The command list itself only exists inside the compiled bot, so
it is not part of this report (`!help` lists it at runtime).

## Offline diagnosis (`doctor`)

`doctor` runs local checks only (`"offline": true`) and exits `1` when any of
them fails:

- `rust`: `rustc` is installed and at least the supported version.
- `token`: present in `FLUXER_BOT_TOKEN` or in `.env` next to the config. The
  value is never printed and the environment is not modified.
- `config`: `flush.toml` parses and validates.
- `client`: `origin`, `api_base` and `gateway_url`, warning on non-loopback
  endpoints without TLS and hiding credentials and query strings.
- `storage`: the data directory, or its closest existing ancestor, is writable.
  Nothing is written.
- `project`: a `Cargo.toml` with a Flush dependency sits next to the config.

## Run and reload lifecycle

`run` and `dev` validate the config and the token first, then build the
project with cargo and start the bot binary directly, with the project
directory as its working directory and the `.env` values in its environment.
`FLUSH_CONFIG` carries the path of the validated config, so a bot started
with `flush run ./config/bot.toml` reads that same file.

`dev` watches `src/**/*.rs`, `Cargo.toml`, the config file and `.env`. Changes
that arrive close together are grouped, and reloads never overlap. A change
triggers a rebuild:

- If the build succeeds, the running bot is stopped and the new one started.
- If the build fails, the reload is refused and the previous version stays up.

`SIGINT` and `SIGTERM` stop the bot and exit `0`. Stopping sends `SIGTERM` and
waits up to 10 seconds before killing the process (on Windows the process is
killed directly). When the bot exits on its own, `run` exits with the bot's
code and `dev` keeps waiting for the next change.

## Scaffolding

```sh
flush init ./my-bot
flush init ./mod-bot --template moderation
flush init ./dev-bot --local
```

The directory must be new; a failure midway removes what was written. The
project has `Cargo.toml`, `flush.toml`, `.env.example`, `.gitignore`,
`README.md`, `src/main.rs` and one file per command under `src/commands/`,
registered in `src/commands/mod.rs`. Without `--template` the commands are
`ping`, `hello` and `ban`. Generated command files carry unit tests for their
pure logic, so `cargo test` runs offline.

By default the dependency is the published `flush-lib` at the CLI's version.
`--local` points it at the checkout the CLI was built from; use it to develop
the framework, and switch to the published crate before sharing the project.

## Templates

| Template | Commands |
|---|---|
| `simple` | `ping`, `about`, `avatar` |
| `moderation` | `simple` plus `ban`, `kick`, `timeout`, `unban`, `case` |
| `entertainment` | `simple` plus `dice`, `coin`, `choose`, `eightball` |

Every project enables the built-in `help`. Moderation commands check the
author's and the bot's permissions and the role hierarchy, and record a case.

## Store maintenance (`data`)

```sh
flush data stats data/cases.json
flush data backup data/cases.json --file cases-backup.json
flush data restore data/cases.json --file cases-backup.json
flush data restore data/cases.json --file cases-backup.json --apply --policy merge
```

Works on `guilds.json` and `cases.json` and always prints JSON. `backup`
without `--file` prints the backup; with `--file` it never overwrites an
existing file. `restore` is a dry run unless `--apply` is given; `--policy`
is `replace` (default) or `merge` (guilds in the backup win, the others are
kept). Restoring a backup of the other store kind is refused, and so is a
backup with an entry the stores could not read back. A `replace` restore also
works when the store is missing or damaged. Restart the bot after `--apply`:
a running bot keeps the store it loaded in memory.

## Not ported yet

These Yuzuha commands and options exit with `UNSUPPORTED` and name what they
wait for (see [PARITY.md](PARITY.md)):

- `functions`, `lint`, `test`, `editor` and `init --functions`: the
  function-mode scripting language.
- `run --shards` / `--shards-per-process`: the sharding and cluster runtime.
- `run`/`dev --health-port` / `--health-host`: Flush has no health server.
- `data cleanup` and `.sqlite`/`.db` stores: the SQLite store and the variable
  service.
- The `community`, `economy`, `utility` and `multifunctional` templates: they
  persist state through the variable service.

`init --js` / `--ts` do not apply: every Flush project is a Rust crate.
