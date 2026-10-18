use serde::{Deserialize, Serialize};

use crate::channel::Channel;
use crate::guild::{Guild, Role};
use crate::message::{Message, ReactionEmoji};
use crate::snowflake::Snowflake;
use crate::user::{PartialUser, User};

/// Envelope de Dispatch (op 0): `{ op: 0, t, s, d }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchEnvelope {
    #[serde(rename = "t")]
    pub kind: String,
    #[serde(rename = "s")]
    pub seq: u64,
    #[serde(rename = "d")]
    pub data: serde_json::Value,
}

/// Payload mínimo do READY (subset de bot: session_id + user + guilds indisponíveis).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadyPayload {
    pub session_id: String,
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default, deserialize_with = "crate::wire::null_vec")]
    pub guilds: Vec<GuildReady>,
    #[serde(default, deserialize_with = "crate::wire::null_vec")]
    pub users: Vec<PartialUser>,
}

/// Guild no READY de bot: `{ id, unavailable: true }` + burst de GUILD_CREATE depois.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildReady {
    pub id: Snowflake,
    #[serde(default, deserialize_with = "crate::wire::null_bool")]
    pub unavailable: bool,
}

/// Eventos tipados que o Flush decodifica (subset de paridade inicial).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Ready(ReadyPayload),
    Resumed {
        seq: u64,
    },
    GuildCreate(Guild),
    GuildUpdate(Guild),
    GuildDelete {
        id: Snowflake,
        unavailable: bool,
    },
    ChannelCreate(Channel),
    ChannelUpdate(Channel),
    ChannelDelete(Channel),
    MessageCreate(Message),
    MessageUpdate(Message),
    MessageDelete {
        id: Snowflake,
        channel_id: Snowflake,
        guild_id: Option<Snowflake>,
    },
    MessageDeleteBulk {
        ids: Vec<Snowflake>,
        channel_id: Snowflake,
        guild_id: Option<Snowflake>,
    },
    GuildMemberAdd {
        guild_id: Snowflake,
        member: crate::guild::MemberUser,
    },
    GuildMemberUpdate {
        guild_id: Snowflake,
        user: PartialUser,
        roles: Vec<Snowflake>,
        nick: Option<String>,
    },
    GuildMemberRemove {
        guild_id: Snowflake,
        user: PartialUser,
    },
    GuildRoleCreate {
        guild_id: Snowflake,
        role: Role,
    },
    GuildRoleUpdate {
        guild_id: Snowflake,
        role: Role,
    },
    GuildRoleDelete {
        guild_id: Snowflake,
        role_id: Snowflake,
    },
    GuildBanAdd {
        guild_id: Snowflake,
        user: PartialUser,
    },
    GuildBanRemove {
        guild_id: Snowflake,
        user: PartialUser,
    },
    MessageReactionAdd {
        channel_id: Snowflake,
        message_id: Snowflake,
        guild_id: Option<Snowflake>,
        user_id: Snowflake,
        emoji: ReactionEmoji,
    },
    MessageReactionRemove {
        channel_id: Snowflake,
        message_id: Snowflake,
        guild_id: Option<Snowflake>,
        user_id: Snowflake,
        emoji: ReactionEmoji,
    },
    TypingStart {
        channel_id: Snowflake,
        guild_id: Option<Snowflake>,
        user_id: Snowflake,
    },
    PresenceUpdate(PresencePayload),
    /// Evento ainda sem tipo dedicado: nome + payload bruto.
    Unknown {
        kind: String,
        seq: u64,
        data: serde_json::Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresencePayload {
    #[serde(default)]
    pub user: Option<PartialUser>,
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    #[serde(default)]
    pub status: Option<String>,
}

impl Event {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Ready(_) => "READY",
            Self::Resumed { .. } => "RESUMED",
            Self::GuildCreate(_) => "GUILD_CREATE",
            Self::GuildUpdate(_) => "GUILD_UPDATE",
            Self::GuildDelete { .. } => "GUILD_DELETE",
            Self::ChannelCreate(_) => "CHANNEL_CREATE",
            Self::ChannelUpdate(_) => "CHANNEL_UPDATE",
            Self::ChannelDelete(_) => "CHANNEL_DELETE",
            Self::MessageCreate(_) => "MESSAGE_CREATE",
            Self::MessageUpdate(_) => "MESSAGE_UPDATE",
            Self::MessageDelete { .. } => "MESSAGE_DELETE",
            Self::MessageDeleteBulk { .. } => "MESSAGE_DELETE_BULK",
            Self::GuildMemberAdd { .. } => "GUILD_MEMBER_ADD",
            Self::GuildMemberUpdate { .. } => "GUILD_MEMBER_UPDATE",
            Self::GuildMemberRemove { .. } => "GUILD_MEMBER_REMOVE",
            Self::GuildRoleCreate { .. } => "GUILD_ROLE_CREATE",
            Self::GuildRoleUpdate { .. } => "GUILD_ROLE_UPDATE",
            Self::GuildRoleDelete { .. } => "GUILD_ROLE_DELETE",
            Self::GuildBanAdd { .. } => "GUILD_BAN_ADD",
            Self::GuildBanRemove { .. } => "GUILD_BAN_REMOVE",
            Self::MessageReactionAdd { .. } => "MESSAGE_REACTION_ADD",
            Self::MessageReactionRemove { .. } => "MESSAGE_REACTION_REMOVE",
            Self::TypingStart { .. } => "TYPING_START",
            Self::PresenceUpdate(_) => "PRESENCE_UPDATE",
            Self::Unknown { .. } => "UNKNOWN",
        }
    }

    /// Decodifica um envelope já parseado para o tipo dedicado (ou Unknown).
    pub fn decode(env: &DispatchEnvelope) -> Self {
        fn de<T: for<'de> Deserialize<'de>>(v: &serde_json::Value) -> Option<T> {
            serde_json::from_value(v.clone()).ok()
        }
        match env.kind.as_str() {
            "READY" => de(&env.data).map(Event::Ready).unwrap_or(unknown(env)),
            "RESUMED" => Event::Resumed { seq: env.seq },
            "GUILD_CREATE" => de(&env.data)
                .map(Event::GuildCreate)
                .unwrap_or(unknown(env)),
            "GUILD_UPDATE" => de(&env.data)
                .map(Event::GuildUpdate)
                .unwrap_or(unknown(env)),
            "GUILD_DELETE" => {
                let id = env
                    .data
                    .get("id")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let unavailable = env
                    .data
                    .get("unavailable")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                id.map(|id| Event::GuildDelete { id, unavailable })
                    .unwrap_or(unknown(env))
            }
            "CHANNEL_CREATE" => de(&env.data)
                .map(Event::ChannelCreate)
                .unwrap_or(unknown(env)),
            "CHANNEL_UPDATE" => de(&env.data)
                .map(Event::ChannelUpdate)
                .unwrap_or(unknown(env)),
            "CHANNEL_DELETE" => de(&env.data)
                .map(Event::ChannelDelete)
                .unwrap_or(unknown(env)),
            "MESSAGE_CREATE" => de(&env.data)
                .map(Event::MessageCreate)
                .unwrap_or(unknown(env)),
            "MESSAGE_UPDATE" => de(&env.data)
                .map(Event::MessageUpdate)
                .unwrap_or(unknown(env)),
            "MESSAGE_DELETE" => {
                let id = env
                    .data
                    .get("id")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let channel_id = env
                    .data
                    .get("channel_id")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let guild_id = env
                    .data
                    .get("guild_id")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                match (id, channel_id) {
                    (Some(id), Some(channel_id)) => Event::MessageDelete {
                        id,
                        channel_id,
                        guild_id,
                    },
                    _ => unknown(env),
                }
            }
            "MESSAGE_DELETE_BULK" => {
                let ids = env
                    .data
                    .get("ids")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                let channel_id = env
                    .data
                    .get("channel_id")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let guild_id = env
                    .data
                    .get("guild_id")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                match channel_id {
                    Some(channel_id) => Event::MessageDeleteBulk {
                        ids,
                        channel_id,
                        guild_id,
                    },
                    None => unknown(env),
                }
            }
            "GUILD_MEMBER_ADD" => {
                let guild_id = env
                    .data
                    .get("guild_id")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let member = serde_json::from_value(env.data.clone()).ok();
                match (guild_id, member) {
                    (Some(guild_id), Some(member)) => Event::GuildMemberAdd { guild_id, member },
                    _ => unknown(env),
                }
            }
            "GUILD_MEMBER_UPDATE" => {
                let guild_id = env
                    .data
                    .get("guild_id")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let user = env
                    .data
                    .get("user")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let roles = env
                    .data
                    .get("roles")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                let nick = env
                    .data
                    .get("nick")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or(None);
                match (guild_id, user) {
                    (Some(guild_id), Some(user)) => Event::GuildMemberUpdate {
                        guild_id,
                        user,
                        roles,
                        nick,
                    },
                    _ => unknown(env),
                }
            }
            "GUILD_MEMBER_REMOVE" => {
                let guild_id = env
                    .data
                    .get("guild_id")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let user = env
                    .data
                    .get("user")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                match (guild_id, user) {
                    (Some(guild_id), Some(user)) => Event::GuildMemberRemove { guild_id, user },
                    _ => unknown(env),
                }
            }
            "GUILD_ROLE_CREATE" => {
                let guild_id = env
                    .data
                    .get("guild_id")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let role = env
                    .data
                    .get("role")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                match (guild_id, role) {
                    (Some(guild_id), Some(role)) => Event::GuildRoleCreate { guild_id, role },
                    _ => unknown(env),
                }
            }
            "GUILD_ROLE_UPDATE" => {
                let guild_id = env
                    .data
                    .get("guild_id")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let role = env
                    .data
                    .get("role")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                match (guild_id, role) {
                    (Some(guild_id), Some(role)) => Event::GuildRoleUpdate { guild_id, role },
                    _ => unknown(env),
                }
            }
            "GUILD_ROLE_DELETE" => {
                let guild_id = env
                    .data
                    .get("guild_id")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let role_id = env
                    .data
                    .get("role_id")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                match (guild_id, role_id) {
                    (Some(guild_id), Some(role_id)) => Event::GuildRoleDelete { guild_id, role_id },
                    _ => unknown(env),
                }
            }
            "GUILD_BAN_ADD" => {
                let guild_id = env
                    .data
                    .get("guild_id")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let user = env
                    .data
                    .get("user")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                match (guild_id, user) {
                    (Some(guild_id), Some(user)) => Event::GuildBanAdd { guild_id, user },
                    _ => unknown(env),
                }
            }
            "GUILD_BAN_REMOVE" => {
                let guild_id = env
                    .data
                    .get("guild_id")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let user = env
                    .data
                    .get("user")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                match (guild_id, user) {
                    (Some(guild_id), Some(user)) => Event::GuildBanRemove { guild_id, user },
                    _ => unknown(env),
                }
            }
            "MESSAGE_REACTION_ADD" => decode_reaction(env, true).unwrap_or(unknown(env)),
            "MESSAGE_REACTION_REMOVE" => decode_reaction(env, false).unwrap_or(unknown(env)),
            "TYPING_START" => {
                let channel_id = env
                    .data
                    .get("channel_id")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let guild_id = env
                    .data
                    .get("guild_id")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let user_id = env
                    .data
                    .get("user_id")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                match (channel_id, user_id) {
                    (Some(channel_id), Some(user_id)) => Event::TypingStart {
                        channel_id,
                        guild_id,
                        user_id,
                    },
                    _ => unknown(env),
                }
            }
            "PRESENCE_UPDATE" => de(&env.data)
                .map(Event::PresenceUpdate)
                .unwrap_or(unknown(env)),
            _ => unknown(env),
        }
    }
}

fn unknown(env: &DispatchEnvelope) -> Event {
    Event::Unknown {
        kind: env.kind.clone(),
        seq: env.seq,
        data: env.data.clone(),
    }
}

fn decode_reaction(env: &DispatchEnvelope, add: bool) -> Option<Event> {
    let channel_id = env
        .data
        .get("channel_id")
        .and_then(|v| serde_json::from_value(v.clone()).ok())?;
    let message_id = env
        .data
        .get("message_id")
        .and_then(|v| serde_json::from_value(v.clone()).ok())?;
    let guild_id = env
        .data
        .get("guild_id")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let user_id = env
        .data
        .get("user_id")
        .and_then(|v| serde_json::from_value(v.clone()).ok())?;
    let emoji = env
        .data
        .get("emoji")
        .and_then(|v| serde_json::from_value(v.clone()).ok())?;
    if add {
        Some(Event::MessageReactionAdd {
            channel_id,
            message_id,
            guild_id,
            user_id,
            emoji,
        })
    } else {
        Some(Event::MessageReactionRemove {
            channel_id,
            message_id,
            guild_id,
            user_id,
            emoji,
        })
    }
}
