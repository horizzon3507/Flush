//! `/v1` routes used by Flush.
//!
//! Mirrors `Routes` in `@fluxerjs/types` 3.3.0:
//! channels, messages, threads, guilds, members, roles, bans, webhooks,
//! expressions, users, applications, gateway.

use flush_core::Snowflake;

fn urlenc(s: &str) -> String {
    const SAFE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.~";
    let mut out = String::new();
    for b in s.as_bytes() {
        if SAFE.contains(b) {
            out.push(*b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub(crate) fn encode_component(s: &str) -> String {
    urlenc(s)
}

pub struct Routes;

impl Routes {
    fn s(id: Snowflake) -> String {
        id.to_string()
    }

    pub fn channel(id: Snowflake) -> String {
        format!("/channels/{}", Self::s(id))
    }

    pub fn channel_messages(id: Snowflake) -> String {
        format!("/channels/{}/messages", Self::s(id))
    }

    pub fn channel_message(channel: Snowflake, message: Snowflake) -> String {
        format!(
            "/channels/{}/messages/{}",
            Self::s(channel),
            Self::s(message)
        )
    }

    pub fn channel_bulk_delete(id: Snowflake) -> String {
        format!("/channels/{}/messages/bulk-delete", Self::s(id))
    }

    pub fn channel_typing(id: Snowflake) -> String {
        format!("/channels/{}/typing", Self::s(id))
    }

    pub fn channel_pins(id: Snowflake) -> String {
        format!("/channels/{}/messages/pins", Self::s(id))
    }

    pub fn channel_pin(channel: Snowflake, message: Snowflake) -> String {
        format!("/channels/{}/pins/{}", Self::s(channel), Self::s(message))
    }

    pub fn channel_reactions(channel: Snowflake, message: Snowflake) -> String {
        format!(
            "/channels/{}/messages/{}/reactions",
            Self::s(channel),
            Self::s(message)
        )
    }

    /// `emoji` already URL-encoded (`ReactionEmoji::encoded()`).
    pub fn channel_reaction(channel: Snowflake, message: Snowflake, emoji: &str) -> String {
        format!(
            "/channels/{}/messages/{}/reactions/{emoji}",
            Self::s(channel),
            Self::s(message)
        )
    }

    pub fn channel_reaction_me(channel: Snowflake, message: Snowflake, emoji: &str) -> String {
        format!("{}/@me", Self::channel_reaction(channel, message, emoji))
    }

    pub fn channel_reaction_user(
        channel: Snowflake,
        message: Snowflake,
        emoji: &str,
        user: Snowflake,
    ) -> String {
        format!(
            "{}/{}",
            Self::channel_reaction(channel, message, emoji),
            Self::s(user)
        )
    }

    pub fn channel_reaction_users(channel: Snowflake, message: Snowflake, emoji: &str) -> String {
        format!("{}/users", Self::channel_reaction(channel, message, emoji))
    }

    pub fn channel_invites(id: Snowflake) -> String {
        format!("/channels/{}/invites", Self::s(id))
    }

    pub fn guild_invites(id: Snowflake) -> String {
        format!("/guilds/{}/invites", Self::s(id))
    }

    pub fn invite(code: &str) -> String {
        format!("/invites/{}", urlenc(code))
    }

    pub fn guild_role(guild: Snowflake, role: Snowflake) -> String {
        format!("/guilds/{}/roles/{}", Self::s(guild), Self::s(role))
    }

    pub fn dm_channels() -> &'static str {
        "/users/@me/channels"
    }

    pub fn guild(id: Snowflake) -> String {
        format!("/guilds/{}", Self::s(id))
    }

    pub fn guild_channels(id: Snowflake) -> String {
        format!("/guilds/{}/channels", Self::s(id))
    }
    pub fn guild_members(id: Snowflake) -> String {
        format!("/guilds/{}/members", Self::s(id))
    }

    pub fn guild_member(guild: Snowflake, user: Snowflake) -> String {
        format!("/guilds/{}/members/{}", Self::s(guild), Self::s(user))
    }

    pub fn guild_member_me(guild: Snowflake) -> String {
        format!("/guilds/{}/members/@me", Self::s(guild))
    }

    pub fn guild_member_role(guild: Snowflake, user: Snowflake, role: Snowflake) -> String {
        format!(
            "/guilds/{}/members/{}/roles/{}",
            Self::s(guild),
            Self::s(user),
            Self::s(role)
        )
    }

    pub fn guild_roles(id: Snowflake) -> String {
        format!("/guilds/{}/roles", Self::s(id))
    }

    pub fn guild_bans(id: Snowflake) -> String {
        format!("/guilds/{}/bans", Self::s(id))
    }

    pub fn guild_ban(guild: Snowflake, user: Snowflake) -> String {
        format!("/guilds/{}/bans/{}", Self::s(guild), Self::s(user))
    }

    pub fn user(id: Snowflake) -> String {
        format!("/users/{}", Self::s(id))
    }

    pub fn current_user() -> &'static str {
        "/users/@me"
    }

    pub fn gateway_bot() -> &'static str {
        "/gateway/bot"
    }

    pub fn channel_bulk_delete_mine(id: Snowflake) -> String {
        format!("/channels/{id}/messages/bulk-delete-mine")
    }

    pub fn channel_messages_ack(id: Snowflake) -> String {
        format!("/channels/{id}/messages/ack")
    }

    pub fn channel_messages_purge(id: Snowflake) -> String {
        format!("/channels/{id}/messages/purge")
    }

    pub fn channel_pins_ack(id: Snowflake) -> String {
        format!("/channels/{id}/pins/ack")
    }

    pub fn channel_attachments(id: Snowflake) -> String {
        format!("/channels/{id}/attachments")
    }

    pub fn channel_attachments_complete(id: Snowflake) -> String {
        format!("/channels/{id}/attachments/complete")
    }

    pub fn channel_webhooks(id: Snowflake) -> String {
        format!("/channels/{id}/webhooks")
    }

    pub fn channel_rtc_regions(id: Snowflake) -> String {
        format!("/channels/{id}/rtc-regions")
    }

    pub fn channel_slowmode(id: Snowflake) -> String {
        format!("/channels/{id}/slowmode")
    }

    pub fn channel_permission(channel: Snowflake, overwrite: Snowflake) -> String {
        format!("/channels/{channel}/permissions/{overwrite}")
    }

    pub fn channel_recipient(channel: Snowflake, user: Snowflake) -> String {
        format!("/channels/{channel}/recipients/{user}")
    }

    pub fn channel_message_attachment(
        channel: Snowflake,
        message: Snowflake,
        attachment: Snowflake,
    ) -> String {
        format!("/channels/{channel}/messages/{message}/attachments/{attachment}")
    }

    pub fn channel_threads(id: Snowflake) -> String {
        format!("/channels/{id}/threads")
    }

    pub fn channel_message_threads(channel: Snowflake, message: Snowflake) -> String {
        format!("/channels/{channel}/messages/{message}/threads")
    }

    pub fn channel_threads_archived_public(id: Snowflake) -> String {
        format!("/channels/{id}/threads/archived/public")
    }

    pub fn channel_threads_archived_private(id: Snowflake) -> String {
        format!("/channels/{id}/threads/archived/private")
    }

    pub fn channel_joined_archived_private_threads(id: Snowflake) -> String {
        format!("/channels/{id}/users/@me/threads/archived/private")
    }

    pub fn channel_thread_search(id: Snowflake) -> String {
        format!("/channels/{id}/threads/search")
    }

    pub fn channel_thread_members(id: Snowflake) -> String {
        format!("/channels/{id}/thread-members")
    }

    pub fn channel_thread_member_me(id: Snowflake) -> String {
        format!("/channels/{id}/thread-members/@me")
    }

    pub fn channel_thread_member_me_settings(id: Snowflake) -> String {
        format!("/channels/{id}/thread-members/@me/settings")
    }

    pub fn channel_thread_member(channel: Snowflake, user: Snowflake) -> String {
        format!("/channels/{channel}/thread-members/{user}")
    }

    pub fn channel_tags(id: Snowflake) -> String {
        format!("/channels/{id}/tags")
    }

    pub fn channel_tag(channel: Snowflake, tag: Snowflake) -> String {
        format!("/channels/{channel}/tags/{tag}")
    }

    pub fn guild_delete(id: Snowflake) -> String {
        format!("/guilds/{id}/delete")
    }

    pub fn guild_vanity_url(id: Snowflake) -> String {
        format!("/guilds/{id}/vanity-url")
    }

    pub fn guild_transfer_ownership(id: Snowflake) -> String {
        format!("/guilds/{id}/transfer-ownership")
    }

    pub fn guild_roles_hoist_positions(id: Snowflake) -> String {
        format!("/guilds/{id}/roles/hoist-positions")
    }

    pub fn guild_emojis_bulk(id: Snowflake) -> String {
        format!("/guilds/{id}/emojis/bulk")
    }

    pub fn guild_emojis_clone(id: Snowflake) -> String {
        format!("/guilds/{id}/emojis/clone")
    }

    pub fn guild_stickers_bulk(id: Snowflake) -> String {
        format!("/guilds/{id}/stickers/bulk")
    }

    pub fn guild_stickers_clone(id: Snowflake) -> String {
        format!("/guilds/{id}/stickers/clone")
    }

    pub fn guild_discovery(id: Snowflake) -> String {
        format!("/guilds/{id}/discovery")
    }

    pub fn guild_active_threads(id: Snowflake) -> String {
        format!("/guilds/{id}/threads/active")
    }

    pub fn guild_members_search(id: Snowflake) -> String {
        format!("/guilds/{id}/members-search")
    }

    pub fn guild_audit_logs(id: Snowflake) -> String {
        format!("/guilds/{id}/audit-logs")
    }

    pub fn guild_emojis(id: Snowflake) -> String {
        format!("/guilds/{id}/emojis")
    }

    pub fn guild_emoji(guild: Snowflake, emoji: Snowflake) -> String {
        format!("/guilds/{guild}/emojis/{emoji}")
    }

    pub fn guild_stickers(id: Snowflake) -> String {
        format!("/guilds/{id}/stickers")
    }

    pub fn guild_sticker(guild: Snowflake, sticker: Snowflake) -> String {
        format!("/guilds/{guild}/stickers/{sticker}")
    }

    pub fn guild_webhooks(id: Snowflake) -> String {
        format!("/guilds/{id}/webhooks")
    }

    pub fn webhook(id: Snowflake) -> String {
        format!("/webhooks/{id}")
    }

    pub fn leave_guild(id: Snowflake) -> String {
        format!("/users/@me/guilds/{id}")
    }

    pub fn guild_bulk_delete_mine(id: Snowflake) -> String {
        format!("/users/@me/guilds/{id}/messages/bulk-delete-mine")
    }

    pub fn user_me_channel_pin(id: Snowflake) -> String {
        format!("/users/@me/channels/{id}/pin")
    }

    pub fn emoji_metadata(id: Snowflake) -> String {
        format!("/emojis/{id}/metadata")
    }

    pub fn sticker_metadata(id: Snowflake) -> String {
        format!("/stickers/{id}/metadata")
    }

    pub fn oauth2_application_bot(id: Snowflake) -> String {
        format!("/oauth2/applications/{id}/bot")
    }

    pub fn oauth2_application_bot_reset_token(id: Snowflake) -> String {
        format!("/oauth2/applications/{id}/bot/reset-token")
    }

    pub fn channels_messages_bulk() -> &'static str {
        "/channels/messages/bulk"
    }

    pub fn guilds() -> &'static str {
        "/guilds"
    }

    pub fn current_user_guilds() -> &'static str {
        "/users/@me/guilds"
    }

    pub fn instance_discovery() -> &'static str {
        "/.well-known/fluxer"
    }

    pub fn applications_me() -> &'static str {
        "/applications/@me"
    }

    pub fn oauth2_applications_me() -> &'static str {
        "/oauth2/applications/@me"
    }

    pub fn check_username_tag() -> &'static str {
        "/users/check-tag"
    }

    pub fn preload_messages() -> &'static str {
        "/users/@me/preload-messages"
    }

    pub fn preload_messages_alt() -> &'static str {
        "/users/@me/channels/messages/preload"
    }

    pub fn search_messages() -> &'static str {
        "/search/messages"
    }

    pub fn webhook_execute(id: Snowflake, token: &str) -> String {
        format!("/webhooks/{id}/{}", urlenc(token))
    }

    pub fn webhook_message(id: Snowflake, token: &str, message: Snowflake) -> String {
        format!("/webhooks/{id}/{}/messages/{message}", urlenc(token))
    }

    /// `GET /users/{id}/profile`; pass `guild` for the server-specific profile.
    pub fn user_profile(id: Snowflake, guild: Option<Snowflake>) -> String {
        match guild {
            Some(g) => format!("/users/{id}/profile?guild_id={g}"),
            None => format!("/users/{id}/profile"),
        }
    }

    pub fn stream_preview(stream_key: &str) -> String {
        format!("/streams/{}/preview", urlenc(stream_key))
    }
}
