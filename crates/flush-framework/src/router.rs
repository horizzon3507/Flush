//! Router: loop Gateway → parse → dispatch (espelho de
//! `Sour events/messageCreate.ts` + `Jully Bot.js::handleMessage` + `index.ts`).
//!
//! - `Bot::run`: discovery via `RestClient::new` → hello/identify/resume loop
//!   via `GatewayClient` → `MESSAGE_CREATE` → `handle_message`.
//! - Dedup por `message.id` (Sour `processingMessages`), ignora bots/self,
//!   prefixo por guild (`GuildStore`), aliases, cooldown com reply "Aguarde Ns",
//!   resposta de menção com o prefixo (Jully), presença rotativa 45s (Sour).

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
    /// Prefixo padrão (env `PREFIX` ou `!`); por guild prevalece o store.
    pub default_prefix: String,
    /// Origem para discovery (default `https://fluxer.app`).
    pub origin: Option<String>,
    /// Base/api/gateway explícitos (pulam discovery).
    pub api_base: Option<String>,
    pub gateway_url: Option<String>,
    /// Caminho do `guilds.json` (default `./data/guilds.json`).
    pub data_path: Option<std::path::PathBuf>,
    /// Presenças rotativas a cada 45s (espelho do Sour). Vazio = sem rotação.
    pub presence_lines: Vec<String>,
    /// Cooldown padrão por comando quando o comando não define (ms, 0 = sem).
    pub default_cooldown_ms: u64,
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

/// Evento repassado ao [`Hooks::dispatch`].
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
}

/// Hooks de eventos do cache do `Bot` (ready, guilds, membros, mensagens).
pub trait Hooks: Send + Sync {
    fn dispatch<'a>(&'a self, ev: HookEvent<'a>) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

/// Implementação vazia (para `hooks: None`).
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
}

impl Bot {
    pub fn new(config: BotConfig) -> Result<Self, CommandError> {
        if config.token.trim().is_empty() {
            return Err(CommandError::User("FLUXER_BOT_TOKEN não definido.".into()));
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
        })
    }

    /// Registra hooks de eventos (ready/guild/membros/mensagens).
    pub fn hooks(mut self, hooks: impl Hooks + 'static) -> Self {
        self.hooks = Some(Arc::new(hooks));
        self
    }

    /// Registra o comando `help` embutido, listando os demais comandos.
    /// O snapshot é sincronizado no `run()`.
    pub fn enable_help(mut self) -> Self {
        if self.help.is_none() {
            let h = Arc::new(crate::help::Help::new());
            self.help = Some(h.clone());
            self.commands.push(Box::new(crate::help::HelpCmd(h)));
        }
        self
    }

    /// Sincroniza o snapshot do `help` (chamado internamente no `run()`).
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

    /// Loop principal: REST (discovery) → Gateway → dispatch até erro fatal.
    pub async fn run(&mut self) -> Result<(), CommandError> {
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
            "flush: login como {} ({})",
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
        let mut gw =
            GatewayClient::new(gw_cfg).map_err(|e| CommandError::Internal(e.to_string()))?;
        self.refresh_help();
        loop {
            // Timeout de 45s = tick da presença rotativa (espelho do Sour).
            let next = gw.next_event();
            match async_std::future::timeout(std::time::Duration::from_secs(45), next).await {
                Ok(Ok(Some(ev))) => self.on_event(ev).await,
                Ok(Ok(None)) => {}
                Ok(Err(e)) => {
                    error!("flush: gateway erro: {e}");
                    return Err(CommandError::Internal(e.to_string()));
                }
                Err(_) => {
                    self.rotate_presence(&mut gw).await;
                }
            }
        }
    }

    async fn on_event(&mut self, ev: Event) {
        match ev {
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
                    "flush: message update {} (edit tratado pelo automod externo)",
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
        // Cap de dedup (Sour usa Set sem limpeza; aqui evitamos crescimento infinito).
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
            // Menção → responde o prefixo (espelho do Jully).
            if let Some(id) = bot_id {
                let mentioned = message.mentions.iter().any(|u| u.id == id);
                if mentioned {
                    let text = format!(
                        "👋 Olá **{}**! Meu prefixo é `{prefix}` — use `{prefix}help` para ver meus comandos!",
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
                "⏱️ Aguarde **{remaining}s** para usar este comando novamente."
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
            "flush: comando {prefix}{cmd_name} por {} ({})",
            message.author.username, message.id
        );

        // Snapshots de leitura para o handler (guild/canal/membros do cache).
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
        // Gate de permissão do comando (dono do servidor e bot com permissão).
        if let Some(action) = cmd.required_permission() {
            let Some(guild_ref) = ctx.guild.as_ref() else {
                let _ = ctx
                    .reply_no_ping(plain_error("Este comando só funciona em servidores."))
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
                "Você precisa",
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
                "Meu bot precisa",
            ) {
                let _ = ctx.reply_no_ping(err).await;
                return;
            }
        }
        // `commands` imutável durante execução: re-resolve para pegar a referência.
        let Some(cmd) = crate::command::parsed_lookup(&parsed, &self.commands) else {
            return;
        };
        if let Err(e) = cmd.execute(ctx).await {
            error!("flush: erro em {cmd_name}: {e}");
            let text = match &e {
                CommandError::User(msg) => plain_error(msg),
                _ => plain_error("Ocorreu um erro ao executar este comando."),
            };
            let payload = flush_core::MessageSend::text(text);
            let _ = rest
                .reply(message.channel_id, &message, payload, false)
                .await;
        }
    }

    async fn rotate_presence(&mut self, gw: &mut GatewayClient) {
        if self.config.presence_lines.is_empty() {
            return;
        }
        self.presence_idx = (self.presence_idx + 1) % self.config.presence_lines.len();
        let line = self.config.presence_lines[self.presence_idx].clone();
        let presence = Presence {
            status: flush_core::PresenceStatus::Online,
            activities: vec![flush_core::Activity::custom(line)],
            ..Default::default()
        };
        if let Err(e) = gw.set_presence(&presence).await {
            warn!("flush: presença falhou: {e}");
        }
    }
}
