//! flush-gateway: Fluxer WebSocket Gateway v1.
//!
//! Contracts (official docs):
//! - Connect to `endpoints.gateway` (or `GET /v1/gateway/bot`) with `?v=1&encoding=json`.
//! - The server sends Hello (op 10) with `heartbeat_interval`; the client sends
//!   Identify (op 2) with the raw token and `properties`, then Heartbeat (op 1)
//!   with the last Dispatch `seq` every interval.
//! - Resume (op 6) with `token`/`session_id`/`seq`; the session is kept for 60s.
//! - Server opcodes: Dispatch (0), Reconnect (7), Invalid Session (9),
//!   Hello (10), Heartbeat ACK (11). Close codes 4000-4012.

pub mod client;
pub mod payload;
pub mod shard;
pub use client::{gateway_url_with_params, ClientConfig, ConnectError, GatewayClient};
pub use payload::{
    CloseCode, GatewayError, GatewayPayload, HelloPayload, IdentifyPayload, IdentifyProperties,
    Opcode, PresenceUpdate, ResumePayload,
};
pub use shard::{
    recommended_shard_count, shard_id_for_guild, ShardError, ShardEvent, ShardInfo, ShardManager,
    ShardManagerOptions, ShardPlan, ShardStatus, DEFAULT_GUILDS_PER_SHARD, MAX_SHARD_COUNT,
};
