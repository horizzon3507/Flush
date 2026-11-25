//! Webhooks (`APIWebhook` in `@fluxerjs/types`).

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::snowflake::Snowflake;
use crate::user::PartialUser;
use crate::wire::null_to_none;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WebhookType {
    #[default]
    Incoming,
    ChannelFollower,
    Unknown(u8),
}

impl Serialize for WebhookType {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u8(match self {
            Self::Incoming => 1,
            Self::ChannelFollower => 2,
            Self::Unknown(v) => *v,
        })
    }
}

impl<'de> Deserialize<'de> for WebhookType {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match u8::deserialize(d)? {
            1 => Self::Incoming,
            2 => Self::ChannelFollower,
            v => Self::Unknown(v),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Webhook {
    pub id: Snowflake,
    #[serde(default, deserialize_with = "null_to_none")]
    pub guild_id: Option<Snowflake>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub channel_id: Option<Snowflake>,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "null_to_none")]
    pub avatar: Option<String>,
    /// Only returned to callers allowed to execute the webhook.
    #[serde(default, deserialize_with = "null_to_none")]
    pub token: Option<String>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub user: Option<PartialUser>,
    #[serde(rename = "type", default)]
    pub kind: WebhookType,
}

/// Body for `POST /webhooks/{id}/{token}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct WebhookExecute {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub embeds: Vec<crate::embed::Embed>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tts: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_mentions: Option<crate::message::AllowedMentions>,
}

impl WebhookExecute {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: Some(content.into()),
            ..Default::default()
        }
    }
}
