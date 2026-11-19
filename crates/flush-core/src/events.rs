use serde::{Deserialize, Serialize};

use crate::audit_log::AuditLogEntry;
use crate::channel::Channel;
use crate::emoji::{Emoji, Sticker};
use crate::gateway_events::{
    AuthSessionChange, Call, ChannelMemberCount, EntranceSoundPlay, FavoriteMeme, ForumUnread,
    GatewaySession, GuildCount, GuildMemberListUpdate, GuildSync, PassiveUpdates, ReactionAddEntry,
    RecentMentionDelete, Relationship, ThreadMemberListEntry, UserConnection, UserGuildSettings,
    UserSettings, VoiceStateAck, WebAuthnCredential,
};
use crate::guild::{Guild, Role};
use crate::invite::Invite;
use crate::message::{Message, ReactionEmoji};
use crate::snowflake::Snowflake;
use crate::thread::ThreadMember;
use crate::user::{PartialUser, User};
use crate::voice::{VoiceServerUpdate, VoiceState};

/// Dispatch envelope (op 0): `{ op: 0, t, s, d }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchEnvelope {
    #[serde(rename = "t")]
    pub kind: String,
    #[serde(rename = "s")]
    pub seq: u64,
    #[serde(rename = "d")]
    pub data: serde_json::Value,
}

/// Minimal READY payload (bot subset: session_id + user + unavailable guilds).
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

/// Guild in a bot READY: `{ id, unavailable: true }`, followed by a GUILD_CREATE burst.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildReady {
    pub id: Snowflake,
    #[serde(default, deserialize_with = "crate::wire::null_bool")]
    pub unavailable: bool,
}

/// Typed events Flush decodes. See [`DISPATCH_EVENTS`] for every wire name.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
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
    UserUpdate(User),
    GuildMembersChunk {
        guild_id: Snowflake,
        members: Vec<crate::guild::MemberUser>,
        chunk_index: u32,
        chunk_count: u32,
        not_found: Vec<Snowflake>,
    },
    GuildRoleUpdateBulk {
        guild_id: Snowflake,
        roles: Vec<Role>,
    },
    GuildEmojisUpdate {
        guild_id: Snowflake,
        emojis: Vec<Emoji>,
    },
    GuildStickersUpdate {
        guild_id: Snowflake,
        stickers: Vec<Sticker>,
    },
    GuildAuditLogEntryCreate {
        guild_id: Option<Snowflake>,
        entry: AuditLogEntry,
    },
    ChannelUpdateBulk {
        channels: Vec<Channel>,
    },
    ChannelPinsUpdate {
        channel_id: Snowflake,
        guild_id: Option<Snowflake>,
        last_pin_timestamp: Option<String>,
    },
    ChannelRecipientAdd {
        channel_id: Snowflake,
        user: PartialUser,
    },
    ChannelRecipientRemove {
        channel_id: Snowflake,
        user: PartialUser,
    },
    ThreadCreate(Channel),
    ThreadUpdate(Channel),
    ThreadDelete {
        id: Snowflake,
        guild_id: Option<Snowflake>,
        parent_id: Option<Snowflake>,
    },
    ThreadListSync {
        guild_id: Snowflake,
        channel_ids: Vec<Snowflake>,
        threads: Vec<Channel>,
        members: Vec<ThreadMember>,
    },
    ThreadMemberUpdate {
        guild_id: Option<Snowflake>,
        member: ThreadMember,
    },
    ThreadMembersUpdate {
        id: Snowflake,
        guild_id: Option<Snowflake>,
        member_count: u64,
        added_members: Vec<ThreadMember>,
        removed_member_ids: Vec<Snowflake>,
    },
    InviteCreate {
        guild_id: Option<Snowflake>,
        channel_id: Option<Snowflake>,
        invite: Invite,
    },
    InviteDelete {
        code: String,
        channel_id: Option<Snowflake>,
        guild_id: Option<Snowflake>,
    },
    MessageReactionRemoveAll {
        channel_id: Snowflake,
        message_id: Snowflake,
        guild_id: Option<Snowflake>,
    },
    MessageReactionRemoveEmoji {
        channel_id: Snowflake,
        message_id: Snowflake,
        guild_id: Option<Snowflake>,
        emoji: ReactionEmoji,
    },
    WebhooksUpdate {
        guild_id: Option<Snowflake>,
        channel_id: Option<Snowflake>,
    },
    VoiceStateUpdate(VoiceState),
    VoiceServerUpdate(VoiceServerUpdate),
    AuthSessionChange(AuthSessionChange),
    CallCreate(Call),
    CallUpdate(Call),
    CallDelete {
        id: Snowflake,
        channel_id: Snowflake,
    },
    ChannelMemberCountsUpdate {
        counts: Vec<ChannelMemberCount>,
    },
    ChannelPinsAck {
        channel_id: Snowflake,
        last_pin_timestamp: Option<String>,
    },
    EntranceSoundPlay(EntranceSoundPlay),
    FavoriteMemeCreate(FavoriteMeme),
    FavoriteMemeUpdate(FavoriteMeme),
    FavoriteMemeDelete(FavoriteMeme),
    ForumUnreads {
        guild_id: Snowflake,
        channel_id: Snowflake,
        threads: Vec<ForumUnread>,
    },
    GuildCountsUpdate {
        counts: Vec<GuildCount>,
    },
    GuildMemberListUpdate(GuildMemberListUpdate),
    GuildSync(GuildSync),
    MessageAck {
        channel_id: Snowflake,
        message_id: Snowflake,
    },
    MessageReactionAddMany {
        channel_id: Snowflake,
        message_id: Snowflake,
        guild_id: Option<Snowflake>,
        reactions: Vec<ReactionAddEntry>,
    },
    PassiveUpdates(PassiveUpdates),
    PresenceUpdateBulk {
        guild_id: Option<Snowflake>,
        presences: Vec<PresencePayload>,
    },
    RecentMentionDelete(RecentMentionDelete),
    RelationshipAdd(Relationship),
    RelationshipUpdate(Relationship),
    RelationshipRemove {
        id: Snowflake,
    },
    SavedMessageCreate(Message),
    SavedMessageDelete {
        id: Snowflake,
    },
    SessionsReplace {
        sessions: Vec<GatewaySession>,
    },
    ThreadMemberListUpdate {
        guild_id: Snowflake,
        thread_id: Snowflake,
        members: Vec<ThreadMemberListEntry>,
    },
    UserConnectionsUpdate {
        connections: Vec<UserConnection>,
    },
    UserGuildSettingsUpdate(UserGuildSettings),
    UserNoteUpdate {
        id: Snowflake,
        note: Option<String>,
    },
    UserPinnedDmsUpdate {
        pinned_channels: Vec<Snowflake>,
    },
    UserSettingsUpdate(UserSettings),
    VoiceStateAck(VoiceStateAck),
    WebAuthnCredentialsUpdate(Vec<WebAuthnCredential>),
    /// Unrecognized name, or a payload that did not match its type: name + raw payload.
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
            Self::UserUpdate(_) => "USER_UPDATE",
            Self::GuildMembersChunk { .. } => "GUILD_MEMBERS_CHUNK",
            Self::GuildRoleUpdateBulk { .. } => "GUILD_ROLE_UPDATE_BULK",
            Self::GuildEmojisUpdate { .. } => "GUILD_EMOJIS_UPDATE",
            Self::GuildStickersUpdate { .. } => "GUILD_STICKERS_UPDATE",
            Self::GuildAuditLogEntryCreate { .. } => "GUILD_AUDIT_LOG_ENTRY_CREATE",
            Self::ChannelUpdateBulk { .. } => "CHANNEL_UPDATE_BULK",
            Self::ChannelPinsUpdate { .. } => "CHANNEL_PINS_UPDATE",
            Self::ChannelRecipientAdd { .. } => "CHANNEL_RECIPIENT_ADD",
            Self::ChannelRecipientRemove { .. } => "CHANNEL_RECIPIENT_REMOVE",
            Self::ThreadCreate(_) => "THREAD_CREATE",
            Self::ThreadUpdate(_) => "THREAD_UPDATE",
            Self::ThreadDelete { .. } => "THREAD_DELETE",
            Self::ThreadListSync { .. } => "THREAD_LIST_SYNC",
            Self::ThreadMemberUpdate { .. } => "THREAD_MEMBER_UPDATE",
            Self::ThreadMembersUpdate { .. } => "THREAD_MEMBERS_UPDATE",
            Self::InviteCreate { .. } => "INVITE_CREATE",
            Self::InviteDelete { .. } => "INVITE_DELETE",
            Self::MessageReactionRemoveAll { .. } => "MESSAGE_REACTION_REMOVE_ALL",
            Self::MessageReactionRemoveEmoji { .. } => "MESSAGE_REACTION_REMOVE_EMOJI",
            Self::WebhooksUpdate { .. } => "WEBHOOKS_UPDATE",
            Self::VoiceStateUpdate(_) => "VOICE_STATE_UPDATE",
            Self::VoiceServerUpdate(_) => "VOICE_SERVER_UPDATE",
            Self::AuthSessionChange(_) => "AUTH_SESSION_CHANGE",
            Self::CallCreate(_) => "CALL_CREATE",
            Self::CallUpdate(_) => "CALL_UPDATE",
            Self::EntranceSoundPlay(_) => "ENTRANCE_SOUND_PLAY",
            Self::FavoriteMemeCreate(_) => "FAVORITE_MEME_CREATE",
            Self::FavoriteMemeUpdate(_) => "FAVORITE_MEME_UPDATE",
            Self::FavoriteMemeDelete(_) => "FAVORITE_MEME_DELETE",
            Self::GuildMemberListUpdate(_) => "GUILD_MEMBER_LIST_UPDATE",
            Self::GuildSync(_) => "GUILD_SYNC",
            Self::PassiveUpdates(_) => "PASSIVE_UPDATES",
            Self::RecentMentionDelete(_) => "RECENT_MENTION_DELETE",
            Self::RelationshipAdd(_) => "RELATIONSHIP_ADD",
            Self::RelationshipUpdate(_) => "RELATIONSHIP_UPDATE",
            Self::SavedMessageCreate(_) => "SAVED_MESSAGE_CREATE",
            Self::UserGuildSettingsUpdate(_) => "USER_GUILD_SETTINGS_UPDATE",
            Self::UserSettingsUpdate(_) => "USER_SETTINGS_UPDATE",
            Self::VoiceStateAck(_) => "VOICE_STATE_ACK",
            Self::WebAuthnCredentialsUpdate(_) => "WEBAUTHN_CREDENTIALS_UPDATE",
            Self::CallDelete { .. } => "CALL_DELETE",
            Self::ChannelMemberCountsUpdate { .. } => "CHANNEL_MEMBER_COUNTS_UPDATE",
            Self::ChannelPinsAck { .. } => "CHANNEL_PINS_ACK",
            Self::ForumUnreads { .. } => "FORUM_UNREADS",
            Self::GuildCountsUpdate { .. } => "GUILD_COUNTS_UPDATE",
            Self::MessageAck { .. } => "MESSAGE_ACK",
            Self::MessageReactionAddMany { .. } => "MESSAGE_REACTION_ADD_MANY",
            Self::PresenceUpdateBulk { .. } => "PRESENCE_UPDATE_BULK",
            Self::RelationshipRemove { .. } => "RELATIONSHIP_REMOVE",
            Self::SavedMessageDelete { .. } => "SAVED_MESSAGE_DELETE",
            Self::SessionsReplace { .. } => "SESSIONS_REPLACE",
            Self::ThreadMemberListUpdate { .. } => "THREAD_MEMBER_LIST_UPDATE",
            Self::UserConnectionsUpdate { .. } => "USER_CONNECTIONS_UPDATE",
            Self::UserNoteUpdate { .. } => "USER_NOTE_UPDATE",
            Self::UserPinnedDmsUpdate { .. } => "USER_PINNED_DMS_UPDATE",
            Self::Unknown { .. } => "UNKNOWN",
        }
    }

    /// Decodes a parsed envelope into its dedicated type (or `Unknown`).
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
            _ => decode_extended(env).unwrap_or_else(|| unknown(env)),
        }
    }

    /// Wire name for [`Event::Unknown`], otherwise the same as [`Event::kind`].
    pub fn wire_name(&self) -> &str {
        match self {
            Self::Unknown { kind, .. } => kind,
            other => other.kind(),
        }
    }
}

/// Every dispatch name in `GatewayDispatchEvents` of `@fluxerjs/types` 3.3.0.
/// Every name has a typed variant; a payload that does not match its type
/// arrives as [`Event::Unknown`] with the raw payload.
pub const DISPATCH_EVENTS: &[&str] = &[
    "READY",
    "RESUMED",
    "SESSIONS_REPLACE",
    "USER_UPDATE",
    "USER_SETTINGS_UPDATE",
    "USER_GUILD_SETTINGS_UPDATE",
    "USER_CONNECTIONS_UPDATE",
    "WEBAUTHN_CREDENTIALS_UPDATE",
    "USER_PINNED_DMS_UPDATE",
    "USER_NOTE_UPDATE",
    "RECENT_MENTION_DELETE",
    "SAVED_MESSAGE_CREATE",
    "SAVED_MESSAGE_DELETE",
    "AUTH_SESSION_CHANGE",
    "PRESENCE_UPDATE",
    "PRESENCE_UPDATE_BULK",
    "GUILD_CREATE",
    "GUILD_UPDATE",
    "GUILD_DELETE",
    "GUILD_MEMBER_ADD",
    "GUILD_MEMBER_UPDATE",
    "GUILD_MEMBER_REMOVE",
    "GUILD_MEMBERS_CHUNK",
    "GUILD_MEMBER_LIST_UPDATE",
    "GUILD_SYNC",
    "GUILD_COUNTS_UPDATE",
    "CHANNEL_MEMBER_COUNTS_UPDATE",
    "GUILD_ROLE_CREATE",
    "GUILD_ROLE_UPDATE",
    "GUILD_ROLE_UPDATE_BULK",
    "GUILD_ROLE_DELETE",
    "GUILD_EMOJIS_UPDATE",
    "GUILD_STICKERS_UPDATE",
    "GUILD_BAN_ADD",
    "GUILD_BAN_REMOVE",
    "GUILD_AUDIT_LOG_ENTRY_CREATE",
    "CHANNEL_CREATE",
    "CHANNEL_UPDATE",
    "CHANNEL_UPDATE_BULK",
    "CHANNEL_DELETE",
    "CHANNEL_RECIPIENT_ADD",
    "CHANNEL_RECIPIENT_REMOVE",
    "CHANNEL_PINS_UPDATE",
    "CHANNEL_PINS_ACK",
    "THREAD_CREATE",
    "THREAD_UPDATE",
    "THREAD_DELETE",
    "THREAD_LIST_SYNC",
    "THREAD_MEMBER_UPDATE",
    "THREAD_MEMBERS_UPDATE",
    "THREAD_MEMBER_LIST_UPDATE",
    "FORUM_UNREADS",
    "PASSIVE_UPDATES",
    "INVITE_CREATE",
    "INVITE_DELETE",
    "MESSAGE_CREATE",
    "MESSAGE_UPDATE",
    "MESSAGE_DELETE",
    "MESSAGE_DELETE_BULK",
    "MESSAGE_REACTION_ADD",
    "MESSAGE_REACTION_ADD_MANY",
    "MESSAGE_REACTION_REMOVE",
    "MESSAGE_REACTION_REMOVE_ALL",
    "MESSAGE_REACTION_REMOVE_EMOJI",
    "MESSAGE_ACK",
    "TYPING_START",
    "WEBHOOKS_UPDATE",
    "RELATIONSHIP_ADD",
    "RELATIONSHIP_UPDATE",
    "RELATIONSHIP_REMOVE",
    "VOICE_STATE_UPDATE",
    "VOICE_STATE_ACK",
    "VOICE_SERVER_UPDATE",
    "ENTRANCE_SOUND_PLAY",
    "CALL_CREATE",
    "CALL_UPDATE",
    "CALL_DELETE",
    "FAVORITE_MEME_CREATE",
    "FAVORITE_MEME_UPDATE",
    "FAVORITE_MEME_DELETE",
];

/// Whether `name` is a dispatch event the SDK knows about.
pub fn is_known_dispatch(name: &str) -> bool {
    DISPATCH_EVENTS.contains(&name)
}

fn field<T: for<'de> Deserialize<'de>>(data: &serde_json::Value, key: &str) -> Option<T> {
    data.get(key)
        .filter(|v| !v.is_null())
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

fn list<T: for<'de> Deserialize<'de>>(data: &serde_json::Value, key: &str) -> Vec<T> {
    field(data, key).unwrap_or_default()
}

/// Missing or `null` is an empty list; anything else must decode in full.
fn strict_list<T: for<'de> Deserialize<'de>>(
    data: &serde_json::Value,
    key: &str,
) -> Option<Vec<T>> {
    match data.get(key) {
        None | Some(serde_json::Value::Null) => Some(Vec::new()),
        Some(v) => serde_json::from_value(v.clone()).ok(),
    }
}

fn whole<T: for<'de> Deserialize<'de>>(data: &serde_json::Value) -> Option<T> {
    serde_json::from_value(data.clone()).ok()
}

fn decode_extended(env: &DispatchEnvelope) -> Option<Event> {
    let d = &env.data;
    Some(match env.kind.as_str() {
        "USER_UPDATE" => Event::UserUpdate(whole(d)?),
        "GUILD_MEMBERS_CHUNK" => Event::GuildMembersChunk {
            guild_id: field(d, "guild_id")?,
            members: list(d, "members"),
            chunk_index: field(d, "chunk_index").unwrap_or(0),
            chunk_count: field(d, "chunk_count").unwrap_or(1),
            not_found: list(d, "not_found"),
        },
        "GUILD_ROLE_UPDATE_BULK" => Event::GuildRoleUpdateBulk {
            guild_id: field(d, "guild_id")?,
            roles: list(d, "roles"),
        },
        "GUILD_EMOJIS_UPDATE" => Event::GuildEmojisUpdate {
            guild_id: field(d, "guild_id")?,
            emojis: list(d, "emojis"),
        },
        "GUILD_STICKERS_UPDATE" => Event::GuildStickersUpdate {
            guild_id: field(d, "guild_id")?,
            stickers: list(d, "stickers"),
        },
        "GUILD_AUDIT_LOG_ENTRY_CREATE" => Event::GuildAuditLogEntryCreate {
            guild_id: field(d, "guild_id"),
            entry: whole(d)?,
        },
        "CHANNEL_UPDATE_BULK" => Event::ChannelUpdateBulk {
            channels: list(d, "channels"),
        },
        "CHANNEL_PINS_UPDATE" => Event::ChannelPinsUpdate {
            channel_id: field(d, "channel_id")?,
            guild_id: field(d, "guild_id"),
            last_pin_timestamp: field(d, "last_pin_timestamp"),
        },
        "CHANNEL_RECIPIENT_ADD" => Event::ChannelRecipientAdd {
            channel_id: field(d, "channel_id")?,
            user: field(d, "user")?,
        },
        "CHANNEL_RECIPIENT_REMOVE" => Event::ChannelRecipientRemove {
            channel_id: field(d, "channel_id")?,
            user: field(d, "user")?,
        },
        "THREAD_CREATE" => Event::ThreadCreate(whole(d)?),
        "THREAD_UPDATE" => Event::ThreadUpdate(whole(d)?),
        "THREAD_DELETE" => Event::ThreadDelete {
            id: field(d, "id")?,
            guild_id: field(d, "guild_id"),
            parent_id: field(d, "parent_id"),
        },
        "THREAD_LIST_SYNC" => Event::ThreadListSync {
            guild_id: field(d, "guild_id")?,
            channel_ids: list(d, "channel_ids"),
            threads: list(d, "threads"),
            members: list(d, "members"),
        },
        "THREAD_MEMBER_UPDATE" => Event::ThreadMemberUpdate {
            guild_id: field(d, "guild_id"),
            member: whole(d)?,
        },
        "THREAD_MEMBERS_UPDATE" => Event::ThreadMembersUpdate {
            id: field(d, "id")?,
            guild_id: field(d, "guild_id"),
            member_count: field(d, "member_count").unwrap_or(0),
            added_members: list(d, "added_members"),
            removed_member_ids: list(d, "removed_member_ids"),
        },
        "INVITE_CREATE" => Event::InviteCreate {
            guild_id: field(d, "guild_id"),
            channel_id: field(d, "channel_id"),
            invite: whole(d)?,
        },
        "INVITE_DELETE" => Event::InviteDelete {
            code: field(d, "code")?,
            channel_id: field(d, "channel_id"),
            guild_id: field(d, "guild_id"),
        },
        "MESSAGE_REACTION_REMOVE_ALL" => Event::MessageReactionRemoveAll {
            channel_id: field(d, "channel_id")?,
            message_id: field(d, "message_id")?,
            guild_id: field(d, "guild_id"),
        },
        "MESSAGE_REACTION_REMOVE_EMOJI" => Event::MessageReactionRemoveEmoji {
            channel_id: field(d, "channel_id")?,
            message_id: field(d, "message_id")?,
            guild_id: field(d, "guild_id"),
            emoji: field(d, "emoji")?,
        },
        "WEBHOOKS_UPDATE" => Event::WebhooksUpdate {
            guild_id: field(d, "guild_id"),
            channel_id: field(d, "channel_id"),
        },
        "VOICE_STATE_UPDATE" => Event::VoiceStateUpdate(whole(d)?),
        "VOICE_SERVER_UPDATE" => Event::VoiceServerUpdate(whole(d)?),
        "AUTH_SESSION_CHANGE" => Event::AuthSessionChange(whole(d)?),
        "CALL_CREATE" => Event::CallCreate(whole(d)?),
        "CALL_UPDATE" => Event::CallUpdate(whole(d)?),
        "CALL_DELETE" => Event::CallDelete {
            id: field(d, "id")?,
            channel_id: field(d, "channel_id")?,
        },
        "CHANNEL_MEMBER_COUNTS_UPDATE" => Event::ChannelMemberCountsUpdate {
            counts: strict_list(d, "counts")?,
        },
        "CHANNEL_PINS_ACK" => Event::ChannelPinsAck {
            channel_id: field(d, "channel_id")?,
            last_pin_timestamp: field(d, "last_pin_timestamp"),
        },
        "ENTRANCE_SOUND_PLAY" => Event::EntranceSoundPlay(whole(d)?),
        "FAVORITE_MEME_CREATE" => Event::FavoriteMemeCreate(whole(d)?),
        "FAVORITE_MEME_UPDATE" => Event::FavoriteMemeUpdate(whole(d)?),
        "FAVORITE_MEME_DELETE" => Event::FavoriteMemeDelete(whole(d)?),
        "FORUM_UNREADS" => Event::ForumUnreads {
            guild_id: field(d, "guild_id")?,
            channel_id: field(d, "channel_id")?,
            threads: strict_list(d, "threads")?,
        },
        "GUILD_COUNTS_UPDATE" => Event::GuildCountsUpdate {
            counts: strict_list(d, "counts")?,
        },
        "GUILD_MEMBER_LIST_UPDATE" => Event::GuildMemberListUpdate(whole(d)?),
        "GUILD_SYNC" => Event::GuildSync(whole(d)?),
        "MESSAGE_ACK" => Event::MessageAck {
            channel_id: field(d, "channel_id")?,
            message_id: field(d, "message_id")?,
        },
        "MESSAGE_REACTION_ADD_MANY" => Event::MessageReactionAddMany {
            channel_id: field(d, "channel_id")?,
            message_id: field(d, "message_id")?,
            guild_id: field(d, "guild_id"),
            reactions: strict_list(d, "reactions")?,
        },
        "PASSIVE_UPDATES" => Event::PassiveUpdates(whole(d)?),
        "PRESENCE_UPDATE_BULK" => Event::PresenceUpdateBulk {
            guild_id: field(d, "guild_id"),
            presences: strict_list(d, "presences")?,
        },
        "RECENT_MENTION_DELETE" => Event::RecentMentionDelete(whole(d)?),
        "RELATIONSHIP_ADD" => Event::RelationshipAdd(whole(d)?),
        "RELATIONSHIP_UPDATE" => Event::RelationshipUpdate(whole(d)?),
        "RELATIONSHIP_REMOVE" => Event::RelationshipRemove {
            id: field(d, "id")?,
        },
        "SAVED_MESSAGE_CREATE" => Event::SavedMessageCreate(whole(d)?),
        "SAVED_MESSAGE_DELETE" => Event::SavedMessageDelete {
            id: field(d, "id")?,
        },
        "SESSIONS_REPLACE" => Event::SessionsReplace {
            sessions: match d {
                serde_json::Value::Array(_) => whole(d)?,
                _ => strict_list(d, "sessions")?,
            },
        },
        "THREAD_MEMBER_LIST_UPDATE" => Event::ThreadMemberListUpdate {
            guild_id: field(d, "guild_id")?,
            thread_id: field(d, "thread_id")?,
            members: strict_list(d, "members")?,
        },
        "USER_CONNECTIONS_UPDATE" => Event::UserConnectionsUpdate {
            connections: strict_list(d, "connections")?,
        },
        "USER_GUILD_SETTINGS_UPDATE" => Event::UserGuildSettingsUpdate(whole(d)?),
        "USER_NOTE_UPDATE" => Event::UserNoteUpdate {
            id: field(d, "id")?,
            note: field(d, "note"),
        },
        "USER_PINNED_DMS_UPDATE" => Event::UserPinnedDmsUpdate {
            pinned_channels: strict_list(d, "pinned_channels")?,
        },
        "USER_SETTINGS_UPDATE" => Event::UserSettingsUpdate(whole(d)?),
        "VOICE_STATE_ACK" => Event::VoiceStateAck(whole(d)?),
        "WEBAUTHN_CREDENTIALS_UPDATE" => {
            Event::WebAuthnCredentialsUpdate(if d.is_null() { Vec::new() } else { whole(d)? })
        }
        _ => return None,
    })
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
