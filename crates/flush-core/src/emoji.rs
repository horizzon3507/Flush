//! Guild emojis and stickers (`APIEmoji`, `APISticker` in `@fluxerjs/types`).

use serde::{Deserialize, Serialize};

use crate::snowflake::Snowflake;
use crate::user::PartialUser;
use crate::wire::{null_bool, null_to_none, null_vec};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Emoji {
    pub id: Snowflake,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "null_bool")]
    pub animated: bool,
    #[serde(default, deserialize_with = "null_bool")]
    pub nsfw: bool,
    /// Creator, only present on the `WithUser` REST variants.
    #[serde(
        default,
        deserialize_with = "null_to_none",
        skip_serializing_if = "Option::is_none"
    )]
    pub user: Option<PartialUser>,
}

impl Emoji {
    /// Message markup: `<:name:id>` or `<a:name:id>`.
    pub fn mention(&self) -> String {
        let prefix = if self.animated { "a" } else { "" };
        format!("<{prefix}:{}:{}>", self.name, self.id)
    }

    /// `name:id`, the form reaction routes expect.
    pub fn reaction(&self) -> String {
        format!("{}:{}", self.name, self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sticker {
    pub id: Snowflake,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "null_to_none")]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "null_vec")]
    pub tags: Vec<String>,
    #[serde(default, deserialize_with = "null_bool")]
    pub animated: bool,
    #[serde(default, deserialize_with = "null_bool")]
    pub nsfw: bool,
    #[serde(
        default,
        deserialize_with = "null_to_none",
        skip_serializing_if = "Option::is_none"
    )]
    pub user: Option<PartialUser>,
}

/// `GET /emojis/{id}/metadata` and `GET /stickers/{id}/metadata`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ExpressionMetadata {
    pub id: Snowflake,
    #[serde(default, deserialize_with = "null_to_none")]
    pub guild_id: Option<Snowflake>,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "null_bool")]
    pub animated: bool,
    #[serde(default, deserialize_with = "null_bool")]
    pub allow_cloning: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct BulkFailure {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub error: String,
}

/// Result of the bulk emoji/sticker create endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct BulkCreateResult<T> {
    #[serde(default = "Vec::new", deserialize_with = "null_vec")]
    pub success: Vec<T>,
    #[serde(default, deserialize_with = "null_vec")]
    pub failed: Vec<BulkFailure>,
}
