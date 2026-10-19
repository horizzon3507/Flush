use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::embed::Embed;
use crate::snowflake::Snowflake;
use crate::user::PartialUser;
use crate::wire::{null_bool, null_vec};
/// Tipo de mensagem como inteiro no wire (subset usado por bots).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum MessageType {
    #[default]
    Default = 0,
    RecipientAdd = 1,
    RecipientRemove = 2,
    ChannelNameChange = 4,
    ChannelPinnedMessage = 6,
    UserJoin = 7,
    Reply = 19,
    Unknown = 255,
}
impl Serialize for MessageType {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u8(*self as u8)
    }
}

impl<'de> Deserialize<'de> for MessageType {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = u8::deserialize(d)?;
        Ok(match v {
            0 => Self::Default,
            1 => Self::RecipientAdd,
            2 => Self::RecipientRemove,
            4 => Self::ChannelNameChange,
            6 => Self::ChannelPinnedMessage,
            7 => Self::UserJoin,
            19 => Self::Reply,
            _ => Self::Unknown,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageReference {
    pub channel_id: Snowflake,
    pub message_id: Snowflake,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<Snowflake>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllowedMentions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parse: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub users: Option<Vec<Snowflake>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<Snowflake>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replied_user: Option<bool>,
}

impl AllowedMentions {
    pub fn no_ping() -> Self {
        Self {
            parse: Some(vec![]),
            users: Some(vec![]),
            roles: Some(vec![]),
            replied_user: Some(false),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionEmoji {
    #[serde(default)]
    pub id: Option<Snowflake>,
    #[serde(default)]
    pub name: String,
}

impl ReactionEmoji {
    /// Formato para rota de reaction: `nome:id` em custom, `nome` em unicode.
    pub fn escaped(&self) -> String {
        match self.id {
            Some(id) => format!("{}:{}", self.name, id),
            None => self.name.clone(),
        }
    }

    /// `escaped()` pronto para URL (percent-encoding dos bytes não-safe).
    pub fn encoded(&self) -> String {
        const SAFE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.~";
        let mut out = String::new();
        for b in self.escaped().as_bytes() {
            if SAFE.contains(b) {
                out.push(*b as char);
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    pub emoji: ReactionEmoji,
    #[serde(default, deserialize_with = "crate::wire::null_u64")]
    pub count: u64,
    #[serde(default, deserialize_with = "null_bool")]
    pub me: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageAttachment {
    pub id: Snowflake,
    #[serde(default)]
    pub filename: String,
    #[serde(default, deserialize_with = "crate::wire::null_u64")]
    pub size: u64,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
}

/// Mensagem completa (subset; campos desconhecidos são ignorados).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: Snowflake,
    pub channel_id: Snowflake,
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    pub author: PartialUser,
    #[serde(default, rename = "type")]
    pub kind: MessageType,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub edited_timestamp: Option<String>,
    #[serde(default, deserialize_with = "null_bool")]
    pub pinned: bool,
    #[serde(default, deserialize_with = "null_bool")]
    pub mention_everyone: bool,
    #[serde(default, deserialize_with = "null_vec")]
    pub mentions: Vec<PartialUser>,
    #[serde(default, deserialize_with = "null_vec")]
    pub mention_roles: Vec<Snowflake>,
    #[serde(default, deserialize_with = "null_vec")]
    pub embeds: Vec<Embed>,
    #[serde(default, deserialize_with = "null_vec")]
    pub attachments: Vec<MessageAttachment>,
    #[serde(default, deserialize_with = "null_vec")]
    pub reactions: Vec<Reaction>,
    #[serde(default)]
    pub message_reference: Option<MessageReference>,
    #[serde(default)]
    pub webhook_id: Option<Snowflake>,
}

impl Message {
    pub fn is_bot(&self) -> bool {
        self.author.bot
    }
}

/// Payload de envio (`POST /channels/{id}/messages`).
/// Só flags enviáveis: SUPPRESS_EMBEDS=4, SUPPRESS_NOTIFICATIONS=4096.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSend {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub embeds: Vec<Embed>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_reference: Option<MessageReference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_mentions: Option<AllowedMentions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tts: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flags: Option<u64>,
}

impl MessageSend {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: Some(content.into()),
            ..Default::default()
        }
    }

    /// Texto sem mencionar ninguém (só a mensagem, sem referência de reply).
    pub fn no_ping_text(content: impl Into<String>) -> Self {
        Self {
            content: Some(content.into()),
            allowed_mentions: Some(AllowedMentions::no_ping()),
            ..Default::default()
        }
    }

    pub fn validate(&self) -> Result<(), crate::error::CoreError> {
        let empty_content = self.content.as_ref().map(|s| s.is_empty()).unwrap_or(true);
        if empty_content && self.embeds.is_empty() {
            return Err(crate::error::CoreError::InvalidMessage(
                "content vazio sem embeds".into(),
            ));
        }
        for e in &self.embeds {
            e.validate()?;
        }
        Ok(())
    }
}
