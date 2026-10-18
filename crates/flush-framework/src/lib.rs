//! flush-framework: roteador de comandos com prefixo sobre Gateway + REST.
//!
//! Espelha `Sour events/messageCreate.ts` + `jully Bot.js::handleMessage`:
//! - ignora bots/self, parse de prefixo (default `!`, por guild via store),
//!   lookup nome+aliases (case-insensitive), dedup por message.id,
//!   cooldown por (usuário, comando), aliases, menção-resposta de prefixo.
//! - `CommandCtx` entrega `rest` + snapshot de leitura (guilds/canals/members
//!   via eventos) para handlers chamarem `rest.reply(...)` etc.
pub mod cases;
pub mod command;
pub mod cooldown;
pub mod guild_store;
pub mod help;
pub mod moderation;
pub mod parse;
pub mod router;

pub use cases::{CaseStore, CaseType, ModCase};
pub use command::{info_block, mod_success, plain_error};
pub use command::{Command, CommandCtx, CommandError, CommandResult};
pub use cooldown::Cooldowns;
pub use guild_store::{GuildConfig, GuildStore};
pub use help::{CommandInfo, Help, HelpCmd};
pub use moderation::{with_mod_action, ModAction, Moderation, PermissionAction};
pub use parse::{find_command, parse_mention_or_id, parse_prefix_body, ParsedCommand};
pub use router::{Bot, BotConfig, HookEvent, Hooks, NoHooks};
