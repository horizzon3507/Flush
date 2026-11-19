//! Guild audit log (`APIGuildAuditLog` in `@fluxerjs/types`).

use std::collections::HashMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::snowflake::Snowflake;
use crate::wire::{null_to_none, null_vec};

macro_rules! audit_actions {
    ($($name:ident = $value:literal),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum AuditLogAction {
            $($name,)+
            Unknown(u16),
        }

        impl AuditLogAction {
            pub fn from_code(code: u16) -> Self {
                match code {
                    $($value => Self::$name,)+
                    other => Self::Unknown(other),
                }
            }

            pub fn code(self) -> u16 {
                match self {
                    $(Self::$name => $value,)+
                    Self::Unknown(other) => other,
                }
            }
        }
    };
}

audit_actions! {
    GuildUpdate = 1,
    ChannelCreate = 10,
    ChannelUpdate = 11,
    ChannelDelete = 12,
    ChannelOverwriteCreate = 13,
    ChannelOverwriteUpdate = 14,
    ChannelOverwriteDelete = 15,
    MemberKick = 20,
    MemberPrune = 21,
    MemberBanAdd = 22,
    MemberBanRemove = 23,
    MemberUpdate = 24,
    MemberRoleUpdate = 25,
    MemberMove = 26,
    MemberDisconnect = 27,
    BotAdd = 28,
    RoleCreate = 30,
    RoleUpdate = 31,
    RoleDelete = 32,
    InviteCreate = 40,
    InviteUpdate = 41,
    InviteDelete = 42,
    WebhookCreate = 50,
    WebhookUpdate = 51,
    WebhookDelete = 52,
    EmojiCreate = 60,
    EmojiUpdate = 61,
    EmojiDelete = 62,
    MessageDelete = 72,
    MessageBulkDelete = 73,
    MessagePin = 74,
    MessageUnpin = 75,
    StickerCreate = 90,
    StickerUpdate = 91,
    StickerDelete = 92,
    ThreadCreate = 110,
    ThreadUpdate = 111,
    ThreadDelete = 112,
}

impl Serialize for AuditLogAction {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u16(self.code())
    }
}

impl<'de> Deserialize<'de> for AuditLogAction {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::from_code(u16::deserialize(d)?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AuditLogChange {
    pub key: String,
    #[serde(default)]
    pub old_value: Option<serde_json::Value>,
    #[serde(default)]
    pub new_value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AuditLogEntry {
    pub id: Snowflake,
    pub action_type: AuditLogAction,
    #[serde(default, deserialize_with = "null_to_none")]
    pub user_id: Option<Snowflake>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub target_id: Option<Snowflake>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub reason: Option<String>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub options: Option<HashMap<String, serde_json::Value>>,
    #[serde(default, deserialize_with = "null_vec")]
    pub changes: Vec<AuditLogChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AuditLogUser {
    pub id: Snowflake,
    #[serde(default, deserialize_with = "null_to_none")]
    pub username: Option<String>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub discriminator: Option<String>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub avatar: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AuditLogWebhook {
    pub id: Snowflake,
    #[serde(default, deserialize_with = "null_to_none")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub avatar: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct AuditLog {
    #[serde(default, deserialize_with = "null_vec")]
    pub audit_log_entries: Vec<AuditLogEntry>,
    #[serde(default, deserialize_with = "null_vec")]
    pub users: Vec<AuditLogUser>,
    #[serde(default, deserialize_with = "null_vec")]
    pub webhooks: Vec<AuditLogWebhook>,
}
