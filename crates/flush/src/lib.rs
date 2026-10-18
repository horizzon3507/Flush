//! Flush: framework/lib em Rust para bots do Fluxer.
//!
//! Re-exporta os quatro crates:
//!
//! - `flush_core`: tipos (Snowflake, permissões, entidades, eventos)
//! - `flush_gateway`: Gateway WebSocket v1 (Hello/Identify/Resume/Heartbeat)
//! - `flush_rest`: cliente REST `/v1` + discovery
//! - `flush_framework`: roteador de comandos com prefixo
//!
//! Exemplo mínimo em `examples/ping.rs` (`!ping` + `!help`).

pub use flush_core as core;
pub use flush_framework as framework;
pub use flush_gateway as gateway;
pub use flush_rest as rest;

pub use flush_framework::{
    with_mod_action, CaseStore, CaseType, ModAction, ModCase, PermissionAction,
};
pub use flush_framework::{Bot, BotConfig, Command, CommandCtx, CommandError, CommandResult};
pub use flush_framework::{CommandInfo, Help, HelpCmd, HookEvent, Hooks, NoHooks};
