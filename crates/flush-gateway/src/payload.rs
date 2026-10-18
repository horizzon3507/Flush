use serde::{Deserialize, Serialize};
use thiserror::Error;

use flush_core::presence::Presence;

/// Opcodes do Gateway principal (registro completo 0-16, 28).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Opcode {
    Dispatch = 0,
    Heartbeat = 1,
    Identify = 2,
    PresenceUpdate = 3,
    VoiceStateUpdate = 4,
    Resume = 6,
    Reconnect = 7,
    RequestGuildMembers = 8,
    InvalidSession = 9,
    Hello = 10,
    HeartbeatAck = 11,
    GatewayError = 12,
    VoiceSignal = 13,
    LazyRequest = 14,
    RequestGuildCounts = 15,
    RequestChannelMemberCounts = 16,
    RequestForumUnreads = 28,
    Unknown = 255,
}

impl Opcode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Dispatch,
            1 => Self::Heartbeat,
            2 => Self::Identify,
            3 => Self::PresenceUpdate,
            4 => Self::VoiceStateUpdate,
            6 => Self::Resume,
            7 => Self::Reconnect,
            8 => Self::RequestGuildMembers,
            9 => Self::InvalidSession,
            10 => Self::Hello,
            11 => Self::HeartbeatAck,
            12 => Self::GatewayError,
            13 => Self::VoiceSignal,
            14 => Self::LazyRequest,
            15 => Self::RequestGuildCounts,
            16 => Self::RequestChannelMemberCounts,
            28 => Self::RequestForumUnreads,
            _ => Self::Unknown,
        }
    }
}

/// Payload genérico do Gateway: `{ op, d?, s?, t? }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayPayload {
    pub op: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t: Option<String>,
}

impl GatewayPayload {
    pub fn opcode(&self) -> Opcode {
        Opcode::from_u8(self.op)
    }

    pub fn command(op: Opcode, data: serde_json::Value) -> Self {
        Self {
            op: op as u8,
            d: Some(data),
            s: None,
            t: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloPayload {
    pub heartbeat_interval: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentifyProperties {
    pub os: String,
    pub browser: String,
    pub device: String,
}

impl IdentifyProperties {
    pub fn flush_bot() -> Self {
        Self {
            os: std::env::consts::OS.into(),
            browser: "Flush".into(),
            device: "flush-bot".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentifyPayload {
    pub token: String,
    pub properties: IdentifyProperties,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence: Option<Presence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignored_events: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shard: Option<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumePayload {
    pub token: String,
    pub session_id: String,
    pub seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceUpdate {
    #[serde(flatten)]
    pub presence: Presence,
}

/// Erro de Gateway (op 12, voz).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayError {
    pub code: String,
    #[serde(default)]
    pub message: String,
}

/// Close codes 4000-4012 do Gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CloseCode {
    UnknownError,
    UnknownOpcode,
    DecodeError,
    NotAuthenticated,
    AuthFailed,
    AlreadyAuthenticated,
    InvalidSequence,
    RateLimited,
    SessionTimeout,
    InvalidShard,
    ShardingRequired,
    InvalidApiVersion,
    Unknown(u16),
}

impl CloseCode {
    pub fn from_u16(v: u16) -> Self {
        match v {
            4000 => Self::UnknownError,
            4001 => Self::UnknownOpcode,
            4002 => Self::DecodeError,
            4003 => Self::NotAuthenticated,
            4004 => Self::AuthFailed,
            4005 => Self::AlreadyAuthenticated,
            4007 => Self::InvalidSequence,
            4008 => Self::RateLimited,
            4009 => Self::SessionTimeout,
            4010 => Self::InvalidShard,
            4011 => Self::ShardingRequired,
            4012 => Self::InvalidApiVersion,
            other => Self::Unknown(other),
        }
    }

    /// Sessão estabelecida ainda pode ser recuperada com Resume?
    pub fn resumable(&self) -> bool {
        match self {
            // 4003/4010/4011/4012 nunca criaram/recuperam sessão.
            Self::NotAuthenticated
            | Self::InvalidShard
            | Self::ShardingRequired
            | Self::InvalidApiVersion => false,
            // 4007: a tentativa falhou, mas a sessão segue retida até expirar.
            Self::InvalidSequence => true,
            _ => true,
        }
    }
}

/// Erros locais do payload (não confundir com close codes do servidor).
#[derive(Debug, Error)]
pub enum PayloadError {
    #[error("payload inválido: {0}")]
    Invalid(String),
}
