//! flush-rest: Fluxer `/v1` REST client on surf.
//!
//! Contracts (official docs + real Sour/Jully usage):
//! - Base URL via `GET /.well-known/fluxer` -> `endpoints.api_public`
//!   (fallback `https://api.fluxer.app`); routes always under `/v1`.
//! - Bot auth: `Authorization: Bot <token>`.
//! - `429 RATE_LIMITED`: honors `Retry-After` and retries once.
//! - Covered routes: messages (CRUD, bulk-delete 2-100, typing),
//!   members (get/list/patch/kick/roles), bans (put/delete/list),
//!   roles (list), users (get), guild (get), gateway bot info.

pub mod account;
pub mod client;
pub mod error;
pub mod resources;
pub mod routes;
pub mod uploads;

pub use account::{
    BulkMessageRequest, ChannelMessages, DiscoveryApplication, DiscoveryRequest, DiscoveryStatus,
    GuildCreate, PreloadedMessages, RoleHoistPosition, SudoVerification, ThreadMemberSettings,
    ThreadMuteConfig,
};
pub use client::{
    BanOptions, ChannelPatch, EditMessageBody, GatewayBotInfo, GuildBan, InstanceEndpoints,
    MemberPatch, RestClient, RestConfig,
};
pub use error::{ApiErrorBody, RestError};
pub use resources::{
    AuditLogQuery, ChannelPosition, GuildPatch, MemberSearch, RolePosition, StickerCreate,
    StickerPatch, ThreadSearch, UserGuild, VanityUrl, WebhookPatch,
};
pub use routes::Routes;
pub use uploads::{
    multipart_body, split_parts, AttachmentUploadComplete, AttachmentUploadPart,
    AttachmentUploadPlan, AttachmentUploadRequest, FileUpload,
};
