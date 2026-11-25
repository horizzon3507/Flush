//! Threads, forum tags and thread members (`@fluxerjs/types` thread section).

use serde::{Deserialize, Serialize};

use crate::channel::Channel;
use crate::guild::MemberUser;
use crate::snowflake::Snowflake;
use crate::wire::{null_bool, null_i64, null_to_none, null_u64, null_vec};

/// Accepted values for `auto_archive_duration`, in minutes.
pub mod auto_archive {
    pub const ONE_HOUR: u32 = 60;
    pub const ONE_DAY: u32 = 1440;
    pub const THREE_DAYS: u32 = 4320;
    pub const ONE_WEEK: u32 = 10080;
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMetadata {
    #[serde(default, deserialize_with = "null_bool")]
    pub archived: bool,
    #[serde(default, deserialize_with = "null_u64")]
    pub auto_archive_duration: u64,
    #[serde(default, deserialize_with = "null_to_none")]
    pub archive_timestamp: Option<String>,
    #[serde(default, deserialize_with = "null_bool")]
    pub locked: bool,
    #[serde(default, deserialize_with = "null_to_none")]
    pub invitable: Option<bool>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub create_timestamp: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMember {
    /// Thread id.
    #[serde(default, deserialize_with = "null_to_none")]
    pub id: Option<Snowflake>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub user_id: Option<Snowflake>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub join_timestamp: Option<String>,
    #[serde(default, deserialize_with = "null_i64")]
    pub flags: i64,
    #[serde(default, deserialize_with = "null_bool")]
    pub muted: bool,
    #[serde(default, deserialize_with = "null_to_none")]
    pub member: Option<MemberUser>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForumTag {
    /// Absent when creating a tag.
    #[serde(
        default,
        deserialize_with = "null_to_none",
        skip_serializing_if = "Option::is_none"
    )]
    pub id: Option<Snowflake>,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "null_bool")]
    pub moderated: bool,
    #[serde(default, deserialize_with = "null_to_none")]
    pub emoji_id: Option<Snowflake>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub emoji_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultReactionEmoji {
    #[serde(default, deserialize_with = "null_to_none")]
    pub emoji_id: Option<Snowflake>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub emoji_name: Option<String>,
}

/// Thread listing returned by active/archived/search endpoints.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ThreadsListing {
    #[serde(default, deserialize_with = "null_vec")]
    pub threads: Vec<Channel>,
    #[serde(default, deserialize_with = "null_vec")]
    pub members: Vec<ThreadMember>,
    #[serde(default, deserialize_with = "null_bool")]
    pub has_more: bool,
}

/// Body for `POST /channels/{id}/threads` and the from-message variant.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct StartThread {
    pub name: String,
    /// Thread channel type; omit when starting from a message.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<crate::channel::ChannelType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_archive_duration: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_user: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invitable: Option<bool>,
    /// Forum/media posts only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_tags: Option<Vec<Snowflake>>,
    /// Forum/media posts only: the starter message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<crate::message::MessageSend>,
}

impl StartThread {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }
}
