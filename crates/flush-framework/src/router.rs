//! Router: Gateway loop → parse → dispatch (mirrors
//! `Sour events/messageCreate.ts` + `Jully Bot.js::handleMessage` + `index.ts`).
//!
//! - `Bot::run`: discovery via `RestClient::new` → hello/identify/resume loop
//!   via `GatewayClient` → `MESSAGE_CREATE` → `handle_message`.
//! - Dedup by `message.id` (Sour `processingMessages`), ignores bots/self,
//!   per-guild prefix (`GuildStore`), aliases, cooldown with a "wait Ns" reply,
//!   replies to a mention with the prefix (Jully), rotates presence every 45s (Sour).

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_std::sync::RwLock;
use flush_core::{Channel, Event, Guild, MemberUser, Message, PartialUser, Presence, Snowflake};
use flush_gateway::{ClientConfig, GatewayClient};
use flush_rest::{RestClient, RestConfig};
use log::{debug, error, info, warn};

use crate::command::{plain_error, Command, CommandCtx, CommandError};
use crate::cooldown::Cooldowns;
use crate::guild_store::GuildStore;
use crate::moderation::Moderation;
use crate::parse::parse_prefix_body;

#[derive(Debug, Clone)]
pub struct BotConfig {
    pub token: String,
    /// Default prefix (env `PREFIX` or `!`); the per-guild store value wins.
    pub default_prefix: String,
    /// Origin for discovery (default `https://fluxer.app`).
    pub origin: Option<String>,
    /// Explicit API base and gateway (skip discovery).
    pub api_base: Option<String>,
    pub gateway_url: Option<String>,
    /// Path to `guilds.json` (default `./data/guilds.json`).
    pub data_path: Option<std::path::PathBuf>,
    /// Presence lines rotated every 45s (mirrors Sour). Empty = no rotation.
    pub presence_lines: Vec<String>,
    /// Default per-command cooldown when the command sets none (ms, 0 = off).
    pub default_cooldown_ms: u64,
    /// Sharding. `None` = one unsharded connection, unless the
    /// `FLUXER_SHARD_*` variables assign shards to this process.
    pub sharding: Option<crate::sharding::ShardingOptions>,
}

impl BotConfig {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            default_prefix: std::env::var("PREFIX")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "!".into()),
            origin: None,
            api_base: None,
            gateway_url: None,
            data_path: None,
            presence_lines: Vec::new(),
            default_cooldown_ms: 0,
            sharding: None,
        }
    }
}

struct Shared {
    rest: Option<RestClient>,
    bot_id: Option<Snowflake>,
    guilds: HashMap<Snowflake, Guild>,
    channels: HashMap<Snowflake, Channel>,
    members: HashMap<(Snowflake, Snowflake), MemberUser>,
}

/// Event handed to [`Hooks::dispatch`].
pub enum HookEvent<'a> {
    Ready {
        session_id: &'a str,
    },
    GuildAdd(&'a Guild),
    MemberJoin {
        guild: &'a Guild,
        member: &'a MemberUser,
    },
    MemberLeave {
        guild_id: Snowflake,
        user: &'a PartialUser,
    },
    Message(&'a Message),
    /// Every gateway event, before the bot updates its cache.
    Raw(&'a Event),
    /// A shard of a sharded bot changed connection status.
    ShardStatus {
        shard_id: u32,
        status: flush_gateway::ShardStatus,
    },
}

/// Event hooks from the `Bot` cache (ready, guilds, members, messages).
pub trait Hooks: Send + Sync {
    fn dispatch<'a>(&'a self, ev: HookEvent<'a>) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

/// Empty implementation (for `hooks: None`).
pub struct NoHooks;

impl Hooks for NoHooks {
    fn dispatch<'a>(&'a self, _ev: HookEvent<'a>) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {})
    }
}

pub struct Bot {
    config: BotConfig,
    commands: Vec<Box<dyn Command>>,
    cooldowns: Cooldowns,
    seen: HashSet<Snowflake>,
    store: GuildStore,
    shared: Arc<RwLock<Shared>>,
    presence_idx: usize,
    hooks: Option<Arc<dyn Hooks>>,
    help: Option<Arc<crate::help::Help>>,
    middleware: Vec<Arc<dyn crate::middleware::Middleware>>,
    collectors: crate::collector::Collectors,
    scheduler: crate::scheduler::Scheduler,
    shard: Arc<std::sync::RwLock<crate::sharding::BotShard>>,
}

impl Bot {
    pub fn new(config: BotConfig) -> Result<Self, CommandError> {
        if config.token.trim().is_empty() {
            return Err(CommandError::User("FLUXER_BOT_TOKEN is not set.".into()));
        }
        let store = match config.data_path.clone() {
            Some(p) => GuildStore::new(p),
            None => GuildStore::with_default_path(),
        };
        Ok(Self {
            config,
            commands: Vec::new(),
            cooldowns: Cooldowns::new(),
            seen: HashSet::new(),
            store,
            shared: Arc::new(RwLock::new(Shared {
                rest: None,
                bot_id: None,
                guilds: HashMap::new(),
                channels: HashMap::new(),
                members: HashMap::new(),
            })),
            presence_idx: 0,
            hooks: None,
            help: None,
            middleware: Vec::new(),
            collectors: crate::collector::Collectors::new(),
            scheduler: crate::scheduler::Scheduler::new(),
            shard: Arc::new(std::sync::RwLock::new(crate::sharding::BotShard::single())),
        })
    }

    /// Adds a middleware; they run in insertion order before each command.
    pub fn middleware(mut self, mw: impl crate::middleware::Middleware + 'static) -> Self {
        self.middleware.push(Arc::new(mw));
        self
    }

    /// Registry fed with this bot's messages and reactions.
    pub fn collectors(&self) -> crate::collector::Collectors {
        self.collectors.clone()
    }

    /// Shards this process runs (shard 0 of 1 until `run()` resolves them).
    pub fn shard(&self) -> crate::sharding::BotShard {
        self.shard.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Live handle to the shard view, for hooks, commands and jobs.
    pub fn shard_handle(&self) -> Arc<std::sync::RwLock<crate::sharding::BotShard>> {
        self.shard.clone()
    }

    /// Resolves the shard plan from the config and the process environment.
    /// `Ok(None)` = unsharded.
    async fn resolve_shards(
        &self,
        rest: &RestClient,
    ) -> Result<Option<(flush_gateway::ShardPlan, crate::sharding::ShardingOptions)>, CommandError>
    {
        use crate::sharding::ShardEnv;
        let invalid = |e: flush_gateway::ShardError| CommandError::Internal(e.to_string());
        let options = self.config.sharding.clone();
        let env = match &options {
            Some(o) if !o.use_env => None,
            _ => ShardEnv::from_env().map_err(invalid)?,
        };
        if options.is_none() && env.is_none() {
            return Ok(None);
        }
        let options = options.unwrap_or_default();
        let guild_count = if options.needs_guild_count() {
            let guilds = rest
                .current_user_guilds(false)
                .await
                .map_err(CommandError::Rest)?;
            Some(guilds.len() as u64)
        } else {
            None
        };
        let plan = options
            .resolve(env.as_ref(), guild_count)
            .map_err(invalid)?;
        Ok(Some((plan, options)))
    }

    pub fn scheduler(&self) -> crate::scheduler::Scheduler {
        self.scheduler.clone()
    }

    /// Registers event hooks (ready/guild/members/messages).
    pub fn hooks(mut self, hooks: impl Hooks + 'static) -> Self {
        self.hooks = Some(Arc::new(hooks));
        self
    }

    /// Registers the built-in `help` command, which lists the other commands.
    /// The snapshot is synced in `run()`.
    pub fn enable_help(mut self) -> Self {
        if self.help.is_none() {
            let h = Arc::new(crate::help::Help::new());
            self.help = Some(h.clone());
            self.commands.push(Box::new(crate::help::HelpCmd(h)));
        }
        self
    }

    /// Syncs the `help` snapshot (called internally by `run()`).
    pub fn refresh_help(&mut self) {
        if let Some(h) = &self.help {
            h.refresh(&self.commands);
        }
    }

    pub fn command(mut self, cmd: impl Command + 'static) -> Self {
        self.commands.push(Box::new(cmd));
        self
    }

    pub fn commands_vec(&self) -> &[Box<dyn Command>] {
        &self.commands
    }

    /// Main loop: REST (discovery) → Gateway → dispatch until a fatal error.
    pub async fn run(&mut self) -> Result<(), CommandError> {
        // Jobs stay off until the shard plan says this process runs them.
        let may_shard = self.config.sharding.is_some()
            || !matches!(crate::sharding::ShardEnv::from_env(), Ok(None));
        if may_shard {
            self.scheduler.set_paused(true);
        }
        let rest = RestClient::new(RestConfig {
            token: self.config.token.clone(),
            api_base: self.config.api_base.clone(),
            origin: self.config.origin.clone(),
            gateway_url: self.config.gateway_url.clone(),
            audit_reason: None,
        })
        .await
        .map_err(CommandError::Rest)?;
        let me = rest.current_user().await.map_err(CommandError::Rest)?;
        info!(
            "flush: logged in as {} ({})",
            me.partial.username, me.partial.id
        );
        {
            let mut s = self.shared.write().await;
            s.rest = Some(rest.clone());
            s.bot_id = Some(me.partial.id);
        }
        let mut gw_cfg = ClientConfig::new(self.config.token.clone(), rest.gateway_url());
        if !self.config.presence_lines.is_empty() {
            gw_cfg.presence = Some(Presence {
                status: flush_core::PresenceStatus::Online,
                activities: vec![flush_core::Activity::custom(
                    self.config.presence_lines[0].clone(),
                )],
                ..Default::default()
            });
        }
        self.refresh_help();
        if let Some((plan, options)) = self.resolve_shards(&rest).await? {
            return self.run_sharded(gw_cfg, plan, options).await;
        }
        let mut gw =
            GatewayClient::new(gw_cfg).map_err(|e| CommandError::Internal(e.to_string()))?;
        loop {
            // 45s timeout = rotating presence tick (mirrors Sour).
            let next = gw.next_event();
            match async_std::future::timeout(std::time::Duration::from_secs(45), next).await {
                Ok(Ok(Some(ev))) => self.on_event(ev).await,
                Ok(Ok(None)) => {}
                Ok(Err(e)) => {
                    error!("flush: gateway error: {e}");
                    return Err(CommandError::Internal(e.to_string()));
                }
                Err(_) => {
                    self.rotate_presence(&mut gw).await;
                }
            }
        }
    }

    /// One gateway connection per shard, all feeding this bot.
    async fn run_sharded(
        &mut self,
        gw_cfg: ClientConfig,
        plan: flush_gateway::ShardPlan,
        options: crate::sharding::ShardingOptions,
    ) -> Result<(), CommandError> {
        use flush_gateway::{ShardEvent, ShardManager, ShardManagerOptions};
        let view = crate::sharding::BotShard::new(
            plan.clone(),
            options.leader_shard_id,
            options.scheduler_mode,
        );
        info!(
            "flush: sharding ids={:?} count={} leader={}",
            view.ids(),
            view.count(),
            view.is_leader()
        );
        self.scheduler.set_paused(!view.should_start_jobs());
        *self.shard.write().unwrap_or_else(|e| e.into_inner()) = view;
        let manager_options = ShardManagerOptions {
            identify_delay: options.identify_delay,
            ..Default::default()
        };
        let mut shards = ShardManager::spawn(gw_cfg, plan, manager_options)
            .map_err(|e| CommandError::Internal(e.to_string()))?;
        loop {
            let next = shards.next();
            match async_std::future::timeout(std::time::Duration::from_secs(45), next).await {
                Ok(Some(ShardEvent::Dispatch { event, .. })) => self.on_event(*event).await,
                Ok(Some(ShardEvent::StatusChanged { shard_id, status })) => {
                    self.shard
                        .write()
                        .unwrap_or_else(|e| e.into_inner())
                        .set_status(shard_id, status);
                    if let Some(h) = &self.hooks {
                        h.dispatch(HookEvent::ShardStatus { shard_id, status })
                            .await;
                    }
                }
                Ok(Some(ShardEvent::Fatal { shard_id, error })) => {
                    error!("flush: shard {shard_id} gateway error: {error}");
                    shards.shutdown().await;
                    return Err(CommandError::Internal(format!("shard {shard_id}: {error}")));
                }
                Ok(None) => {
                    return Err(CommandError::Internal("every shard stopped".into()));
                }
                Err(_) => {
                    if let Some(presence) = self.next_presence() {
                        shards.set_presence(&presence);
                    }
                }
            }
        }
    }

    async fn on_event(&mut self, ev: Event) {
        if let Some(h) = &self.hooks {
            h.dispatch(HookEvent::Raw(&ev)).await;
        }
        match &ev {
            Event::MessageCreate(m) => self.collectors.offer_message(m),
            Event::MessageReactionAdd {
                channel_id,
                message_id,
                user_id,
                emoji,
                ..
            } => self
                .collectors
                .offer_reaction(&crate::collector::CollectedReaction {
                    channel_id: *channel_id,
                    message_id: *message_id,
                    user_id: *user_id,
                    emoji: emoji.clone(),
                }),
            _ => {}
        }
        match ev {
            Event::ThreadCreate(c) | Event::ThreadUpdate(c) => {
                self.shared.write().await.channels.insert(c.id, c);
            }
            Event::ThreadDelete { id, .. } => {
                self.shared.write().await.channels.remove(&id);
            }
            Event::ChannelUpdateBulk { channels } => {
                let mut s = self.shared.write().await;
                for c in channels {
                    s.channels.insert(c.id, c);
                }
            }
            Event::Ready(r) => {
                info!(
                    "flush: ready session={} guilds={}",
                    r.session_id,
                    r.guilds.len()
                );
                if let Some(h) = &self.hooks {
                    h.dispatch(HookEvent::Ready {
                        session_id: &r.session_id,
                    })
                    .await;
                }
            }
            Event::GuildCreate(g) => {
                debug!("flush: guild create {} ({})", g.name, g.id);
                self.shared.write().await.guilds.insert(g.id, g.clone());
                if let Some(h) = &self.hooks {
                    h.dispatch(HookEvent::GuildAdd(&g)).await;
                }
            }
            Event::GuildUpdate(g) => {
                self.shared.write().await.guilds.insert(g.id, g);
            }
            Event::GuildDelete { id, .. } => {
                self.shared.write().await.guilds.remove(&id);
            }
            Event::ChannelCreate(c) | Event::ChannelUpdate(c) => {
                self.shared.write().await.channels.insert(c.id, c);
            }
            Event::ChannelDelete(c) => {
                self.shared.write().await.channels.remove(&c.id);
            }
            Event::GuildMemberAdd { guild_id, member } => {
                {
                    let mut s = self.shared.write().await;
                    s.members.insert((guild_id, member.user.id), member.clone());
                }
                if let Some(g) = self.shared.read().await.guilds.get(&guild_id).cloned() {
                    if let Some(h) = &self.hooks {
                        h.dispatch(HookEvent::MemberJoin {
                            guild: &g,
                            member: &member,
                        })
                        .await;
                    }
                }
            }
            Event::GuildMemberUpdate {
                guild_id,
                user,
                roles,
                nick,
                ..
            } => {
                let mut s = self.shared.write().await;
                let e = s.members.entry((guild_id, user.id)).or_insert(MemberUser {
                    user: user.clone(),
                    nick: None,
                    roles: Vec::new(),
                    joined_at: None,
                    communication_disabled_until: None,
                    mute: false,
                    deaf: false,
                });
                e.user = user;
                e.roles = roles;
                e.nick = nick;
            }
            Event::GuildMemberRemove { guild_id, user } => {
                self.shared
                    .write()
                    .await
                    .members
                    .remove(&(guild_id, user.id));
                if let Some(h) = &self.hooks {
                    h.dispatch(HookEvent::MemberLeave {
                        guild_id,
                        user: &user,
                    })
                    .await;
                }
            }
            Event::GuildRoleCreate { guild_id, role }
            | Event::GuildRoleUpdate { guild_id, role } => {
                let mut s = self.shared.write().await;
                if let Some(g) = s.guilds.get_mut(&guild_id) {
                    if let Some(r) = g.roles.iter_mut().find(|r| r.id == role.id) {
                        *r = role;
                    } else {
                        g.roles.push(role);
                    }
                }
            }
            Event::GuildRoleDelete { guild_id, role_id } => {
                let mut s = self.shared.write().await;
                if let Some(g) = s.guilds.get_mut(&guild_id) {
                    g.roles.retain(|r| r.id != role_id);
                }
            }
            Event::MessageCreate(m) => self.handle_message(m).await,
            Event::MessageUpdate(m) => {
                debug!(
                    "flush: message update {} (edits are left to external automod)",
                    m.id
                );
            }
            _ => {}
        }
        self.cooldowns.sweep();
    }

    async fn handle_message(&mut self, message: Message) {
        let (rest, bot_id) = {
            let s = self.shared.read().await;
            (s.rest.clone(), s.bot_id)
        };
        let Some(rest) = rest else {
            return;
        };
        if message.author.bot {
            return;
        }
        if let Some(id) = bot_id {
            if message.author.id == id {
                return;
            }
        }
        if !self.seen.insert(message.id) {
            return;
        }
        if let Some(h) = &self.hooks {
            h.dispatch(HookEvent::Message(&message)).await;
        }
        // Dedup cap (Sour uses a Set that never shrinks; this avoids unbounded growth).
        if self.seen.len() > 20_000 {
            self.seen.clear();
            self.seen.insert(message.id);
        }

        let guild_key = message.guild_id.map(|g| g.to_string());
        let stored = self.store.prefix(guild_key.as_deref());
        let prefix = if stored.is_empty() {
            self.config.default_prefix.clone()
        } else {
            stored
        };
        let Some(parsed) = parse_prefix_body(&message.content, &prefix, false) else {
            // Mention → reply with the prefix (mirrors Jully).
            if let Some(id) = bot_id {
                let mentioned = message.mentions.iter().any(|u| u.id == id);
                if mentioned {
                    let text = format!(
                        "👋 Hi **{}**! My prefix is `{prefix}`. Use `{prefix}help` to see my commands!",
                        message.author.username
                    );
                    let payload = flush_core::MessageSend::text(text);
                    let _ = rest
                        .reply(message.channel_id, &message, payload, true)
                        .await;
                }
            }
            return;
        };

        let Some(cmd) = crate::command::parsed_lookup(&parsed, &self.commands) else {
            return;
        };
        let cmd_name = cmd.name().to_owned();
        let cooldown = cmd.cooldown_ms().max(self.config.default_cooldown_ms);
        if cooldown > 0
            && self
                .cooldowns
                .on_cooldown(&message.author.id.to_string(), &cmd_name)
        {
            let remaining = self
                .cooldowns
                .remaining(&message.author.id.to_string(), &cmd_name)
                .unwrap_or(1);
            let payload = flush_core::MessageSend::text(format!(
                "⏱️ Wait **{remaining}s** before using this command again."
            ));
            let _ = rest
                .reply(message.channel_id, &message, payload, false)
                .await;
            return;
        }
        if cooldown > 0 {
            self.cooldowns
                .set(&message.author.id.to_string(), &cmd_name, cooldown);
        }
        info!(
            "flush: command {prefix}{cmd_name} by {} ({})",
            message.author.username, message.id
        );

        // Read snapshots for the handler (guild/channel/members from the cache).
        let s = self.shared.read().await;
        let guild = message.guild_id.and_then(|g| s.guilds.get(&g).cloned());
        let channel = s.channels.get(&message.channel_id).cloned();
        let member = message
            .guild_id
            .and_then(|g| s.members.get(&(g, message.author.id)).cloned());
        let bot_member = message
            .guild_id
            .and_then(|g| bot_id.and_then(|b| s.members.get(&(g, b)).cloned()));
        drop(s);
        let ctx = CommandCtx {
            message: &message,
            args: parsed.args.clone(),
            prefix: prefix.clone(),
            rest: &rest,
            bot_id,
            guild,
            channel,
            member,
            bot_member,
        };
        for mw in &self.middleware {
            if let crate::middleware::Flow::Stop(reply) = mw.before(&cmd_name, &ctx).await {
                if let Some(text) = reply {
                    let _ = ctx.reply_no_ping(text).await;
                }
                return;
            }
        }
        // Command permission gate (server owner passes; the bot needs the permission too).
        if let Some(action) = cmd.required_permission() {
            let Some(guild_ref) = ctx.guild.as_ref() else {
                let _ = ctx
                    .reply_no_ping(plain_error("This command only works in servers."))
                    .await;
                return;
            };
            let executor_is_owner = ctx
                .member
                .as_ref()
                .map(|m| guild_ref.owner_id == m.user.id)
                .unwrap_or(false);
            if let Some(err) = Moderation::check_member_with_owner(
                guild_ref,
                ctx.member.as_ref(),
                action,
                ctx.channel.as_ref(),
                executor_is_owner,
                "You need",
            ) {
                let _ = ctx.reply_no_ping(err).await;
                return;
            }
            let bot_is_owner = ctx
                .bot_member
                .as_ref()
                .map(|b| guild_ref.owner_id == b.user.id)
                .unwrap_or(false);
            if let Some(err) = Moderation::check_member_with_owner(
                guild_ref,
                ctx.bot_member.as_ref(),
                action,
                ctx.channel.as_ref(),
                bot_is_owner,
                "I need",
            ) {
                let _ = ctx.reply_no_ping(err).await;
                return;
            }
        }
        // `commands` is immutable while executing: resolve again to get the reference.
        let Some(cmd) = crate::command::parsed_lookup(&parsed, &self.commands) else {
            return;
        };
        if let Err(e) = cmd.execute(ctx).await {
            error!("flush: error in {cmd_name}: {e}");
            let text = match &e {
                CommandError::User(msg) => plain_error(msg),
                _ => plain_error("Something went wrong while running this command."),
            };
            let payload = flush_core::MessageSend::text(text);
            let _ = rest
                .reply(message.channel_id, &message, payload, false)
                .await;
        }
    }

    fn next_presence(&mut self) -> Option<Presence> {
        if self.config.presence_lines.is_empty() {
            return None;
        }
        self.presence_idx = (self.presence_idx + 1) % self.config.presence_lines.len();
        let line = self.config.presence_lines[self.presence_idx].clone();
        Some(Presence {
            status: flush_core::PresenceStatus::Online,
            activities: vec![flush_core::Activity::custom(line)],
            ..Default::default()
        })
    }

    async fn rotate_presence(&mut self, gw: &mut GatewayClient) {
        let Some(presence) = self.next_presence() else {
            return;
        };
        if let Err(e) = gw.set_presence(&presence).await {
            warn!("flush: presence update failed: {e}");
        }
    }
}
