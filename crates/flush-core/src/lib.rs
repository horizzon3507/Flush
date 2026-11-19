//! flush-core: tipos centrais do Flush.
//!
//! Paridade inicial com `@fluxerjs/types`: Snowflake, permissões,
//! usuários, guilds, canais, mensagens, embeds e eventos de Gateway.
//! Todo ID de recurso viaja como string decimal no wire.

pub mod application;
pub mod audit_log;
pub mod cdn;
pub mod channel;
pub mod embed;
pub mod emoji;
pub mod error;
pub mod events;
pub mod guild;
pub mod invite;
pub mod message;
pub mod permissions;
pub mod presence;
pub mod snowflake;
pub mod thread;
pub mod user;
pub mod voice;
pub mod webhook;
pub mod wire;

pub use application::Application;
pub use audit_log::{
    AuditLog, AuditLogAction, AuditLogChange, AuditLogEntry, AuditLogUser, AuditLogWebhook,
};
pub use cdn::{
    avatar_url, banner_url, default_avatar_url, display_avatar_url, emoji_url, guild_asset_url,
    member_avatar_url, member_banner_url, snap_size, sticker_url, AssetClass, GuildAsset, CDN_URL,
    STATIC_CDN_URL,
};
pub use channel::{Channel, ChannelType, OverwriteType, PermissionOverwrite};
pub use embed::{Embed, EmbedAuthor, EmbedBuilder, EmbedField, EmbedFooter, EmbedMedia};
pub use emoji::{BulkCreateResult, BulkFailure, Emoji, ExpressionMetadata, Sticker};
pub use error::CoreError;
pub use events::{is_known_dispatch, DISPATCH_EVENTS};
pub use events::{DispatchEnvelope, Event, GuildReady, PresencePayload, ReadyPayload};
pub use guild::{Guild, GuildFlat, GuildMember, GuildSnapshot, MemberUser, Role};
pub use invite::Invite;
pub use message::{
    AllowedMentions, Message, MessageAttachment, MessageReference, MessageSend, MessageType,
    Reaction, ReactionEmoji,
};
pub use permissions::Permissions;
pub use presence::{Activity, Presence, PresenceStatus};
pub use snowflake::{Snowflake, FLUXER_EPOCH};
pub use thread::{
    DefaultReactionEmoji, ForumTag, StartThread, ThreadMember, ThreadMetadata, ThreadsListing,
};
pub use user::{PartialUser, User};
pub use voice::{VoiceServerUpdate, VoiceState};
pub use webhook::{Webhook, WebhookExecute, WebhookType};
