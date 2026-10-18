//! flush-core: tipos centrais do Flush.
//!
//! Paridade inicial com `@fluxerjs/types`: Snowflake, permissões,
//! usuários, guilds, canais, mensagens, embeds e eventos de Gateway.
//! Todo ID de recurso viaja como string decimal no wire.

pub mod cdn;
pub mod channel;
pub mod embed;
pub mod error;
pub mod events;
pub mod guild;
pub mod invite;
pub mod message;
pub mod permissions;
pub mod presence;
pub mod snowflake;
pub mod user;
pub mod wire;

pub use cdn::{avatar_url, default_avatar_url, display_avatar_url, CDN_URL, STATIC_CDN_URL};
pub use channel::{Channel, ChannelType, OverwriteType, PermissionOverwrite};
pub use embed::{Embed, EmbedAuthor, EmbedBuilder, EmbedField, EmbedFooter, EmbedMedia};
pub use error::CoreError;
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
pub use user::{PartialUser, User};
