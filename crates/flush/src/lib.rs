//! Flush: a Rust framework/library for Fluxer bots.
//!
//! Re-exports the four crates:
//!
//! - `flush_core`: types (Snowflake, permissions, entities, events)
//! - `flush_gateway`: Gateway WebSocket v1 (Hello/Identify/Resume/Heartbeat)
//! - `flush_rest`: `/v1` REST client + discovery
//! - `flush_framework`: prefix command router
//!
//! Minimal example in `examples/ping.rs` (`!ping` + `!help`).

pub use flush_canvas as canvas;
pub use flush_core as core;
pub use flush_framework as framework;
pub use flush_gateway as gateway;
pub use flush_rest as rest;

#[cfg(feature = "sqlite")]
pub use flush_framework::SqliteStore;
pub use flush_framework::{bot_from_project, ConfigError, ProjectConfig};
pub use flush_framework::{
    with_mod_action, CaseStore, CaseType, ModAction, ModCase, PermissionAction,
};
pub use flush_framework::{Bot, BotConfig, Command, CommandCtx, CommandError, CommandResult};
pub use flush_framework::{CommandInfo, Help, HelpCmd, HookEvent, Hooks, NoHooks};
pub use flush_framework::{
    MemoryVariableStore, Ttl, VariableError, VariableQueryOptions, VariableScope, VariableService,
    VariableStore,
};
