//! Rotas `/v1` usadas pelo Flush.
//!
//! Espelho de `Routes` do `@fluxerjs/types` (subset de bots):
//! canais, mensagens, guilds, membros, roles, bans, users, gateway.

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

    /// `emoji` já URL-encodado (`ReactionEmoji::encoded()`).
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
}
