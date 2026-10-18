//! Convites (`GET /invites/{code}`, `POST /channels/{id}/invites`).
//! Wire tolerante: campos de evento/REST com `default`.

use serde::Deserialize;

use crate::channel::Channel;
use crate::guild::Guild;
use crate::user::PartialUser;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Invite {
    pub code: String,
    #[serde(default, deserialize_with = "crate::wire::null_u64")]
    pub uses: u64,
    #[serde(default, deserialize_with = "crate::wire::null_u64")]
    pub max_uses: u64,
    #[serde(default, deserialize_with = "crate::wire::null_to_none")]
    pub max_age: Option<u64>,
    #[serde(default, deserialize_with = "crate::wire::null_bool")]
    pub temporary: bool,
    #[serde(default, deserialize_with = "crate::wire::null_to_none")]
    pub created_at: Option<String>,
    #[serde(default, deserialize_with = "crate::wire::null_to_none")]
    pub expires_at: Option<String>,
    #[serde(default, deserialize_with = "crate::wire::null_to_none")]
    pub inviter: Option<PartialUser>,
    #[serde(default, deserialize_with = "crate::wire::null_to_none")]
    pub guild: Option<Guild>,
    #[serde(default, deserialize_with = "crate::wire::null_to_none")]
    pub channel: Option<Channel>,
    #[serde(default, deserialize_with = "crate::wire::null_to_none")]
    pub approximate_member_count: Option<u64>,
    #[serde(default, deserialize_with = "crate::wire::null_to_none")]
    pub approximate_online_count: Option<u64>,
}
