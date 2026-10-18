//! Cliente REST `/v1` sobre surf (async-std + h1/rustls).
//!
//! Uso real espelhado:
//! - Jully `Bot.js::apiRequest`: `Authorization: Bot <token>`, JSON,
//!   retry em 429 via `Retry-After`, retry de rede 3x com backoff.
//! - Sour `@fluxerjs/core`: reply via `POST /channels/{id}/messages` com
//!   `message_reference {channel_id, message_id, guild_id?}`;
//!   `bulkDelete(n)` = lista recentes + DELETE único ou POST bulk-delete;
//!   ban = `PUT /guilds/{g}/bans/{u}` com `reason/delete_message_days`;
//!   timeout = `PATCH /guilds/{g}/members/{u}` com
//!   `communication_disabled_until/timeout_reason`;
//!   roles via `GET /guilds/{g}/roles` (sem GET individual confiável).

use log::{debug, warn};
use serde::{Deserialize, Serialize};

use flush_core::{
    Channel, Guild, Invite, MemberUser, Message, MessageSend, PartialUser, ReactionEmoji, Role,
    Snowflake, User,
};

use crate::error::{ApiErrorBody, RestError};
use crate::routes::Routes;

const DEFAULT_API_BASE: &str = "https://api.fluxer.app";
const DEFAULT_ORIGIN: &str = "https://fluxer.app";
const DEFAULT_GATEWAY: &str = "wss://gateway.fluxer.app";

#[derive(Debug, Clone)]
pub struct RestConfig {
    pub token: String,
    /// Base da API pública (ex.: `https://api.fluxer.app`). Se ausente, usa discovery.
    pub api_base: Option<String>,
    /// Origem para discovery (`GET /.well-known/fluxer`). Padrão fluxer.app.
    pub origin: Option<String>,
    /// Gateway explícito (pula discovery do gateway).
    pub gateway_url: Option<String>,
    pub audit_reason: Option<String>,
}

impl RestConfig {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            api_base: None,
            origin: None,
            gateway_url: None,
            audit_reason: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct DiscoveryDoc {
    #[serde(default)]
    endpoints: Option<DiscoveryEndpoints>,
}

#[derive(Debug, Clone, Deserialize)]
struct DiscoveryEndpoints {
    #[serde(default)]
    api_public: Option<String>,
    #[serde(default)]
    gateway: Option<String>,
}

#[derive(Debug, Clone)]
pub struct InstanceEndpoints {
    pub api_public: String,
    pub gateway: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayBotInfo {
    pub url: String,
    #[serde(default)]
    pub shards: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GuildBan {
    pub user: PartialUser,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub moderator_id: Option<String>,
    #[serde(default)]
    pub banned_at: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BanOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete_message_days: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete_message_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ban_duration_seconds: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MemberPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nick: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<Snowflake>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub communication_disabled_until: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mute: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deaf: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct EditMessageBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embeds: Option<Vec<flush_core::Embed>>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ChannelPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_user: Option<u64>,
}
#[derive(Debug, Clone, Serialize)]
struct BulkDeleteBody {
    message_ids: Vec<Snowflake>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RolePatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hoist: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mentionable: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ChannelCreate {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_user: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct InviteCreate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_age_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temporary: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unique: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct RestClient {
    http: surf::Client,
    token: String,
    api_base: String,
    gateway_url: String,
    audit_reason: Option<String>,
}

impl RestClient {
    pub async fn new(config: RestConfig) -> Result<Self, RestError> {
        if config.token.trim().is_empty() {
            return Err(RestError::Config("token vazio".into()));
        }
        let (api_base, gateway_url) = if let Some(base) = config.api_base.clone() {
            let gw = config
                .gateway_url
                .clone()
                .unwrap_or_else(|| DEFAULT_GATEWAY.into());
            (trim_slash(&base).to_owned(), gw)
        } else {
            let origin = config
                .origin
                .clone()
                .unwrap_or_else(|| DEFAULT_ORIGIN.into());
            match Self::discover(&origin).await {
                Ok(ep) => {
                    let gw = config.gateway_url.clone().unwrap_or(ep.gateway);
                    (ep.api_public, gw)
                }
                Err(e) => {
                    warn!("rest: discovery falhou em {origin} ({e}), usando defaults");
                    (
                        DEFAULT_API_BASE.to_owned(),
                        config
                            .gateway_url
                            .clone()
                            .unwrap_or_else(|| DEFAULT_GATEWAY.into()),
                    )
                }
            }
        };
        debug!("rest: api_base={api_base} gateway={gateway_url}");
        Ok(Self {
            http: surf::Client::new(),
            token: config.token,
            api_base,
            gateway_url,
            audit_reason: config.audit_reason,
        })
    }

    pub fn api_base(&self) -> &str {
        &self.api_base
    }

    pub fn gateway_url(&self) -> &str {
        &self.gateway_url
    }

    pub async fn discover(origin: &str) -> Result<InstanceEndpoints, RestError> {
        let url = format!("{}/.well-known/fluxer", trim_slash(origin));
        let mut doc: DiscoveryDoc = surf::get(&url)
            .recv_json()
            .await
            .map_err(|e| RestError::Transport(format!("discovery {url}: {e}")))?;
        let _ = &mut doc;
        let ep = doc
            .endpoints
            .ok_or_else(|| RestError::BadResponse("sem endpoints".into()))?;
        Ok(InstanceEndpoints {
            api_public: ep.api_public.unwrap_or_else(|| DEFAULT_API_BASE.into()),
            gateway: ep.gateway.unwrap_or_else(|| DEFAULT_GATEWAY.into()),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/v1{path}", self.api_base)
    }

    fn auth(&self, audit: Option<&str>) -> Vec<(String, String)> {
        let mut h = vec![(String::from("Authorization"), format!("Bot {}", self.token))];
        let reason = audit.or(self.audit_reason.as_deref());
        if let Some(r) = reason {
            h.push((String::from("X-Audit-Log-Reason"), r.to_owned()));
        }
        h
    }

    async fn request<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
        audit: Option<&str>,
        query: Option<&str>,
    ) -> Result<T, RestError> {
        let mut attempt = 0u8;
        loop {
            let mut url = self.url(path);
            if let Some(q) = query {
                url.push('?');
                url.push_str(q);
            }
            let req = match method {
                "GET" => self.http.get(&url),
                "POST" => self.http.post(&url),
                "PUT" => self.http.put(&url),
                "PATCH" => self.http.patch(&url),
                "DELETE" => self.http.delete(&url),
                _ => return Err(RestError::Config(format!("método {method}"))),
            };
            let mut req = req;
            for (k, v) in self.auth(audit) {
                req = req.header(k.as_str(), v.as_str());
            }
            let req = match &body {
                Some(b) => req
                    .body_json(b)
                    .map_err(|e| RestError::Transport(format!("serializar body: {e}")))?,
                None => req,
            };
            let mut resp = req.await.map_err(|e| RestError::Transport(e.to_string()))?;
            let status = u16::from(resp.status());
            if status == 429 {
                let retry = retry_after(&resp).unwrap_or(1);
                if attempt == 0 {
                    warn!("rest: 429 em {method} {path}, retry em {retry}s");
                    async_std::task::sleep(std::time::Duration::from_secs(retry)).await;
                    attempt += 1;
                    continue;
                }
                return Err(RestError::RateLimited {
                    retry_after_secs: retry,
                });
            }
            if (200..300).contains(&status) {
                let text = resp
                    .body_string()
                    .await
                    .map_err(|e| RestError::Transport(e.to_string()))?;
                if text.trim().is_empty() {
                    return serde_json::from_value(serde_json::Value::Null)
                        .map_err(|e| RestError::BadResponse(e.to_string()));
                }
                return serde_json::from_str(&text)
                    .map_err(|e| RestError::BadResponse(e.to_string()));
            }
            let text = resp
                .body_string()
                .await
                .map_err(|e| RestError::Transport(e.to_string()))?;
            let parsed: Result<ApiErrorBody, _> = serde_json::from_str(&text);
            return match parsed {
                Ok(b) => Err(RestError::Api {
                    status,
                    code: b.code,
                    message: b.message,
                }),
                Err(_) => Err(RestError::Api {
                    status,
                    code: "UNKNOWN".into(),
                    message: text.chars().take(300).collect(),
                }),
            };
        }
    }

    async fn request_unit(
        &self,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
        audit: Option<&str>,
    ) -> Result<(), RestError> {
        let v: serde_json::Value = self.request(method, path, body, audit, None).await?;
        let _ = v;
        Ok(())
    }

    // ---- discovery/gateway ----

    pub async fn gateway_bot(&self) -> Result<GatewayBotInfo, RestError> {
        self.request("GET", Routes::gateway_bot(), None, None, None)
            .await
    }

    // ---- self/users ----

    pub async fn current_user(&self) -> Result<User, RestError> {
        self.request("GET", Routes::current_user(), None, None, None)
            .await
    }

    pub async fn user(&self, id: Snowflake) -> Result<PartialUser, RestError> {
        self.request("GET", &Routes::user(id), None, None, None)
            .await
    }

    // ---- guilds/roles ----

    pub async fn guild(&self, id: Snowflake) -> Result<Guild, RestError> {
        self.request("GET", &Routes::guild(id), None, None, None)
            .await
    }

    pub async fn guild_channels(&self, id: Snowflake) -> Result<Vec<Channel>, RestError> {
        self.request("GET", &Routes::guild_channels(id), None, None, None)
            .await
    }

    pub async fn roles(&self, guild: Snowflake) -> Result<Vec<Role>, RestError> {
        self.request("GET", &Routes::guild_roles(guild), None, None, None)
            .await
    }

    pub async fn role(
        &self,
        guild: Snowflake,
        role_id: Snowflake,
    ) -> Result<Option<Role>, RestError> {
        // Sem GET individual confiável: lista e acha pelo ID (padrão do Sour).
        let roles: Vec<Role> = self.roles(guild).await?;
        Ok(roles.into_iter().find(|r| r.id == role_id))
    }

    // ---- members ----

    pub async fn member(&self, guild: Snowflake, user: Snowflake) -> Result<MemberUser, RestError> {
        self.request("GET", &Routes::guild_member(guild, user), None, None, None)
            .await
    }

    pub async fn me_member(&self, guild: Snowflake) -> Result<MemberUser, RestError> {
        self.request("GET", &Routes::guild_member_me(guild), None, None, None)
            .await
    }

    pub async fn list_members(
        &self,
        guild: Snowflake,
        limit: u32,
        after: Option<Snowflake>,
    ) -> Result<Vec<MemberUser>, RestError> {
        let limit = limit.clamp(1, 1000);
        let mut q = format!("limit={limit}");
        if let Some(a) = after {
            q.push_str(&format!("&after={a}"));
        }
        self.request("GET", &Routes::guild_members(guild), None, None, Some(&q))
            .await
    }

    pub async fn all_members(&self, guild: Snowflake) -> Result<Vec<MemberUser>, RestError> {
        // Paginação `after` (padrão do Sour `darcargo`, lote 1000).
        let mut out = Vec::new();
        let mut after = None;
        loop {
            let page: Vec<MemberUser> = self.list_members(guild, 1000, after).await?;
            let done = page.len() < 1000;
            after = page.last().map(|m| m.user.id);
            out.extend(page);
            if done {
                break;
            }
        }
        Ok(out)
    }

    pub async fn patch_member(
        &self,
        guild: Snowflake,
        user: Snowflake,
        patch: &MemberPatch,
        audit: Option<&str>,
    ) -> Result<MemberUser, RestError> {
        let body = serde_json::to_value(patch).map_err(|e| RestError::Transport(e.to_string()))?;
        self.request(
            "PATCH",
            &Routes::guild_member(guild, user),
            Some(body),
            audit,
            None,
        )
        .await
    }

    pub async fn set_timeout(
        &self,
        guild: Snowflake,
        user: Snowflake,
        until_iso: Option<String>,
        reason: Option<&str>,
        audit: Option<&str>,
    ) -> Result<MemberUser, RestError> {
        let patch = MemberPatch {
            communication_disabled_until: Some(until_iso),
            timeout_reason: reason.map(str::to_owned),
            ..Default::default()
        };
        self.patch_member(guild, user, &patch, audit).await
    }

    pub async fn add_role(
        &self,
        guild: Snowflake,
        user: Snowflake,
        role: Snowflake,
        audit: Option<&str>,
    ) -> Result<(), RestError> {
        self.request_unit(
            "PUT",
            &Routes::guild_member_role(guild, user, role),
            None,
            audit,
        )
        .await
    }

    pub async fn remove_role(
        &self,
        guild: Snowflake,
        user: Snowflake,
        role: Snowflake,
        audit: Option<&str>,
    ) -> Result<(), RestError> {
        self.request_unit(
            "DELETE",
            &Routes::guild_member_role(guild, user, role),
            None,
            audit,
        )
        .await
    }

    pub async fn kick(
        &self,
        guild: Snowflake,
        user: Snowflake,
        audit: Option<&str>,
    ) -> Result<(), RestError> {
        self.request_unit("DELETE", &Routes::guild_member(guild, user), None, audit)
            .await
    }

    // ---- bans ----

    pub async fn bans(&self, guild: Snowflake) -> Result<Vec<GuildBan>, RestError> {
        self.request("GET", &Routes::guild_bans(guild), None, None, None)
            .await
    }

    pub async fn ban(
        &self,
        guild: Snowflake,
        user: Snowflake,
        opts: &BanOptions,
        audit: Option<&str>,
    ) -> Result<(), RestError> {
        let body = serde_json::to_value(opts).map_err(|e| RestError::Transport(e.to_string()))?;
        let body = if body.as_object().map(|o| o.is_empty()).unwrap_or(true) {
            None
        } else {
            Some(body)
        };
        self.request_unit("PUT", &Routes::guild_ban(guild, user), body, audit)
            .await
    }

    pub async fn unban(
        &self,
        guild: Snowflake,
        user: Snowflake,
        audit: Option<&str>,
    ) -> Result<(), RestError> {
        self.request_unit("DELETE", &Routes::guild_ban(guild, user), None, audit)
            .await
    }

    // ---- channels/messages ----

    pub async fn channel(&self, id: Snowflake) -> Result<Channel, RestError> {
        self.request("GET", &Routes::channel(id), None, None, None)
            .await
    }

    pub async fn edit_channel(
        &self,
        id: Snowflake,
        patch: &ChannelPatch,
        audit: Option<&str>,
    ) -> Result<Channel, RestError> {
        let body = serde_json::to_value(patch).map_err(|e| RestError::Transport(e.to_string()))?;
        self.request("PATCH", &Routes::channel(id), Some(body), audit, None)
            .await
    }

    pub async fn set_slowmode(
        &self,
        id: Snowflake,
        seconds: u64,
        audit: Option<&str>,
    ) -> Result<Channel, RestError> {
        let patch = ChannelPatch {
            rate_limit_per_user: Some(seconds),
        };
        self.edit_channel(id, &patch, audit).await
    }

    pub async fn send_message(
        &self,
        channel: Snowflake,
        payload: &MessageSend,
    ) -> Result<Message, RestError> {
        payload
            .validate()
            .map_err(|e| RestError::Config(e.to_string()))?;
        let body =
            serde_json::to_value(payload).map_err(|e| RestError::Transport(e.to_string()))?;
        self.request(
            "POST",
            &Routes::channel_messages(channel),
            Some(body),
            None,
            None,
        )
        .await
    }

    pub async fn reply(
        &self,
        channel: Snowflake,
        reply_to: &Message,
        mut payload: MessageSend,
        ping: bool,
    ) -> Result<Message, RestError> {
        // Espelha `replyToMessage` do SDK: reference + allowed_mentions.replied_user.
        if payload.message_reference.is_none() {
            payload.message_reference = Some(flush_core::MessageReference {
                channel_id: channel,
                message_id: reply_to.id,
                guild_id: reply_to.guild_id,
                kind: Some(0),
            });
        }
        if !ping {
            let mut am = payload.allowed_mentions.unwrap_or_default();
            am.replied_user = Some(false);
            payload.allowed_mentions = Some(am);
        }
        self.send_message(channel, &payload).await
    }

    pub async fn edit_message(
        &self,
        channel: Snowflake,
        message: Snowflake,
        body: &EditMessageBody,
    ) -> Result<Message, RestError> {
        let v = serde_json::to_value(body).map_err(|e| RestError::Transport(e.to_string()))?;
        self.request(
            "PATCH",
            &Routes::channel_message(channel, message),
            Some(v),
            None,
            None,
        )
        .await
    }

    pub async fn delete_message(
        &self,
        channel: Snowflake,
        message: Snowflake,
    ) -> Result<(), RestError> {
        self.request_unit(
            "DELETE",
            &Routes::channel_message(channel, message),
            None,
            None,
        )
        .await
    }

    pub async fn list_messages(
        &self,
        channel: Snowflake,
        limit: u32,
    ) -> Result<Vec<Message>, RestError> {
        let limit = limit.clamp(1, 100);
        let q = format!("limit={limit}");
        self.request(
            "GET",
            &Routes::channel_messages(channel),
            None,
            None,
            Some(&q),
        )
        .await
    }

    /// `bulkDelete` do SDK: 1 id = DELETE único; 2-100 = POST bulk-delete.
    pub async fn bulk_delete(
        &self,
        channel: Snowflake,
        ids: &[Snowflake],
    ) -> Result<Vec<Snowflake>, RestError> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        if ids.len() == 1 {
            self.delete_message(channel, ids[0]).await?;
            return Ok(ids.to_vec());
        }
        if ids.len() > 100 {
            return Err(RestError::Config("bulkDelete: 1-100 ids".into()));
        }
        let body = serde_json::to_value(BulkDeleteBody {
            message_ids: ids.to_vec(),
        })
        .map_err(|e| RestError::Transport(e.to_string()))?;
        let _: serde_json::Value = self
            .request(
                "POST",
                &Routes::channel_bulk_delete(channel),
                Some(body),
                None,
                None,
            )
            .await?;
        Ok(ids.to_vec())
    }

    /// Apaga as N recentes (padrão do `!clear`): lista + bulk-delete.
    pub async fn bulk_delete_recent(
        &self,
        channel: Snowflake,
        count: u32,
    ) -> Result<Vec<Snowflake>, RestError> {
        let count = count.clamp(1, 100);
        let msgs = self.list_messages(channel, count).await?;
        let ids: Vec<Snowflake> = msgs.into_iter().map(|m| m.id).collect();
        self.bulk_delete(channel, &ids).await
    }

    // ---- reactions ----

    pub async fn add_reaction(
        &self,
        channel: Snowflake,
        message: Snowflake,
        emoji: &ReactionEmoji,
        audit: Option<&str>,
    ) -> Result<(), RestError> {
        self.request_unit(
            "PUT",
            &Routes::channel_reaction_me(channel, message, &emoji.encoded()),
            None,
            audit,
        )
        .await
    }

    /// Remove a própria reação (`DELETE .../reactions/{emoji}/@me`).
    pub async fn remove_own_reaction(
        &self,
        channel: Snowflake,
        message: Snowflake,
        emoji: &ReactionEmoji,
    ) -> Result<(), RestError> {
        self.request_unit(
            "DELETE",
            &Routes::channel_reaction_me(channel, message, &emoji.encoded()),
            None,
            None,
        )
        .await
    }

    /// Remove a reação de um usuário específico (precisa MANAGE_MESSAGES).
    pub async fn remove_reaction(
        &self,
        channel: Snowflake,
        message: Snowflake,
        emoji: &ReactionEmoji,
        user: Snowflake,
        audit: Option<&str>,
    ) -> Result<(), RestError> {
        self.request_unit(
            "DELETE",
            &Routes::channel_reaction_user(channel, message, &emoji.encoded(), user),
            None,
            audit,
        )
        .await
    }

    /// Limpa todas as reações da mensagem (precisa MANAGE_MESSAGES).
    pub async fn clear_reactions(
        &self,
        channel: Snowflake,
        message: Snowflake,
        audit: Option<&str>,
    ) -> Result<(), RestError> {
        self.request_unit(
            "DELETE",
            &Routes::channel_reactions(channel, message),
            None,
            audit,
        )
        .await
    }

    pub async fn reaction_users(
        &self,
        channel: Snowflake,
        message: Snowflake,
        emoji: &ReactionEmoji,
        limit: u32,
    ) -> Result<Vec<PartialUser>, RestError> {
        let limit = limit.clamp(1, 100);
        let q = format!("limit={limit}");
        self.request(
            "GET",
            &Routes::channel_reaction_users(channel, message, &emoji.encoded()),
            None,
            None,
            Some(&q),
        )
        .await
    }

    // ---- pins ----

    pub async fn pinned_messages(&self, channel: Snowflake) -> Result<Vec<Message>, RestError> {
        self.request("GET", &Routes::channel_pins(channel), None, None, None)
            .await
    }

    pub async fn pin_message(
        &self,
        channel: Snowflake,
        message: Snowflake,
        audit: Option<&str>,
    ) -> Result<(), RestError> {
        self.request_unit("PUT", &Routes::channel_pin(channel, message), None, audit)
            .await
    }

    pub async fn unpin_message(
        &self,
        channel: Snowflake,
        message: Snowflake,
        audit: Option<&str>,
    ) -> Result<(), RestError> {
        self.request_unit(
            "DELETE",
            &Routes::channel_pin(channel, message),
            None,
            audit,
        )
        .await
    }

    // ---- invites ----

    pub async fn channel_invites(&self, channel: Snowflake) -> Result<Vec<Invite>, RestError> {
        self.request("GET", &Routes::channel_invites(channel), None, None, None)
            .await
    }

    pub async fn guild_invites(&self, guild: Snowflake) -> Result<Vec<Invite>, RestError> {
        self.request("GET", &Routes::guild_invites(guild), None, None, None)
            .await
    }

    pub async fn create_invite(
        &self,
        channel: Snowflake,
        opts: &InviteCreate,
        audit: Option<&str>,
    ) -> Result<Invite, RestError> {
        let body = serde_json::to_value(opts).map_err(|e| RestError::Transport(e.to_string()))?;
        let body = if body.as_object().map(|o| o.is_empty()).unwrap_or(true) {
            None
        } else {
            Some(body)
        };
        self.request("POST", &Routes::channel_invites(channel), body, audit, None)
            .await
    }

    pub async fn invite(&self, code: &str) -> Result<Invite, RestError> {
        self.request("GET", &Routes::invite(code), None, None, None)
            .await
    }

    pub async fn delete_invite(
        &self,
        code: &str,
        audit: Option<&str>,
    ) -> Result<Invite, RestError> {
        self.request("DELETE", &Routes::invite(code), None, audit, None)
            .await
    }

    // ---- roles CRUD ----

    pub async fn create_role(
        &self,
        guild: Snowflake,
        patch: &RolePatch,
        audit: Option<&str>,
    ) -> Result<Role, RestError> {
        let body = serde_json::to_value(patch).map_err(|e| RestError::Transport(e.to_string()))?;
        self.request("POST", &Routes::guild_roles(guild), Some(body), audit, None)
            .await
    }

    pub async fn edit_role(
        &self,
        guild: Snowflake,
        role: Snowflake,
        patch: &RolePatch,
        audit: Option<&str>,
    ) -> Result<Role, RestError> {
        let body = serde_json::to_value(patch).map_err(|e| RestError::Transport(e.to_string()))?;
        self.request(
            "PATCH",
            &Routes::guild_role(guild, role),
            Some(body),
            audit,
            None,
        )
        .await
    }

    pub async fn delete_role(
        &self,
        guild: Snowflake,
        role: Snowflake,
        audit: Option<&str>,
    ) -> Result<(), RestError> {
        self.request_unit("DELETE", &Routes::guild_role(guild, role), None, audit)
            .await
    }

    // ---- channels CRUD / DM ----

    pub async fn create_channel(
        &self,
        guild: Snowflake,
        create: &ChannelCreate,
        audit: Option<&str>,
    ) -> Result<Channel, RestError> {
        let body = serde_json::to_value(create).map_err(|e| RestError::Transport(e.to_string()))?;
        self.request(
            "POST",
            &Routes::guild_channels(guild),
            Some(body),
            audit,
            None,
        )
        .await
    }

    pub async fn delete_channel(
        &self,
        channel: Snowflake,
        audit: Option<&str>,
    ) -> Result<Channel, RestError> {
        self.request("DELETE", &Routes::channel(channel), None, audit, None)
            .await
    }

    /// Abre (cria no primeiro contato) um canal de DM com o usuário.
    pub async fn create_dm(&self, user: Snowflake) -> Result<Channel, RestError> {
        let body = serde_json::json!({ "recipient_id": user.to_string() });
        self.request("POST", Routes::dm_channels(), Some(body), None, None)
            .await
    }

    /// Apelido do membro (`None` remove). Usa PATCH member com `nick`.
    pub async fn set_nick(
        &self,
        guild: Snowflake,
        user: Snowflake,
        nick: Option<&str>,
        audit: Option<&str>,
    ) -> Result<MemberUser, RestError> {
        let patch = MemberPatch {
            nick: Some(nick.map(str::to_owned)),
            ..Default::default()
        };
        self.patch_member(guild, user, &patch, audit).await
    }

    pub async fn typing(&self, channel: Snowflake) -> Result<(), RestError> {
        // Erro de typing nunca deve quebrar comando (padrão Jully/Sour).
        match self
            .request::<serde_json::Value>(
                "POST",
                &Routes::channel_typing(channel),
                None,
                None,
                None,
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(_) => Ok(()),
        }
    }
}

fn trim_slash(s: &str) -> &str {
    s.trim_end_matches('/')
}

fn retry_after(resp: &surf::Response) -> Option<u64> {
    resp.header("retry-after")
        .and_then(|v| v.get(0))
        .and_then(|hv| hv.as_str().trim().parse::<u64>().ok())
}
