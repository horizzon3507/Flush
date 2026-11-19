//! Resource endpoints beyond the moderation core: webhooks, emojis,
//! stickers, audit logs, threads, forum tags, permission overwrites and the
//! guild/application/user routes of `@fluxerjs/core` 3.x.
//!
//! Methods and bodies follow what the SDK sends for each route.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use flush_core::{
    Application, AuditLog, AuditLogAction, BulkCreateResult, Channel, Emoji, ExpressionMetadata,
    ForumTag, Guild, MemberUser, Message, OverwriteType, Role, Snowflake, StartThread, Sticker,
    ThreadMember, ThreadsListing, Webhook, WebhookExecute,
};

use crate::client::RestClient;
use crate::error::RestError;
use crate::routes::{encode_component, Routes};

fn to_body<T: Serialize>(value: &T) -> Result<Value, RestError> {
    serde_json::to_value(value).map_err(|e| RestError::Transport(e.to_string()))
}

fn non_empty(body: Value) -> Option<Value> {
    match &body {
        Value::Object(map) if map.is_empty() => None,
        Value::Null => None,
        _ => Some(body),
    }
}

/// Builds `a=b&c=d`, percent-encoding values and skipping `None`.
pub(crate) fn query_string(pairs: &[(&str, Option<String>)]) -> Option<String> {
    let parts: Vec<String> = pairs
        .iter()
        .filter_map(|(k, v)| v.as_ref().map(|v| format!("{k}={}", encode_component(v))))
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("&"))
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct WebhookPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Data URI; `Some(None)` clears the avatar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<Option<String>>,
    /// Ignored by the token-scoped edit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<Snowflake>,
}

#[derive(Debug, Clone, Default)]
pub struct AuditLogQuery {
    pub limit: Option<u32>,
    pub before: Option<Snowflake>,
    pub after: Option<Snowflake>,
    pub user_id: Option<Snowflake>,
    pub action_type: Option<AuditLogAction>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct StickerCreate {
    pub name: String,
    /// Data URI.
    pub image: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct StickerPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GuildPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Data URI; `Some(None)` clears it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub banner: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub splash: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_channel_id: Option<Option<Snowflake>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub afk_channel_id: Option<Option<Snowflake>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub afk_timeout: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_level: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_message_notifications: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explicit_content_filter: Option<u8>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ChannelPosition {
    pub id: Snowflake,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<Option<Snowflake>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock_permissions: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RolePosition {
    pub id: Snowflake,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<i32>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MemberSearch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_ids: Option<Vec<Snowflake>>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ThreadSearch {
    #[serde(skip)]
    pub name: Option<String>,
    #[serde(skip)]
    pub tag: Option<Snowflake>,
    #[serde(skip)]
    pub archived: Option<bool>,
    /// `last_message_time`, `archive_time`, `relevance` or `creation_time`.
    #[serde(skip)]
    pub sort_by: Option<String>,
    /// `asc` or `desc`.
    #[serde(skip)]
    pub sort_order: Option<String>,
    #[serde(skip)]
    pub limit: Option<u32>,
    #[serde(skip)]
    pub offset: Option<u32>,
}

/// Entry of `GET /users/@me/guilds`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UserGuild {
    pub id: Snowflake,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub owner: Option<bool>,
    #[serde(default)]
    pub permissions: Option<String>,
    #[serde(default)]
    pub approximate_member_count: Option<u64>,
    #[serde(default)]
    pub approximate_presence_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VanityUrl {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub uses: u64,
}

impl RestClient {
    // ---- webhooks ----

    pub async fn channel_webhooks(&self, channel: Snowflake) -> Result<Vec<Webhook>, RestError> {
        self.request("GET", &Routes::channel_webhooks(channel), None, None, None)
            .await
    }

    pub async fn guild_webhooks(&self, guild: Snowflake) -> Result<Vec<Webhook>, RestError> {
        self.request("GET", &Routes::guild_webhooks(guild), None, None, None)
            .await
    }

    /// `avatar` is a data URI.
    pub async fn create_webhook(
        &self,
        channel: Snowflake,
        name: &str,
        avatar: Option<&str>,
        audit: Option<&str>,
    ) -> Result<Webhook, RestError> {
        let mut body = json!({ "name": name });
        if let Some(a) = avatar {
            body["avatar"] = json!(a);
        }
        self.request(
            "POST",
            &Routes::channel_webhooks(channel),
            Some(body),
            audit,
            None,
        )
        .await
    }

    pub async fn webhook(&self, id: Snowflake) -> Result<Webhook, RestError> {
        self.request("GET", &Routes::webhook(id), None, None, None)
            .await
    }

    pub async fn edit_webhook(
        &self,
        id: Snowflake,
        patch: &WebhookPatch,
        audit: Option<&str>,
    ) -> Result<Webhook, RestError> {
        self.request(
            "PATCH",
            &Routes::webhook(id),
            Some(to_body(patch)?),
            audit,
            None,
        )
        .await
    }

    pub async fn delete_webhook(
        &self,
        id: Snowflake,
        audit: Option<&str>,
    ) -> Result<(), RestError> {
        self.request_unit("DELETE", &Routes::webhook(id), None, audit)
            .await
    }

    /// Token-scoped fetch; the bot token is not sent.
    pub async fn webhook_with_token(
        &self,
        id: Snowflake,
        token: &str,
    ) -> Result<Webhook, RestError> {
        self.request_with(
            "GET",
            &Routes::webhook_execute(id, token),
            None,
            None,
            None,
            false,
        )
        .await
    }

    /// Token-scoped edit (name/avatar only); the bot token is not sent.
    pub async fn edit_webhook_with_token(
        &self,
        id: Snowflake,
        token: &str,
        patch: &WebhookPatch,
    ) -> Result<Webhook, RestError> {
        let mut body = to_body(patch)?;
        if let Some(map) = body.as_object_mut() {
            map.remove("channel_id");
        }
        self.request_with(
            "PATCH",
            &Routes::webhook_execute(id, token),
            Some(body),
            None,
            None,
            false,
        )
        .await
    }

    pub async fn delete_webhook_with_token(
        &self,
        id: Snowflake,
        token: &str,
    ) -> Result<(), RestError> {
        let _: Value = self
            .request_with(
                "DELETE",
                &Routes::webhook_execute(id, token),
                None,
                None,
                None,
                false,
            )
            .await?;
        Ok(())
    }

    /// Executes a webhook. With `wait`, the created message is returned.
    pub async fn execute_webhook(
        &self,
        id: Snowflake,
        token: &str,
        payload: &WebhookExecute,
        wait: bool,
    ) -> Result<Option<Message>, RestError> {
        if payload.content.as_deref().unwrap_or("").is_empty() && payload.embeds.is_empty() {
            return Err(RestError::Config("webhook payload is empty".into()));
        }
        let query = wait.then_some("wait=true");
        let value: Value = self
            .request_with(
                "POST",
                &Routes::webhook_execute(id, token),
                Some(to_body(payload)?),
                None,
                query,
                false,
            )
            .await?;
        if value.is_null() {
            return Ok(None);
        }
        serde_json::from_value(value)
            .map(Some)
            .map_err(|e| RestError::BadResponse(e.to_string()))
    }

    pub async fn edit_webhook_message(
        &self,
        id: Snowflake,
        token: &str,
        message: Snowflake,
        body: &crate::client::EditMessageBody,
    ) -> Result<Message, RestError> {
        self.request_with(
            "PATCH",
            &Routes::webhook_message(id, token, message),
            Some(to_body(body)?),
            None,
            None,
            false,
        )
        .await
    }

    pub async fn delete_webhook_message(
        &self,
        id: Snowflake,
        token: &str,
        message: Snowflake,
    ) -> Result<(), RestError> {
        let _: Value = self
            .request_with(
                "DELETE",
                &Routes::webhook_message(id, token, message),
                None,
                None,
                None,
                false,
            )
            .await?;
        Ok(())
    }

    // ---- emojis ----

    pub async fn guild_emojis(&self, guild: Snowflake) -> Result<Vec<Emoji>, RestError> {
        self.request("GET", &Routes::guild_emojis(guild), None, None, None)
            .await
    }

    pub async fn guild_emoji(
        &self,
        guild: Snowflake,
        emoji: Snowflake,
    ) -> Result<Emoji, RestError> {
        self.request("GET", &Routes::guild_emoji(guild, emoji), None, None, None)
            .await
    }

    /// `image` is a data URI.
    pub async fn create_emoji(
        &self,
        guild: Snowflake,
        name: &str,
        image: &str,
        audit: Option<&str>,
    ) -> Result<Emoji, RestError> {
        let body = json!({ "name": name, "image": image });
        self.request(
            "POST",
            &Routes::guild_emojis(guild),
            Some(body),
            audit,
            None,
        )
        .await
    }

    /// Each item is `(name, data URI)`.
    pub async fn create_emojis_bulk(
        &self,
        guild: Snowflake,
        emojis: &[(String, String)],
        audit: Option<&str>,
    ) -> Result<BulkCreateResult<Emoji>, RestError> {
        let items: Vec<Value> = emojis
            .iter()
            .map(|(name, image)| json!({ "name": name, "image": image }))
            .collect();
        self.request(
            "POST",
            &Routes::guild_emojis_bulk(guild),
            Some(json!({ "emojis": items })),
            audit,
            None,
        )
        .await
    }

    pub async fn clone_emoji(
        &self,
        guild: Snowflake,
        source_emoji: Snowflake,
        audit: Option<&str>,
    ) -> Result<Emoji, RestError> {
        let body = json!({ "source_emoji_id": source_emoji.to_string() });
        self.request(
            "POST",
            &Routes::guild_emojis_clone(guild),
            Some(body),
            audit,
            None,
        )
        .await
    }

    pub async fn rename_emoji(
        &self,
        guild: Snowflake,
        emoji: Snowflake,
        name: &str,
        audit: Option<&str>,
    ) -> Result<Emoji, RestError> {
        self.request(
            "PATCH",
            &Routes::guild_emoji(guild, emoji),
            Some(json!({ "name": name })),
            audit,
            None,
        )
        .await
    }

    pub async fn delete_emoji(
        &self,
        guild: Snowflake,
        emoji: Snowflake,
        audit: Option<&str>,
    ) -> Result<(), RestError> {
        self.request_unit("DELETE", &Routes::guild_emoji(guild, emoji), None, audit)
            .await
    }

    pub async fn emoji_metadata(&self, emoji: Snowflake) -> Result<ExpressionMetadata, RestError> {
        self.request("GET", &Routes::emoji_metadata(emoji), None, None, None)
            .await
    }

    // ---- stickers ----

    pub async fn guild_stickers(&self, guild: Snowflake) -> Result<Vec<Sticker>, RestError> {
        self.request("GET", &Routes::guild_stickers(guild), None, None, None)
            .await
    }

    pub async fn guild_sticker(
        &self,
        guild: Snowflake,
        sticker: Snowflake,
    ) -> Result<Sticker, RestError> {
        self.request(
            "GET",
            &Routes::guild_sticker(guild, sticker),
            None,
            None,
            None,
        )
        .await
    }

    pub async fn create_sticker(
        &self,
        guild: Snowflake,
        create: &StickerCreate,
        audit: Option<&str>,
    ) -> Result<Sticker, RestError> {
        self.request(
            "POST",
            &Routes::guild_stickers(guild),
            Some(to_body(create)?),
            audit,
            None,
        )
        .await
    }

    pub async fn create_stickers_bulk(
        &self,
        guild: Snowflake,
        stickers: &[StickerCreate],
        audit: Option<&str>,
    ) -> Result<BulkCreateResult<Sticker>, RestError> {
        self.request(
            "POST",
            &Routes::guild_stickers_bulk(guild),
            Some(json!({ "stickers": to_body(&stickers)? })),
            audit,
            None,
        )
        .await
    }

    pub async fn clone_sticker(
        &self,
        guild: Snowflake,
        source_sticker: Snowflake,
        audit: Option<&str>,
    ) -> Result<Sticker, RestError> {
        let body = json!({ "source_sticker_id": source_sticker.to_string() });
        self.request(
            "POST",
            &Routes::guild_stickers_clone(guild),
            Some(body),
            audit,
            None,
        )
        .await
    }

    pub async fn edit_sticker(
        &self,
        guild: Snowflake,
        sticker: Snowflake,
        patch: &StickerPatch,
        audit: Option<&str>,
    ) -> Result<Sticker, RestError> {
        self.request(
            "PATCH",
            &Routes::guild_sticker(guild, sticker),
            Some(to_body(patch)?),
            audit,
            None,
        )
        .await
    }

    pub async fn delete_sticker(
        &self,
        guild: Snowflake,
        sticker: Snowflake,
        audit: Option<&str>,
    ) -> Result<(), RestError> {
        self.request_unit(
            "DELETE",
            &Routes::guild_sticker(guild, sticker),
            None,
            audit,
        )
        .await
    }

    pub async fn sticker_metadata(
        &self,
        sticker: Snowflake,
    ) -> Result<ExpressionMetadata, RestError> {
        self.request("GET", &Routes::sticker_metadata(sticker), None, None, None)
            .await
    }

    // ---- audit log ----

    pub async fn audit_logs(
        &self,
        guild: Snowflake,
        query: &AuditLogQuery,
    ) -> Result<AuditLog, RestError> {
        let q = query_string(&[
            ("limit", query.limit.map(|v| v.to_string())),
            ("before", query.before.map(|v| v.to_string())),
            ("after", query.after.map(|v| v.to_string())),
            ("user_id", query.user_id.map(|v| v.to_string())),
            (
                "action_type",
                query.action_type.map(|v| v.code().to_string()),
            ),
        ]);
        self.request(
            "GET",
            &Routes::guild_audit_logs(guild),
            None,
            None,
            q.as_deref(),
        )
        .await
    }

    // ---- guild ----

    pub async fn edit_guild(
        &self,
        guild: Snowflake,
        patch: &GuildPatch,
        audit: Option<&str>,
    ) -> Result<Guild, RestError> {
        self.request(
            "PATCH",
            &Routes::guild(guild),
            Some(to_body(patch)?),
            audit,
            None,
        )
        .await
    }

    pub async fn current_user_guilds(
        &self,
        with_counts: bool,
    ) -> Result<Vec<UserGuild>, RestError> {
        let query = with_counts.then_some("with_counts=true");
        self.request("GET", Routes::current_user_guilds(), None, None, query)
            .await
    }

    pub async fn leave_guild(&self, guild: Snowflake) -> Result<(), RestError> {
        self.request_unit("DELETE", &Routes::leave_guild(guild), None, None)
            .await
    }

    pub async fn vanity_url(&self, guild: Snowflake) -> Result<VanityUrl, RestError> {
        self.request("GET", &Routes::guild_vanity_url(guild), None, None, None)
            .await
    }

    pub async fn set_vanity_url(
        &self,
        guild: Snowflake,
        code: Option<&str>,
        audit: Option<&str>,
    ) -> Result<VanityUrl, RestError> {
        self.request(
            "PATCH",
            &Routes::guild_vanity_url(guild),
            Some(json!({ "code": code })),
            audit,
            None,
        )
        .await
    }

    pub async fn set_channel_positions(
        &self,
        guild: Snowflake,
        positions: &[ChannelPosition],
        audit: Option<&str>,
    ) -> Result<(), RestError> {
        self.request_unit(
            "PATCH",
            &Routes::guild_channels(guild),
            Some(to_body(&positions)?),
            audit,
        )
        .await
    }

    pub async fn set_role_positions(
        &self,
        guild: Snowflake,
        positions: &[RolePosition],
        audit: Option<&str>,
    ) -> Result<(), RestError> {
        self.request_unit(
            "PATCH",
            &Routes::guild_roles(guild),
            Some(to_body(&positions)?),
            audit,
        )
        .await
    }

    /// Direct `GET /guilds/{g}/roles/{r}`; [`RestClient::role`] lists and
    /// filters instead, which also works on instances without this route.
    pub async fn fetch_role(&self, guild: Snowflake, role: Snowflake) -> Result<Role, RestError> {
        self.request("GET", &Routes::guild_role(guild, role), None, None, None)
            .await
    }

    /// `POST /guilds/{g}/members-search`. The response keeps the SDK shape
    /// (`members`, `total`, paging fields), returned as JSON.
    pub async fn search_members(
        &self,
        guild: Snowflake,
        search: &MemberSearch,
    ) -> Result<Value, RestError> {
        self.request(
            "POST",
            &Routes::guild_members_search(guild),
            Some(to_body(search)?),
            None,
            None,
        )
        .await
    }

    /// Prefix search over `GET /guilds/{g}/members?query=`.
    pub async fn query_members(
        &self,
        guild: Snowflake,
        query: &str,
        limit: u32,
    ) -> Result<Vec<MemberUser>, RestError> {
        let q = query_string(&[
            ("query", Some(query.to_owned())),
            ("limit", Some(limit.clamp(1, 1000).to_string())),
        ]);
        self.request(
            "GET",
            &Routes::guild_members(guild),
            None,
            None,
            q.as_deref(),
        )
        .await
    }

    // ---- permission overwrites ----

    pub async fn set_channel_permission(
        &self,
        channel: Snowflake,
        target: Snowflake,
        kind: OverwriteType,
        allow: u64,
        deny: u64,
        audit: Option<&str>,
    ) -> Result<(), RestError> {
        let body = json!({
            "type": kind,
            "allow": allow.to_string(),
            "deny": deny.to_string(),
        });
        self.request_unit(
            "PUT",
            &Routes::channel_permission(channel, target),
            Some(body),
            audit,
        )
        .await
    }

    pub async fn delete_channel_permission(
        &self,
        channel: Snowflake,
        target: Snowflake,
        audit: Option<&str>,
    ) -> Result<(), RestError> {
        self.request_unit(
            "DELETE",
            &Routes::channel_permission(channel, target),
            None,
            audit,
        )
        .await
    }

    // ---- threads ----

    /// Starts a thread, or a forum/media post when `message` is set.
    pub async fn start_thread(
        &self,
        channel: Snowflake,
        thread: &StartThread,
        audit: Option<&str>,
    ) -> Result<Channel, RestError> {
        self.request(
            "POST",
            &Routes::channel_threads(channel),
            Some(to_body(thread)?),
            audit,
            None,
        )
        .await
    }

    pub async fn start_thread_from_message(
        &self,
        channel: Snowflake,
        message: Snowflake,
        thread: &StartThread,
        audit: Option<&str>,
    ) -> Result<Channel, RestError> {
        self.request(
            "POST",
            &Routes::channel_message_threads(channel, message),
            Some(to_body(thread)?),
            audit,
            None,
        )
        .await
    }

    pub async fn active_threads(&self, guild: Snowflake) -> Result<ThreadsListing, RestError> {
        let value: Value = self
            .request(
                "GET",
                &Routes::guild_active_threads(guild),
                None,
                None,
                None,
            )
            .await?;
        threads_listing(value)
    }

    pub async fn archived_threads(
        &self,
        channel: Snowflake,
        private: bool,
        before: Option<&str>,
        limit: Option<u32>,
    ) -> Result<ThreadsListing, RestError> {
        let path = if private {
            Routes::channel_threads_archived_private(channel)
        } else {
            Routes::channel_threads_archived_public(channel)
        };
        let q = query_string(&[
            ("before", before.map(str::to_owned)),
            ("limit", limit.map(|v| v.to_string())),
        ]);
        let value: Value = self.request("GET", &path, None, None, q.as_deref()).await?;
        threads_listing(value)
    }

    pub async fn joined_archived_private_threads(
        &self,
        channel: Snowflake,
    ) -> Result<ThreadsListing, RestError> {
        let value: Value = self
            .request(
                "GET",
                &Routes::channel_joined_archived_private_threads(channel),
                None,
                None,
                None,
            )
            .await?;
        threads_listing(value)
    }

    pub async fn search_threads(
        &self,
        channel: Snowflake,
        search: &ThreadSearch,
    ) -> Result<ThreadsListing, RestError> {
        let q = query_string(&[
            ("name", search.name.clone()),
            ("tag", search.tag.map(|v| v.to_string())),
            ("archived", search.archived.map(|v| v.to_string())),
            ("sort_by", search.sort_by.clone()),
            ("sort_order", search.sort_order.clone()),
            ("limit", search.limit.map(|v| v.to_string())),
            ("offset", search.offset.map(|v| v.to_string())),
        ]);
        let value: Value = self
            .request(
                "GET",
                &Routes::channel_thread_search(channel),
                None,
                None,
                q.as_deref(),
            )
            .await?;
        threads_listing(value)
    }

    pub async fn thread_members(&self, thread: Snowflake) -> Result<Vec<ThreadMember>, RestError> {
        self.request(
            "GET",
            &Routes::channel_thread_members(thread),
            None,
            None,
            None,
        )
        .await
    }

    pub async fn thread_member(
        &self,
        thread: Snowflake,
        user: Snowflake,
    ) -> Result<ThreadMember, RestError> {
        self.request(
            "GET",
            &Routes::channel_thread_member(thread, user),
            None,
            None,
            None,
        )
        .await
    }

    pub async fn join_thread(&self, thread: Snowflake) -> Result<(), RestError> {
        self.request_unit("PUT", &Routes::channel_thread_member_me(thread), None, None)
            .await
    }

    pub async fn leave_thread(&self, thread: Snowflake) -> Result<(), RestError> {
        self.request_unit(
            "DELETE",
            &Routes::channel_thread_member_me(thread),
            None,
            None,
        )
        .await
    }

    pub async fn add_thread_member(
        &self,
        thread: Snowflake,
        user: Snowflake,
    ) -> Result<(), RestError> {
        self.request_unit(
            "PUT",
            &Routes::channel_thread_member(thread, user),
            None,
            None,
        )
        .await
    }

    pub async fn remove_thread_member(
        &self,
        thread: Snowflake,
        user: Snowflake,
    ) -> Result<(), RestError> {
        self.request_unit(
            "DELETE",
            &Routes::channel_thread_member(thread, user),
            None,
            None,
        )
        .await
    }

    /// Archives/unarchives and locks/unlocks a thread via `PATCH /channels/{id}`.
    pub async fn set_thread_state(
        &self,
        thread: Snowflake,
        archived: Option<bool>,
        locked: Option<bool>,
        audit: Option<&str>,
    ) -> Result<Channel, RestError> {
        let mut body = serde_json::Map::new();
        if let Some(a) = archived {
            body.insert("archived".into(), json!(a));
        }
        if let Some(l) = locked {
            body.insert("locked".into(), json!(l));
        }
        self.request(
            "PATCH",
            &Routes::channel(thread),
            Some(Value::Object(body)),
            audit,
            None,
        )
        .await
    }

    // ---- forum tags ----

    /// Returns the updated channel.
    pub async fn create_tag(
        &self,
        channel: Snowflake,
        tag: &ForumTag,
        audit: Option<&str>,
    ) -> Result<Channel, RestError> {
        self.request(
            "POST",
            &Routes::channel_tags(channel),
            Some(to_body(tag)?),
            audit,
            None,
        )
        .await
    }

    pub async fn edit_tag(
        &self,
        channel: Snowflake,
        tag_id: Snowflake,
        tag: &ForumTag,
        audit: Option<&str>,
    ) -> Result<Channel, RestError> {
        self.request(
            "PUT",
            &Routes::channel_tag(channel, tag_id),
            Some(to_body(tag)?),
            audit,
            None,
        )
        .await
    }

    pub async fn delete_tag(
        &self,
        channel: Snowflake,
        tag_id: Snowflake,
        audit: Option<&str>,
    ) -> Result<Channel, RestError> {
        self.request(
            "DELETE",
            &Routes::channel_tag(channel, tag_id),
            None,
            audit,
            None,
        )
        .await
    }

    // ---- channel extras ----

    /// Generic `PATCH /channels/{id}` for fields without a typed helper.
    pub async fn patch_channel(
        &self,
        channel: Snowflake,
        body: Value,
        audit: Option<&str>,
    ) -> Result<Channel, RestError> {
        self.request("PATCH", &Routes::channel(channel), Some(body), audit, None)
            .await
    }

    pub async fn rtc_regions(&self, channel: Snowflake) -> Result<Vec<Value>, RestError> {
        self.request(
            "GET",
            &Routes::channel_rtc_regions(channel),
            None,
            None,
            None,
        )
        .await
    }

    /// Slowmode state of the bot in `channel`, as returned by the API.
    pub async fn slowmode_state(&self, channel: Snowflake) -> Result<Value, RestError> {
        self.request("GET", &Routes::channel_slowmode(channel), None, None, None)
            .await
    }

    /// Deletes one attachment from a message.
    pub async fn delete_attachment(
        &self,
        channel: Snowflake,
        message: Snowflake,
        attachment: Snowflake,
    ) -> Result<(), RestError> {
        self.request_unit(
            "DELETE",
            &Routes::channel_message_attachment(channel, message, attachment),
            None,
            None,
        )
        .await
    }

    pub async fn message(
        &self,
        channel: Snowflake,
        message: Snowflake,
    ) -> Result<Message, RestError> {
        self.request(
            "GET",
            &Routes::channel_message(channel, message),
            None,
            None,
            None,
        )
        .await
    }

    /// Removes every reaction of one emoji (`emoji` as in `add_reaction`).
    pub async fn clear_reaction(
        &self,
        channel: Snowflake,
        message: Snowflake,
        emoji: &str,
    ) -> Result<(), RestError> {
        self.request_unit(
            "DELETE",
            &Routes::channel_reaction(channel, message, emoji),
            None,
            None,
        )
        .await
    }

    /// `POST /search/messages`; body and response keep the SDK wire shape.
    pub async fn search_messages(&self, body: Value) -> Result<Value, RestError> {
        self.request(
            "POST",
            Routes::search_messages(),
            non_empty(body),
            None,
            None,
        )
        .await
    }

    // ---- application / users ----

    pub async fn application(&self) -> Result<Application, RestError> {
        self.request("GET", Routes::applications_me(), None, None, None)
            .await
    }

    pub async fn oauth2_application(&self) -> Result<Application, RestError> {
        self.request("GET", Routes::oauth2_applications_me(), None, None, None)
            .await
    }

    /// `GET /users/{id}/profile`, guild-scoped when `guild` is set.
    pub async fn user_profile(
        &self,
        user: Snowflake,
        guild: Option<Snowflake>,
    ) -> Result<Value, RestError> {
        let q = guild.map(|g| format!("guild_id={g}"));
        self.request(
            "GET",
            &Routes::user_profile(user, None),
            None,
            None,
            q.as_deref(),
        )
        .await
    }

    /// Edits the bot user (`username`, `avatar` data URI, ...).
    pub async fn edit_current_user(&self, body: Value) -> Result<flush_core::User, RestError> {
        self.request("PATCH", Routes::current_user(), Some(body), None, None)
            .await
    }

    /// Escape hatch for routes without a typed method. `path` is relative to
    /// `/v1` (use [`Routes`]); `query` is an already-encoded query string.
    pub async fn raw(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        query: Option<&str>,
        audit: Option<&str>,
    ) -> Result<Value, RestError> {
        self.request(method, path, body, audit, query).await
    }
}

/// `GET /guilds/{id}/threads/active` may answer a bare channel array.
fn threads_listing(value: Value) -> Result<ThreadsListing, RestError> {
    let value = match value {
        Value::Array(threads) => json!({ "threads": threads }),
        Value::Null => json!({}),
        other => other,
    };
    serde_json::from_value(value).map_err(|e| RestError::BadResponse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_string_skips_none_and_encodes() {
        let q = query_string(&[
            ("name", Some("a b&c".into())),
            ("tag", None),
            ("limit", Some("5".into())),
        ]);
        assert_eq!(q.as_deref(), Some("name=a%20b%26c&limit=5"));
        assert_eq!(query_string(&[("x", None)]), None);
    }

    #[test]
    fn threads_listing_accepts_array_and_object() {
        let arr = json!([{ "id": "1", "type": 11 }]);
        assert_eq!(threads_listing(arr).unwrap().threads.len(), 1);
        let obj = json!({ "threads": null, "members": null, "has_more": true });
        let listing = threads_listing(obj).unwrap();
        assert!(listing.threads.is_empty() && listing.has_more);
    }

    #[test]
    fn webhook_patch_serializes_clear_avatar() {
        let patch = WebhookPatch {
            avatar: Some(None),
            ..Default::default()
        };
        assert_eq!(to_body(&patch).unwrap(), json!({ "avatar": null }));
    }
}
