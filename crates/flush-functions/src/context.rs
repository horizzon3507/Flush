//! `ScriptContext`: the `ctx` handed to `$` functions (mirrors yuzuha's
//! `CommandContext` at the level the builtins use it).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use flush_core::{Channel, Guild, MemberUser, Message, Snowflake};
use flush_framework::{CaseStore, Collectors, ModCase, VariableService, VariableStore};
use flush_rest::RestClient;

use crate::errors::{GuardError, GuardReason};

pub type Variables = VariableService<Arc<dyn VariableStore + Send + Sync>>;

/// Guard message used when a command runs outside a server.
pub const NOT_IN_GUILD: &str = "This command only works inside a server.";
/// Guard message for a missing actor.
pub const NO_AUTHOR: &str = "Could not resolve the command author.";

/// Everything a script execution can reach: the triggering message,
/// identity snapshots, the REST client and the optional framework
/// services (variables, cases, scheduler, cooldowns, collectors).
pub struct ScriptContext {
    /// The message that ran the command (or a synthetic one for events).
    pub message: Message,
    /// Event name + payload when running under a script event.
    pub event: Option<(String, Value)>,
    /// Actor override (script events may run as a different member).
    pub actor: Option<(Snowflake, Snowflake)>, // (guild_id, author_id)
    pub prefix: String,
    pub input: String,
    pub rest: Arc<RestClient>,
    pub bot_id: Option<Snowflake>,
    pub guild: Option<Guild>,
    pub channel: Option<Channel>,
    pub member: Option<MemberUser>,
    pub bot_member: Option<MemberUser>,
    /// `$let`/`$get` locals and loop state.
    pub vars: Mutex<HashMap<String, Value>>,
    /// Innermost active loop frame (0 = outside loops).
    pub loop_frame: AtomicU64,
    /// Abort flag shared with the evaluator (`ctx.signal.aborted`).
    pub aborted: AtomicBool,
    pub execution_id: Mutex<Option<String>>,
    /// Last recorded moderation case (`ctx.lastCase`).
    pub last_case: Mutex<Option<ModCase>>,
    /// Whether a moderation action ran in this execution.
    pub moderation_applied: AtomicBool,
    /// Name → emoji mapping configured on the bot.
    pub emoji: HashMap<String, String>,

    /// Declarative argument values, for `$arg[name]` (empty by default).
    pub args: HashMap<String, Value>,

    /// Gateway latency in ms, for `$ping` (`None` → empty).
    pub ping_ms: Option<u64>,

    /// Help text of the invoked command, for `$help` (`None` → empty).
    pub help: Option<String>,
    /// Owner user IDs (for `isOwner`).
    pub owners: Vec<Snowflake>,
    /// Dev guild IDs (for `isDevGuild` checks).
    pub dev_guilds: Vec<Snowflake>,
    pub variables: Option<Arc<Variables>>,
    pub cases: Option<Arc<Mutex<CaseStore>>>,
    /// In-memory scheduler records for the `schedule` builtins.
    pub scheduler: Option<Arc<crate::scheduler::ScriptScheduler>>,
    /// In-memory `(key -> until ms)` cooldown claims for `$cooldown`.
    pub cooldowns: Option<Arc<Mutex<HashMap<String, u64>>>>,
    pub collectors: Option<Arc<Collectors>>,
    /// Milliseconds clock (`ctx.now`) — a fn so tests can pin it.
    pub now: Box<dyn Fn() -> u64 + Send + Sync>,
}

impl ScriptContext {
    pub fn channel_id(&self) -> Snowflake {
        self.message.channel_id
    }

    /// `ctx.guildID`: the actor's server or the message's.
    pub fn guild_id(&self) -> Snowflake {
        if let Some((guild, _)) = &self.actor {
            return *guild;
        }
        self.message.guild_id.unwrap_or_default()
    }

    /// `ctx.authorID`: the actor or the message author.
    pub fn author_id(&self) -> Snowflake {
        if let Some((_, author)) = &self.actor {
            return *author;
        }
        self.message.author.id
    }

    /// `ctx.authorName`.
    pub fn author_name(&self) -> String {
        self.message.author.username.clone()
    }

    /// `ctx.message.id`.
    pub fn message_id(&self) -> Snowflake {
        self.message.id
    }

    /// `ctx.executionID` — lazily generated.
    pub fn execution_id(&self) -> String {
        let mut slot = self.execution_id.lock().unwrap();
        if slot.is_none() {
            *slot = Some(format!("{:x}-{:x}", (self.now)(), self.message.id.get()));
        }
        slot.clone().unwrap()
    }

    /// `ctx.isOwner()`.
    pub fn is_owner(&self) -> bool {
        let id = self.author_id();
        id.get() != 0 && self.owners.contains(&id)
    }

    /// `ctx.isDevGuild(guildID)`.
    pub fn is_dev_guild(&self, guild_id: Snowflake) -> bool {
        self.dev_guilds.contains(&guild_id)
    }

    /// `ctx.requireGuild()`.
    pub fn require_guild(&self) -> Result<Snowflake, GuardError> {
        let guild = self.guild_id();
        if guild.get() == 0 {
            return Err(GuardError::new(NOT_IN_GUILD, GuardReason::NotInGuild));
        }
        Ok(guild)
    }

    /// `ctx.assert(condition, message)`.
    pub fn assert(&self, condition: bool, message: impl Into<String>) -> Result<(), GuardError> {
        if condition {
            Ok(())
        } else {
            Err(GuardError::generic(message))
        }
    }

    /// `ctx.vars` snapshot/set helpers used by builtins.
    pub fn var_get(&self, key: &str) -> Option<Value> {
        self.vars.lock().unwrap().get(key).cloned()
    }

    pub fn var_set(&self, key: impl Into<String>, value: Value) {
        self.vars.lock().unwrap().insert(key.into(), value);
    }

    pub fn var_delete(&self, key: &str) {
        self.vars.lock().unwrap().remove(key);
    }

    pub fn loop_frame(&self) -> Option<u64> {
        match self.loop_frame.load(Ordering::Relaxed) {
            0 => None,
            frame => Some(frame),
        }
    }

    pub fn set_loop_frame(&self, frame: Option<u64>) {
        self.loop_frame.store(frame.unwrap_or(0), Ordering::Relaxed);
    }

    /// Permission bits of a member inside `guild` (+ channel overwrites when
    /// a channel snapshot is provided), mirroring `member_permissions`.
    pub fn member_permission_bits(
        guild: &Guild,
        channel: Option<&Channel>,
        roles: &[Snowflake],
        member_id: Snowflake,
    ) -> u64 {
        let everyone_bits = guild
            .roles
            .iter()
            .find(|r| r.id == guild.id)
            .map(|r| r.permissions)
            .unwrap_or(0);
        let role_bits: Vec<u64> = guild
            .roles
            .iter()
            .filter(|r| roles.contains(&r.id))
            .map(|r| r.permissions)
            .collect();
        flush_framework::Moderation::member_permissions(
            everyone_bits,
            &role_bits,
            channel,
            member_id,
            roles,
            Some(guild.id),
        )
        .bits()
    }
}
