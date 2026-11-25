//! flush-framework: prefix command router on Gateway + REST.
//!
//! Mirrors `Sour events/messageCreate.ts` + `jully Bot.js::handleMessage`:
//! - ignores bots/self, parses the prefix (default `!`, per guild via the store),
//!   looks up name + aliases (case-insensitive), dedups by message.id,
//!   cooldown per (user, command), replies with the prefix when mentioned.
//! - `CommandCtx` carries `rest` + read snapshots (guilds/channels/members
//!   from events) so handlers can call `rest.reply(...)` etc.
//! - [`ProjectConfig`]: `flush.toml` + `.env` loading shared with the `flush` CLI.
//! - Typed arguments ([`Args`]), [`Middleware`], [`Collectors`] and a [`Scheduler`].
//! - Scoped variables ([`VariableService`]) over memory or SQLite (`SqliteStore`).
pub mod args;
pub mod cases;
pub mod collector;
pub mod command;
pub mod cooldown;
pub mod guild_store;
pub mod help;
pub mod middleware;
pub mod moderation;
pub mod parse;
pub mod project;
pub mod router;
pub mod scheduler;
#[cfg(feature = "sqlite")]
pub mod sqlite;
pub mod store_lock;
pub mod variables;

pub use args::{parse_bool, parse_channel, parse_duration, parse_role, parse_user, ArgError, Args};
pub use cases::{CaseStore, CaseType, ModCase};
pub use collector::{CollectedReaction, Collector, Collectors};
pub use command::{info_block, mod_success, plain_error};
pub use command::{Command, CommandCtx, CommandError, CommandResult};
pub use cooldown::Cooldowns;
pub use guild_store::{GuildConfig, GuildConfigPatch, GuildStore};
pub use help::{CommandInfo, Help, HelpCmd};
pub use middleware::{guild_only, Flow, Guard, Middleware};
pub use moderation::{with_mod_action, ModAction, Moderation, PermissionAction};
pub use parse::{find_command, parse_mention_or_id, parse_prefix_body, ParsedCommand};
pub use project::{
    bot_from_project, find_config, load_env, parse_env, read_env_file, ConfigError, ProjectConfig,
    CONFIG_ENV, CONFIG_FILE,
};
pub use router::{Bot, BotConfig, HookEvent, Hooks, NoHooks};
pub use scheduler::{JobHandle, Scheduler};
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteStore;
pub use store_lock::StoreLock;
pub use variables::{
    execute_operations, MemoryVariableStore, OperationKind, ScopedVariables, StoreError,
    StoredVariable, Ttl, VariableAccess, VariableEntry, VariableError, VariableKey,
    VariableOperation, VariablePage, VariableQuery, VariableQueryOptions, VariableScope,
    VariableService, VariableStore, VariableTransaction, DATA_TRANSACTION_MAX,
};
