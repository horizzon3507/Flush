//! Guild lifecycle, read state, group DM, account and application routes.
//!
//! Most of these are session-token helpers in `@fluxerjs/core` 3.3.0; the API
//! rejects several of them for bot tokens. Bodies follow what the SDK sends.
//! `@fluxerjs/core` ships no caller for `check_username_tag`,
//! `edit_application_bot`, `reset_application_bot_token`, `create_guild` and
//! the stream preview routes, so those take or return loosely typed JSON.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use flush_core::{Guild, Message, Role, Snowflake, ThreadMember};

use crate::client::RestClient;
use crate::error::RestError;
use crate::resources::{non_empty, query_string, to_body};
use crate::routes::Routes;

/// Sudo / MFA confirmation some destructive routes ask of user accounts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SudoVerification {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// `totp` or `webauthn`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mfa_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mfa_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webauthn_response: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webauthn_challenge: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GuildCreate {
    pub name: String,
    /// Data URI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// Body of the discovery application (`POST`/`PATCH /guilds/{id}/discovery`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DiscoveryRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_type: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DiscoveryApplication {
    pub guild_id: Snowflake,
    #[serde(default)]
    pub guild_nsfw_level: Option<u32>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category_type: u32,
    #[serde(default)]
    pub primary_language: Option<String>,
    #[serde(default, deserialize_with = "flush_core::wire::null_vec")]
    pub custom_tags: Vec<String>,
    #[serde(default)]
    pub applied_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DiscoveryStatus {
    #[serde(default)]
    pub application: Option<DiscoveryApplication>,
    #[serde(default)]
    pub eligible: bool,
    #[serde(default)]
    pub min_member_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoleHoistPosition {
    pub id: Snowflake,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hoist_position: Option<i32>,
}

/// One channel of `POST /channels/messages/bulk`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BulkMessageRequest {
    pub channel_id: Snowflake,
    pub limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Snowflake>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Snowflake>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub around: Option<Snowflake>,
}

impl BulkMessageRequest {
    pub fn latest(channel_id: Snowflake, limit: u32) -> Self {
        Self {
            channel_id,
            limit,
            before: None,
            after: None,
            around: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ChannelMessages {
    pub channel_id: Snowflake,
    #[serde(default, deserialize_with = "flush_core::wire::null_vec")]
    pub messages: Vec<Message>,
}

#[derive(Deserialize)]
struct BulkMessagesResponse {
    #[serde(default, deserialize_with = "flush_core::wire::null_vec")]
    channels: Vec<ChannelMessages>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThreadMuteConfig {
    /// ISO-8601 expiry; `Some(None)` means it never expires.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<Option<String>>,
    /// Selected mute duration in seconds.
    pub selected_time_window: i64,
}

/// Body of `PATCH /channels/{id}/thread-members/@me/settings`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ThreadMemberSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flags: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    /// `Some(None)` clears the mute configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mute_config: Option<Option<ThreadMuteConfig>>,
}

/// Preload result: channel ID to its latest message, if any.
pub type PreloadedMessages = BTreeMap<Snowflake, Option<Message>>;

fn sudo_body(sudo: Option<&SudoVerification>) -> Result<Option<Value>, RestError> {
    match sudo {
        Some(s) => Ok(non_empty(to_body(s)?)),
        None => Ok(None),
    }
}

fn check_channel_batch(name: &str, channels: &[Snowflake]) -> Result<(), RestError> {
    if channels.is_empty() || channels.len() > 100 {
        return Err(RestError::Config(format!(
            "{name} requires between 1 and 100 channel IDs"
        )));
    }
    Ok(())
}

fn preloaded(value: Value) -> Result<PreloadedMessages, RestError> {
    let map: BTreeMap<String, Option<Message>> = match value {
        Value::Null => BTreeMap::new(),
        other => {
            serde_json::from_value(other).map_err(|e| RestError::BadResponse(e.to_string()))?
        }
    };
    map.into_iter()
        .map(|(k, v)| {
            k.parse::<u64>()
                .map(|id| (Snowflake(id), v))
                .map_err(|_| RestError::BadResponse(format!("invalid channel id {k}")))
        })
        .collect()
}

impl RestClient {
    // ---- guild lifecycle ----

    /// `POST /guilds`.
    pub async fn create_guild(&self, guild: &GuildCreate) -> Result<Guild, RestError> {
        self.request("POST", Routes::guilds(), Some(to_body(guild)?), None, None)
            .await
    }

    /// `POST /guilds/{id}/delete` (owner only).
    pub async fn delete_guild(
        &self,
        guild: Snowflake,
        sudo: Option<&SudoVerification>,
    ) -> Result<(), RestError> {
        let body = sudo_body(sudo)?.unwrap_or_else(|| json!({}));
        self.request_unit("POST", &Routes::guild_delete(guild), Some(body), None)
            .await
    }

    /// `POST /guilds/{id}/transfer-ownership` (owner only).
    pub async fn transfer_guild_ownership(
        &self,
        guild: Snowflake,
        new_owner: Snowflake,
        password: Option<&str>,
    ) -> Result<(), RestError> {
        let mut body = json!({ "new_owner_id": new_owner });
        if let Some(p) = password {
            body["password"] = json!(p);
        }
        self.request_unit(
            "POST",
            &Routes::guild_transfer_ownership(guild),
            Some(body),
            None,
        )
        .await
    }

    /// `GET /guilds/{id}/discovery`.
    pub async fn guild_discovery_status(
        &self,
        guild: Snowflake,
    ) -> Result<DiscoveryStatus, RestError> {
        self.request("GET", &Routes::guild_discovery(guild), None, None, None)
            .await
    }

    /// `POST /guilds/{id}/discovery`.
    pub async fn apply_for_discovery(
        &self,
        guild: Snowflake,
        application: &DiscoveryRequest,
    ) -> Result<DiscoveryApplication, RestError> {
        self.request(
            "POST",
            &Routes::guild_discovery(guild),
            Some(to_body(application)?),
            None,
            None,
        )
        .await
    }

    /// `PATCH /guilds/{id}/discovery`.
    pub async fn edit_discovery_application(
        &self,
        guild: Snowflake,
        application: &DiscoveryRequest,
    ) -> Result<DiscoveryApplication, RestError> {
        self.request(
            "PATCH",
            &Routes::guild_discovery(guild),
            Some(to_body(application)?),
            None,
            None,
        )
        .await
    }

    /// `DELETE /guilds/{id}/discovery`.
    pub async fn withdraw_discovery_application(&self, guild: Snowflake) -> Result<(), RestError> {
        self.request_unit("DELETE", &Routes::guild_discovery(guild), None, None)
            .await
    }

    /// `PATCH /guilds/{id}/roles/hoist-positions`. Returns the guild roles.
    pub async fn set_role_hoist_positions(
        &self,
        guild: Snowflake,
        positions: &[RoleHoistPosition],
    ) -> Result<Vec<Role>, RestError> {
        let roles: Option<Vec<Role>> = self
            .request(
                "PATCH",
                &Routes::guild_roles_hoist_positions(guild),
                Some(to_body(&positions)?),
                None,
                None,
            )
            .await?;
        Ok(roles.unwrap_or_default())
    }

    /// `DELETE /guilds/{id}/roles/hoist-positions`: resets every hoist position.
    pub async fn reset_role_hoist_positions(
        &self,
        guild: Snowflake,
    ) -> Result<Vec<Role>, RestError> {
        let roles: Option<Vec<Role>> = self
            .request(
                "DELETE",
                &Routes::guild_roles_hoist_positions(guild),
                None,
                None,
                None,
            )
            .await?;
        Ok(roles.unwrap_or_default())
    }

    // ---- bulk deletes and read state ----

    /// `POST /channels/{id}/messages/bulk-delete-mine`: deletes every message
    /// the caller sent in the channel.
    pub async fn bulk_delete_my_messages(
        &self,
        channel: Snowflake,
        sudo: Option<&SudoVerification>,
    ) -> Result<(), RestError> {
        self.request_unit(
            "POST",
            &Routes::channel_bulk_delete_mine(channel),
            sudo_body(sudo)?,
            None,
        )
        .await
    }

    /// `POST /users/@me/guilds/{id}/messages/bulk-delete-mine`.
    pub async fn bulk_delete_my_messages_in_guild(
        &self,
        guild: Snowflake,
        sudo: Option<&SudoVerification>,
    ) -> Result<(), RestError> {
        self.request_unit(
            "POST",
            &Routes::guild_bulk_delete_mine(guild),
            sudo_body(sudo)?,
            None,
        )
        .await
    }

    /// `POST /channels/{id}/messages/purge` (personal notes channels only).
    /// Returns the number of deleted messages.
    pub async fn purge_messages(&self, channel: Snowflake) -> Result<u64, RestError> {
        let res: Value = self
            .request(
                "POST",
                &Routes::channel_messages_purge(channel),
                None,
                None,
                None,
            )
            .await?;
        Ok(res
            .get("deleted_count")
            .and_then(Value::as_u64)
            .unwrap_or(0))
    }

    /// `DELETE /channels/{id}/messages/ack`: clears the channel read state.
    pub async fn clear_read_state(&self, channel: Snowflake) -> Result<(), RestError> {
        self.request_unit("DELETE", &Routes::channel_messages_ack(channel), None, None)
            .await
    }

    /// `POST /channels/{id}/pins/ack`: marks the channel pins as read.
    pub async fn acknowledge_pins(&self, channel: Snowflake) -> Result<(), RestError> {
        self.request_unit("POST", &Routes::channel_pins_ack(channel), None, None)
            .await
    }

    /// `POST /channels/messages/bulk`: fetches messages of several channels.
    pub async fn bulk_fetch_messages(
        &self,
        requests: &[BulkMessageRequest],
    ) -> Result<Vec<ChannelMessages>, RestError> {
        let res: BulkMessagesResponse = self
            .request(
                "POST",
                Routes::channels_messages_bulk(),
                Some(json!({ "requests": requests })),
                None,
                None,
            )
            .await?;
        Ok(res.channels)
    }

    /// `POST /users/@me/preload-messages` (1 to 100 channels).
    pub async fn preload_messages(
        &self,
        channels: &[Snowflake],
    ) -> Result<PreloadedMessages, RestError> {
        check_channel_batch("preload_messages", channels)?;
        let res: Value = self
            .request(
                "POST",
                Routes::preload_messages(),
                Some(json!({ "channels": channels })),
                None,
                None,
            )
            .await?;
        preloaded(res)
    }

    /// `POST /users/@me/channels/messages/preload`, the alternate preload route.
    pub async fn preload_messages_alt(
        &self,
        channels: &[Snowflake],
    ) -> Result<PreloadedMessages, RestError> {
        check_channel_batch("preload_messages_alt", channels)?;
        let res: Value = self
            .request(
                "POST",
                Routes::preload_messages_alt(),
                Some(json!({ "channels": channels })),
                None,
                None,
            )
            .await?;
        preloaded(res)
    }

    // ---- group DMs and user state ----

    /// `PUT /channels/{id}/recipients/{user}`: adds a user to a group DM.
    pub async fn add_group_recipient(
        &self,
        channel: Snowflake,
        user: Snowflake,
    ) -> Result<(), RestError> {
        self.request_unit("PUT", &Routes::channel_recipient(channel, user), None, None)
            .await
    }

    /// `DELETE /channels/{id}/recipients/{user}`, optionally without the system message.
    pub async fn remove_group_recipient(
        &self,
        channel: Snowflake,
        user: Snowflake,
        silent: bool,
    ) -> Result<(), RestError> {
        let q = silent.then_some("silent=true");
        let _: Value = self
            .request(
                "DELETE",
                &Routes::channel_recipient(channel, user),
                None,
                None,
                q,
            )
            .await?;
        Ok(())
    }

    /// `PUT /users/@me/channels/{id}/pin`: pins a DM to the top of the DM list.
    pub async fn pin_dm_channel(&self, channel: Snowflake) -> Result<(), RestError> {
        self.request_unit("PUT", &Routes::user_me_channel_pin(channel), None, None)
            .await
    }

    /// `DELETE /users/@me/channels/{id}/pin`.
    pub async fn unpin_dm_channel(&self, channel: Snowflake) -> Result<(), RestError> {
        self.request_unit("DELETE", &Routes::user_me_channel_pin(channel), None, None)
            .await
    }

    /// `PATCH /channels/{id}/thread-members/@me/settings`.
    pub async fn edit_thread_member_settings(
        &self,
        thread: Snowflake,
        settings: &ThreadMemberSettings,
    ) -> Result<Option<ThreadMember>, RestError> {
        self.request(
            "PATCH",
            &Routes::channel_thread_member_me_settings(thread),
            Some(to_body(settings)?),
            None,
            None,
        )
        .await
    }

    /// `GET /users/check-tag`: whether `username#discriminator` is taken.
    pub async fn check_username_tag(
        &self,
        username: &str,
        discriminator: &str,
    ) -> Result<bool, RestError> {
        let q = query_string(&[
            ("username", Some(username.to_owned())),
            ("discriminator", Some(discriminator.to_owned())),
        ]);
        let res: Value = self
            .request(
                "GET",
                Routes::check_username_tag(),
                None,
                None,
                q.as_deref(),
            )
            .await?;
        Ok(res.get("taken").and_then(Value::as_bool).unwrap_or(false))
    }

    // ---- applications ----

    /// `PATCH /oauth2/applications/{id}/bot`: edits the application's bot user.
    pub async fn edit_application_bot(
        &self,
        application: Snowflake,
        patch: Value,
    ) -> Result<Value, RestError> {
        self.request(
            "PATCH",
            &Routes::oauth2_application_bot(application),
            Some(patch),
            None,
            None,
        )
        .await
    }

    /// `POST /oauth2/applications/{id}/bot/reset-token`. Returns the new
    /// token; the one this client holds stops working.
    pub async fn reset_application_bot_token(
        &self,
        application: Snowflake,
        sudo: Option<&SudoVerification>,
    ) -> Result<String, RestError> {
        let res: Value = self
            .request(
                "POST",
                &Routes::oauth2_application_bot_reset_token(application),
                sudo_body(sudo)?,
                None,
                None,
            )
            .await?;
        res.get("token")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| RestError::BadResponse("no token in response".into()))
    }

    // ---- instance and streams ----

    /// `GET {origin}/.well-known/fluxer`: the full instance document
    /// ([`RestClient::discover`] keeps only the endpoints).
    pub async fn instance_discovery(origin: &str) -> Result<Value, RestError> {
        let url = format!(
            "{}{}",
            origin.trim_end_matches('/'),
            Routes::instance_discovery()
        );
        surf::get(&url)
            .recv_json()
            .await
            .map_err(|e| RestError::Transport(format!("discovery {url}: {e}")))
    }

    /// `GET /streams/{key}/preview`.
    pub async fn stream_preview(&self, stream_key: &str) -> Result<Value, RestError> {
        self.request("GET", &Routes::stream_preview(stream_key), None, None, None)
            .await
    }

    /// `POST /streams/{key}/preview`: uploads a preview thumbnail.
    pub async fn upload_stream_preview(
        &self,
        stream_key: &str,
        body: Value,
    ) -> Result<(), RestError> {
        self.request_unit(
            "POST",
            &Routes::stream_preview(stream_key),
            Some(body),
            None,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sudo_body_is_omitted_when_empty() {
        assert_eq!(sudo_body(None).unwrap(), None);
        assert_eq!(sudo_body(Some(&SudoVerification::default())).unwrap(), None);
        let sudo = SudoVerification {
            mfa_method: Some("totp".into()),
            mfa_code: Some("123456".into()),
            ..Default::default()
        };
        assert_eq!(
            sudo_body(Some(&sudo)).unwrap(),
            Some(json!({ "mfa_method": "totp", "mfa_code": "123456" }))
        );
    }

    #[test]
    fn channel_batch_bounds() {
        assert!(check_channel_batch("x", &[]).is_err());
        assert!(check_channel_batch("x", &[Snowflake(1)]).is_ok());
        assert!(check_channel_batch("x", &vec![Snowflake(1); 101]).is_err());
    }

    #[test]
    fn preload_response_maps_channels() {
        let map = preloaded(json!({
            "10": null,
            "11": { "id": "5", "channel_id": "11", "author": { "id": "1", "username": "a" } }
        }))
        .unwrap();
        assert_eq!(map[&Snowflake(10)], None);
        assert_eq!(map[&Snowflake(11)].as_ref().unwrap().id, Snowflake(5));
        assert!(preloaded(Value::Null).unwrap().is_empty());
        assert!(preloaded(json!({ "abc": null })).is_err());
    }
}
