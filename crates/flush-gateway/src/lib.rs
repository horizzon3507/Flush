//! flush-gateway: Gateway WebSocket v1 do Fluxer.
//!
//! Contratos (docs oficiais):
//! - Conectar em `endpoints.gateway` (ou `GET /v1/gateway/bot`) com `?v=1&encoding=json`.
//! - Servidor manda Hello (op 10) com `heartbeat_interval`; cliente manda
//!   Identify (op 2) com token raw e `properties`, depois Heartbeat (op 1)
//!   com o último `seq` de Dispatch a cada intervalo.
//! - Resume (op 6) com `token`/`session_id`/`seq`; sessão retida por 60s.
//! - Opcodes de servidor: Dispatch (0), Reconnect (7), Invalid Session (9),
//!   Hello (10), Heartbeat ACK (11). Close codes 4000-4012.

pub mod client;
pub mod payload;
pub use client::{gateway_url_with_params, ClientConfig, ConnectError, GatewayClient};
pub use payload::{
    CloseCode, GatewayError, GatewayPayload, HelloPayload, IdentifyPayload, IdentifyProperties,
    Opcode, PresenceUpdate, ResumePayload,
};
