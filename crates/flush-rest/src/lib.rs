//! flush-rest: cliente REST `/v1` do Fluxer sobre surf.
//!
//! Contratos (docs oficiais + uso real do Sour/Jully):
//! - Base via `GET /.well-known/fluxer` -> `endpoints.api_public`
//!   (fallback `https://api.fluxer.app`); rotas sempre sob `/v1`.
//! - Auth de bot: `Authorization: Bot <token>`.
//! - `429 RATE_LIMITED`: respeita `Retry-After` e repete uma vez.
//! - Rotas cobertas: mensagens (CRUD, bulk-delete 2-100, typing),
//!   membros (get/list/patch/kick/roles), bans (put/delete/list),
//!   roles (list), users (get), guild (get), gateway bot info.

pub mod client;
pub mod error;
pub mod routes;

pub use client::{
    BanOptions, ChannelPatch, EditMessageBody, GatewayBotInfo, GuildBan, InstanceEndpoints,
    MemberPatch, RestClient, RestConfig,
};
pub use error::{ApiErrorBody, RestError};
pub use routes::Routes;
