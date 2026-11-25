//! Voice state payloads (`APIVoiceState`). The driver-backed connection
//! manager lives in `flush-framework::voice`; these types describe
//! gateway/REST data.

use serde::{Deserialize, Serialize};

use crate::guild::MemberUser;
use crate::snowflake::Snowflake;
use crate::wire::{null_bool, null_to_none};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceState {
    #[serde(default, deserialize_with = "null_to_none")]
    pub guild_id: Option<Snowflake>,
    /// `None` means the user left voice.
    #[serde(default, deserialize_with = "null_to_none")]
    pub channel_id: Option<Snowflake>,
    pub user_id: Snowflake,
    #[serde(default, deserialize_with = "null_to_none")]
    pub connection_id: Option<String>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub session_id: Option<String>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub member: Option<MemberUser>,
    #[serde(default, deserialize_with = "null_bool")]
    pub mute: bool,
    #[serde(default, deserialize_with = "null_bool")]
    pub deaf: bool,
    #[serde(default, deserialize_with = "null_bool")]
    pub self_mute: bool,
    #[serde(default, deserialize_with = "null_bool")]
    pub self_deaf: bool,
    #[serde(default, deserialize_with = "null_bool")]
    pub suppress: bool,
    #[serde(default, deserialize_with = "null_bool")]
    pub self_video: bool,
    #[serde(default, deserialize_with = "null_bool")]
    pub self_stream: bool,
    /// ms timestamp or ISO string; the wire type is not fixed.
    #[serde(default)]
    pub request_to_speak_timestamp: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VoiceServerUpdate {
    #[serde(default, deserialize_with = "null_to_none")]
    pub guild_id: Option<Snowflake>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub channel_id: Option<Snowflake>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub token: Option<String>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub endpoint: Option<String>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub connection_id: Option<String>,
}
