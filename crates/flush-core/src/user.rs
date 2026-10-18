use serde::{Deserialize, Serialize};

use crate::snowflake::Snowflake;

/// Usuário parcial (mensagens, membros, webhooks).
/// Espelho de `APIUserPartial` do `@fluxerjs/types`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartialUser {
    pub id: Snowflake,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub discriminator: String,
    #[serde(default)]
    pub global_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub avatar_color: Option<i32>,
    #[serde(default, deserialize_with = "crate::wire::null_i64")]
    pub flags: i64,
    #[serde(default, deserialize_with = "crate::wire::null_bool")]
    pub bot: bool,
    #[serde(default, deserialize_with = "crate::wire::null_bool")]
    pub system: bool,
}

impl PartialUser {
    pub fn display_name(&self) -> &str {
        self.global_name.as_deref().unwrap_or(&self.username)
    }

    pub fn is_bot(&self) -> bool {
        self.bot
    }

    pub fn mention(&self) -> String {
        format!("<@{}>", self.id)
    }
}

/// Usuário completo (`GET /users/@me`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    #[serde(flatten)]
    pub partial: PartialUser,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub banner: Option<String>,
    #[serde(default)]
    pub accent_color: Option<i32>,
}

impl User {
    pub fn id(&self) -> Snowflake {
        self.partial.id
    }
}
