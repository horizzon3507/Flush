use serde::{Deserialize, Deserializer, Serialize};

use crate::permissions::wire as perm_wire;
use crate::snowflake::Snowflake;
use crate::user::PartialUser;
use crate::wire::{null_bool, null_vec};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    pub id: Snowflake,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "crate::wire::null_u32")]
    pub color: u32,
    #[serde(default, deserialize_with = "crate::wire::null_i32")]
    pub position: i32,
    #[serde(default)]
    pub hoist_position: Option<i32>,
    #[serde(with = "perm_wire")]
    pub permissions: u64,
    #[serde(default, deserialize_with = "null_bool")]
    pub hoist: bool,
    #[serde(default, deserialize_with = "null_bool")]
    pub mentionable: bool,
}

impl Role {
    pub fn mention(&self) -> String {
        format!("<@&{}>", self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberUser {
    pub user: PartialUser,
    #[serde(default)]
    pub nick: Option<String>,
    #[serde(default, deserialize_with = "null_vec")]
    pub roles: Vec<Snowflake>,
    #[serde(default)]
    pub joined_at: Option<String>,
    #[serde(default)]
    pub communication_disabled_until: Option<String>,
    #[serde(default, deserialize_with = "null_bool")]
    pub mute: bool,
    #[serde(default, deserialize_with = "null_bool")]
    pub deaf: bool,
}

/// Alias: guild member.
pub type GuildMember = MemberUser;

impl MemberUser {
    pub fn display_name(&self) -> &str {
        self.nick
            .as_deref()
            .unwrap_or_else(|| self.user.display_name())
    }

    pub fn timed_out(&self) -> bool {
        // Lexicographic comparison works for RFC 3339 UTC.
        self.communication_disabled_until
            .as_deref()
            .map(|s| !s.is_empty() && s > "1970-01-01T00:00:00Z")
            .unwrap_or(false)
    }
}

/// Guild snapshot: the Gateway nests metadata under `properties` or sends it flat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum GuildSnapshot {
    Nested(GuildNested),
    Flat(GuildFlat),
}

impl<'de> Deserialize<'de> for GuildSnapshot {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // `properties` means the payload is nested. A malformed nested
        // section must fail outright instead of degrading to `Flat`, which
        // would silently drop the metadata inside `properties`.
        let value = serde_json::Value::deserialize(d)?;
        if value.get("properties").is_some() {
            GuildNested::deserialize(value)
                .map(Self::Nested)
                .map_err(serde::de::Error::custom)
        } else {
            GuildFlat::deserialize(value)
                .map(Self::Flat)
                .map_err(serde::de::Error::custom)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildNested {
    pub id: Snowflake,
    pub properties: GuildFlat,
    #[serde(default, deserialize_with = "null_vec")]
    pub roles: Vec<Role>,
    #[serde(default, deserialize_with = "null_vec")]
    pub channels: Vec<crate::channel::Channel>,
    #[serde(default, deserialize_with = "null_vec")]
    pub members: Vec<MemberUser>,
    #[serde(default, deserialize_with = "null_vec")]
    pub voice_states: Vec<crate::voice::VoiceState>,
    #[serde(default)]
    pub member_count: Option<u64>,
    #[serde(default)]
    pub online_count: Option<u64>,
    #[serde(default)]
    pub unavailable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildFlat {
    pub id: Snowflake,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub owner_id: Option<Snowflake>,
    #[serde(default, deserialize_with = "null_vec")]
    pub roles: Vec<Role>,
    #[serde(default, deserialize_with = "null_vec")]
    pub voice_states: Vec<crate::voice::VoiceState>,
    #[serde(default)]
    pub member_count: Option<u64>,
    #[serde(default)]
    pub online_count: Option<u64>,
    #[serde(default)]
    pub unavailable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "GuildSnapshot")]
pub struct Guild {
    pub id: Snowflake,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub owner_id: Snowflake,
    #[serde(default, deserialize_with = "null_vec")]
    pub roles: Vec<Role>,
    /// Voice states carried by `GUILD_CREATE` (the SDK's `voiceStatesSync`
    /// source). Empty when the gateway does not send them.
    #[serde(default, deserialize_with = "null_vec")]
    pub voice_states: Vec<crate::voice::VoiceState>,
    #[serde(default)]
    pub member_count: Option<u64>,
    #[serde(default)]
    pub online_count: Option<u64>,
    #[serde(default, deserialize_with = "null_bool")]
    pub unavailable: bool,
}

impl From<GuildSnapshot> for Guild {
    fn from(s: GuildSnapshot) -> Self {
        match s {
            GuildSnapshot::Nested(GuildNested {
                id,
                properties,
                roles,
                member_count,
                online_count,
                unavailable,
                voice_states,
                ..
            }) => Self {
                id,
                name: properties.name,
                icon: properties.icon,
                owner_id: properties.owner_id.unwrap_or_default(),
                roles: if roles.is_empty() {
                    properties.roles
                } else {
                    roles
                },
                voice_states: if voice_states.is_empty() {
                    properties.voice_states
                } else {
                    voice_states
                },
                member_count: member_count.or(properties.member_count),
                online_count: online_count.or(properties.online_count),
                unavailable: unavailable.or(properties.unavailable).unwrap_or(false),
            },
            GuildSnapshot::Flat(f) => Self {
                id: f.id,
                name: f.name,
                icon: f.icon,
                owner_id: f.owner_id.unwrap_or_default(),
                voice_states: f.voice_states,
                roles: f.roles,
                member_count: f.member_count,
                online_count: f.online_count,
                unavailable: f.unavailable.unwrap_or(false),
            },
        }
    }
}

impl Guild {
    pub fn role(&self, id: Snowflake) -> Option<&Role> {
        self.roles.iter().find(|r| r.id == id)
    }

    pub fn highest_role_position(&self, member: &MemberUser) -> i32 {
        member
            .roles
            .iter()
            .filter_map(|id| self.role(*id))
            .map(|r| r.position)
            .max()
            .unwrap_or(-1)
    }

    /// The caller manages the role when ranked strictly above it. The owner always can.
    pub fn manages_role(&self, caller: &MemberUser, is_owner: bool, target: &Role) -> bool {
        if is_owner {
            return true;
        }
        self.highest_role_position(caller) > target.position
    }
}
