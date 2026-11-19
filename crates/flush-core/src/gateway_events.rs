//! Payloads of the session, read-state and bulk dispatch events.
//!
//! Shapes follow the `Gateway*DispatchData` interfaces of `@fluxerjs/types`
//! 3.3.0. Where the SDK types a payload as an open map, the known fields are
//! typed and the remainder is kept in `extra`.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::guild::MemberUser;
use crate::message::ReactionEmoji;
use crate::snowflake::Snowflake;
use crate::voice::VoiceState;
use crate::wire::{null_u32, null_u64, null_vec};

/// Relationship kind, an integer on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum RelationshipType {
    Friend = 1,
    Blocked = 2,
    IncomingRequest = 3,
    OutgoingRequest = 4,
    #[default]
    Unknown = 255,
}

impl Serialize for RelationshipType {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u8(*self as u8)
    }
}

impl<'de> Deserialize<'de> for RelationshipType {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match u64::deserialize(d)? {
            1 => Self::Friend,
            2 => Self::Blocked,
            3 => Self::IncomingRequest,
            4 => Self::OutgoingRequest,
            _ => Self::Unknown,
        })
    }
}

/// `RELATIONSHIP_ADD` / `RELATIONSHIP_UPDATE`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relationship {
    pub id: Snowflake,
    #[serde(default, rename = "type")]
    pub kind: RelationshipType,
}

/// `CALL_CREATE` / `CALL_UPDATE`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    pub id: Snowflake,
    pub channel_id: Snowflake,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `FAVORITE_MEME_CREATE` / `_UPDATE` / `_DELETE`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FavoriteMeme {
    #[serde(default)]
    pub id: Option<Snowflake>,
    #[serde(default)]
    pub user_id: Option<Snowflake>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `AUTH_SESSION_CHANGE`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthSessionChange {
    #[serde(default)]
    pub session_id: Option<String>,
    /// `login`, `logout`, `update` or a future value.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `PASSIVE_UPDATES`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PassiveUpdates {
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `GUILD_SYNC`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildSync {
    #[serde(default)]
    pub id: Option<Snowflake>,
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `RECENT_MENTION_DELETE`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentMentionDelete {
    #[serde(default)]
    pub id: Option<Snowflake>,
    #[serde(default)]
    pub channel_id: Option<Snowflake>,
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildCount {
    pub guild_id: Snowflake,
    #[serde(default, deserialize_with = "null_u64")]
    pub member_count: u64,
    #[serde(default, deserialize_with = "null_u64")]
    pub online_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelMemberCount {
    pub guild_id: Snowflake,
    pub channel_id: Snowflake,
    #[serde(default, deserialize_with = "null_u64")]
    pub member_count: u64,
    #[serde(default, deserialize_with = "null_u64")]
    pub online_count: u64,
}

/// `ENTRANCE_SOUND_PLAY`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntranceSoundPlay {
    pub user_id: Snowflake,
    pub channel_id: Snowflake,
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    pub sound_id: Snowflake,
    #[serde(default)]
    pub hash: String,
    #[serde(default)]
    pub url: String,
    #[serde(default, deserialize_with = "null_u64")]
    pub duration_ms: u64,
    #[serde(default)]
    pub content_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForumUnread {
    pub thread_id: Snowflake,
    #[serde(default)]
    pub count: Option<u64>,
    #[serde(default)]
    pub missing: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberListGroup {
    pub id: String,
    #[serde(default, deserialize_with = "null_u64")]
    pub count: u64,
}

/// One operation of `GUILD_MEMBER_LIST_UPDATE`. `op` is `SYNC`, `INVALIDATE`,
/// `INSERT`, `UPDATE` or `DELETE`; items are open maps in the SDK.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberListOp {
    pub op: String,
    #[serde(default)]
    pub range: Option<(u64, u64)>,
    #[serde(default)]
    pub index: Option<u64>,
    #[serde(default)]
    pub item: Option<Value>,
    #[serde(default, deserialize_with = "null_vec")]
    pub items: Vec<Value>,
}

/// `GUILD_MEMBER_LIST_UPDATE`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildMemberListUpdate {
    pub guild_id: Snowflake,
    /// Member list ID (not a snowflake).
    #[serde(default)]
    pub id: String,
    #[serde(default, deserialize_with = "null_u64")]
    pub member_count: u64,
    #[serde(default, deserialize_with = "null_u64")]
    pub online_count: u64,
    #[serde(default, deserialize_with = "null_vec")]
    pub groups: Vec<MemberListGroup>,
    #[serde(default, deserialize_with = "null_vec")]
    pub ops: Vec<MemberListOp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMemberListEntry {
    pub user_id: Snowflake,
    #[serde(default)]
    pub join_timestamp: Option<String>,
    #[serde(default, deserialize_with = "null_u64")]
    pub flags: u64,
    #[serde(default)]
    pub member: Option<MemberUser>,
    #[serde(default)]
    pub presence: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionAddEntry {
    pub user_id: Snowflake,
    pub emoji: ReactionEmoji,
    /// Member snapshot (open map in the SDK).
    #[serde(default)]
    pub member: Option<Value>,
}

/// One gateway session of `SESSIONS_REPLACE`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewaySession {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "null_vec")]
    pub activities: Vec<Value>,
    #[serde(default)]
    pub client_info: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomStatus {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub emoji_name: Option<String>,
    #[serde(default)]
    pub emoji_id: Option<String>,
}

/// `USER_SETTINGS_UPDATE`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSettings {
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub custom_status: Option<CustomStatus>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelOverride {
    pub channel_id: Snowflake,
    #[serde(default)]
    pub muted: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `USER_GUILD_SETTINGS_UPDATE`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserGuildSettings {
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    #[serde(default, deserialize_with = "null_vec")]
    pub channel_overrides: Vec<ChannelOverride>,
    #[serde(default)]
    pub muted: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserConnection {
    pub id: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub verified: bool,
    #[serde(default, deserialize_with = "null_u64")]
    pub visibility_flags: u64,
    #[serde(default, deserialize_with = "null_u32")]
    pub sort_order: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebAuthnCredential {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub last_used_at: Option<String>,
}

/// `VOICE_STATE_ACK`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceStateAck {
    #[serde(default)]
    pub mutation_id: Option<String>,
    #[serde(default)]
    pub runtime_epoch: Option<String>,
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    #[serde(default)]
    pub channel_id: Option<Snowflake>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub server_version: Option<u64>,
    #[serde(default)]
    pub canonical_state: Option<VoiceState>,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub error_message: Option<String>,
}
