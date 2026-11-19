use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::permissions::wire as perm_wire;
use crate::snowflake::Snowflake;
use crate::thread::{DefaultReactionEmoji, ForumTag, ThreadMember, ThreadMetadata};
use crate::user::PartialUser;
use crate::wire::{null_bool, null_vec};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u16)]
pub enum ChannelType {
    GuildText = 0,
    Dm = 1,
    GuildVoice = 2,
    GroupDm = 3,
    GuildCategory = 4,
    GuildAnnouncement = 5,
    AnnouncementThread = 10,
    PublicThread = 11,
    PrivateThread = 12,
    GuildForum = 15,
    GuildMedia = 16,
    GuildLink = 998,
    DmPersonalNotes = 999,
    #[default]
    Unknown = u16::MAX,
}

impl Serialize for ChannelType {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u16(*self as u16)
    }
}

impl<'de> Deserialize<'de> for ChannelType {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = u16::deserialize(d)?;
        Ok(match v {
            0 => Self::GuildText,
            1 => Self::Dm,
            2 => Self::GuildVoice,
            3 => Self::GroupDm,
            4 => Self::GuildCategory,
            5 => Self::GuildAnnouncement,
            10 => Self::AnnouncementThread,
            11 => Self::PublicThread,
            12 => Self::PrivateThread,
            15 => Self::GuildForum,
            16 => Self::GuildMedia,
            998 => Self::GuildLink,
            999 => Self::DmPersonalNotes,
            _ => Self::Unknown,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum OverwriteType {
    Role = 0,
    Member = 1,
}

impl Serialize for OverwriteType {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u8(*self as u8)
    }
}

impl<'de> Deserialize<'de> for OverwriteType {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = u8::deserialize(d)?;
        Ok(match v {
            1 => Self::Member,
            _ => Self::Role,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionOverwrite {
    pub id: Snowflake,
    #[serde(rename = "type")]
    pub kind: OverwriteType,
    #[serde(with = "perm_wire")]
    pub allow: u64,
    #[serde(with = "perm_wire")]
    pub deny: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    pub id: Snowflake,
    #[serde(rename = "type")]
    pub kind: ChannelType,
    #[serde(default)]
    pub guild_id: Option<Snowflake>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub position: Option<i32>,
    #[serde(default)]
    pub parent_id: Option<Snowflake>,
    #[serde(default)]
    pub last_message_id: Option<Snowflake>,
    #[serde(default, deserialize_with = "null_vec")]
    pub permission_overwrites: Vec<PermissionOverwrite>,
    #[serde(default, deserialize_with = "null_vec")]
    pub recipients: Vec<PartialUser>,
    #[serde(default, deserialize_with = "null_bool")]
    pub nsfw: bool,
    #[serde(default)]
    pub rate_limit_per_user: Option<u64>,
    #[serde(default)]
    pub owner_id: Option<Snowflake>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub bitrate: Option<u64>,
    #[serde(default)]
    pub user_limit: Option<u64>,
    #[serde(default)]
    pub rtc_region: Option<String>,
    #[serde(default)]
    pub last_pin_timestamp: Option<String>,
    #[serde(default)]
    pub flags: Option<u64>,
    #[serde(default)]
    pub thread_metadata: Option<ThreadMetadata>,
    #[serde(default, deserialize_with = "null_vec")]
    pub applied_tags: Vec<Snowflake>,
    #[serde(default)]
    pub message_count: Option<u64>,
    #[serde(default)]
    pub member_count: Option<u64>,
    #[serde(default)]
    pub member: Option<ThreadMember>,
    #[serde(default)]
    pub default_auto_archive_duration: Option<u64>,
    #[serde(default, deserialize_with = "null_vec")]
    pub available_tags: Vec<ForumTag>,
    #[serde(default)]
    pub default_reaction_emoji: Option<DefaultReactionEmoji>,
}

impl Channel {
    pub fn is_text_bearing(&self) -> bool {
        !matches!(
            self.kind,
            ChannelType::GuildCategory
                | ChannelType::GuildLink
                | ChannelType::GuildForum
                | ChannelType::GuildMedia
        )
    }

    pub fn is_thread(&self) -> bool {
        matches!(
            self.kind,
            ChannelType::AnnouncementThread
                | ChannelType::PublicThread
                | ChannelType::PrivateThread
        )
    }

    /// Forum and media channels hold posts (threads), not messages.
    pub fn is_forum_like(&self) -> bool {
        matches!(self.kind, ChannelType::GuildForum | ChannelType::GuildMedia)
    }

    pub fn is_voice(&self) -> bool {
        self.kind == ChannelType::GuildVoice
    }

    pub fn mention(&self) -> String {
        format!("<#{}>", self.id)
    }

    pub fn is_dm(&self) -> bool {
        matches!(
            self.kind,
            ChannelType::Dm | ChannelType::GroupDm | ChannelType::DmPersonalNotes
        )
    }
}
