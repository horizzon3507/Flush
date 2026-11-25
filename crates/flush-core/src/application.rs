//! Bot application (`GET /applications/@me`, `GET /oauth2/applications/@me`).

use serde::{Deserialize, Serialize};

use crate::snowflake::Snowflake;
use crate::user::PartialUser;
use crate::wire::{null_bool, null_i64, null_to_none};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Application {
    pub id: Snowflake,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "null_to_none")]
    pub icon: Option<String>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "null_bool")]
    pub bot_public: bool,
    #[serde(default, deserialize_with = "null_bool")]
    pub bot_require_code_grant: bool,
    #[serde(default, deserialize_with = "null_to_none")]
    pub verify_key: Option<String>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub owner: Option<PartialUser>,
    #[serde(default, deserialize_with = "null_to_none")]
    pub bot: Option<PartialUser>,
    #[serde(default, deserialize_with = "null_i64")]
    pub flags: i64,
}
