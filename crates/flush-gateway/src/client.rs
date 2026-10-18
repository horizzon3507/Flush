//! Cliente do Gateway: conexão, Hello/Identify/Resume/Heartbeat e loop de Dispatch.
//!
//! Espelha o fluxo do `jully-fluxer/src/core/Bot.js` e do `Client` do
//! `@fluxerjs/core`, mas sobre `async-std` + `async-tungstenite`.

use std::time::{Duration, Instant};

#[allow(deprecated)]
use async_tungstenite::async_std::{connect_async, ConnectStream};
#[allow(deprecated)]
use async_tungstenite::tungstenite::Message as WsMessage;
use futures::prelude::*;
use log::{debug, info, warn};
use thiserror::Error;

use flush_core::events::{DispatchEnvelope, Event};
use flush_core::presence::Presence;

use crate::payload::{
    CloseCode, GatewayPayload, HelloPayload, IdentifyPayload, IdentifyProperties, Opcode,
    ResumePayload,
};
#[derive(Debug, Error)]
pub enum ConnectError {
    #[error("token vazio")]
    EmptyToken,
    #[error("URL do gateway inválida: {0}")]
    BadUrl(String),
    #[error("websocket: {0}")]
    Ws(String),
    #[error("hello inválido: {0}")]
    BadHello(String),
    #[error("token inválido (4004)")]
    InvalidToken,
    #[error("sharding necessário (4011): o bot resolve >2500 guilds neste shard")]
    ShardingRequired,
    #[error("shard inválido (4010)")]
    InvalidShard,
    #[error("versão de API inválida (4012)")]
    BadVersion,
    #[error("serialização: {0}")]
    Serde(String),
}

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub token: String,
    /// URL base do gateway (sem query ou com query parcial).
    /// Ex.: `wss://gateway.fluxer.app` — o cliente completa `?v=1&encoding=json`.
    pub gateway_url: String,
    pub properties: IdentifyProperties,
    pub presence: Option<Presence>,
    pub ignored_events: Vec<String>,
    pub shard: Option<(u32, u32)>,
    /// Backoff máximo entre reconexões (padrão 60s).
    pub max_backoff: Duration,
}

impl ClientConfig {
    pub fn new(token: impl Into<String>, gateway_url: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            gateway_url: gateway_url.into(),
            properties: IdentifyProperties::flush_bot(),
            presence: None,
            ignored_events: Vec::new(),
            shard: None,
            max_backoff: Duration::from_secs(60),
        }
    }

    fn validate(&self) -> Result<(), ConnectError> {
        if self.token.trim().is_empty() {
            return Err(ConnectError::EmptyToken);
        }
        if self.gateway_url.trim().is_empty() {
            return Err(ConnectError::BadUrl("vazia".into()));
        }
        Ok(())
    }
}

pub fn gateway_url_with_params(base: &str) -> String {
    // ⚠️ Barra obrigatória antes do query: o tungstenite ≥ 0.30 serializa
    // `path_and_query` cru — sem path, o request sai como
    // `GET ?v=1&encoding=json` e o gateway responde HTTP 400 Bad Request.
    let base = base.trim();
    let mut url = match base.split_once('?') {
        Some((host, query)) => {
            let host = host.trim_end_matches('/');
            let query = query.trim_end_matches('/');
            if query.is_empty() {
                format!("{host}/")
            } else {
                format!("{host}/?{query}")
            }
        }
        None => format!("{}/", base.trim_end_matches('/')),
    };
    let has_v = url.contains("v=1&") || url.ends_with("v=1");
    let has_enc = url.contains("encoding=json&") || url.ends_with("encoding=json");
    if !url.contains('?') {
        url.push_str("?v=1&encoding=json");
    } else {
        if !has_v {
            url.push_str("&v=1");
        }
        if !has_enc {
            url.push_str("&encoding=json");
        }
    }
    url
}

#[cfg(test)]
mod tests_url {
    use super::*;

    #[test]
    fn adiciona_barra_e_params() {
        assert_eq!(
            gateway_url_with_params("wss://gateway.fluxer.app"),
            "wss://gateway.fluxer.app/?v=1&encoding=json"
        );
        assert_eq!(
            gateway_url_with_params("wss://gateway.fluxer.app/"),
            "wss://gateway.fluxer.app/?v=1&encoding=json"
        );
        assert_eq!(
            gateway_url_with_params("wss://gw.example.com/?v=1&encoding=json"),
            "wss://gw.example.com/?v=1&encoding=json"
        );
        assert_eq!(
            gateway_url_with_params("wss://gw.example.com/?v=1"),
            "wss://gw.example.com/?v=1&encoding=json"
        );
    }
}


#[allow(deprecated)]
type WsStream = async_tungstenite::WebSocketStream<ConnectStream>;

/// Cliente stateful: mantém `seq`/`session_id` para Resume após quedas.
pub struct GatewayClient {
    config: ClientConfig,
    ws: Option<WsStream>,
    seq: Option<u64>,
    session_id: Option<String>,
    heartbeat_interval: Duration,
    backoff_attempts: u32,
    ready: bool,
}

impl GatewayClient {
    pub fn new(config: ClientConfig) -> Result<Self, ConnectError> {
        config.validate()?;
        Ok(Self {
            config,
            ws: None,
            seq: None,
            session_id: None,
            heartbeat_interval: Duration::from_millis(41_250),
            backoff_attempts: 0,
            ready: false,
        })
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn seq(&self) -> Option<u64> {
        self.seq
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// Próximo evento tipado. Retorna `Ok(None)` em heartbeats ACKs e
    /// janelas sem Dispatch (o chamador continua o loop).
    /// Reconexão com Resume é interna; erro fatal só em token/shard/versão.
    pub async fn next_event(&mut self) -> Result<Option<Event>, ConnectError> {
        loop {
            if self.ws.is_none() {
                self.connect_once().await?;
            }
            match self.step().await? {
                Step::Event(ev) => return Ok(Some(*ev)),
                Step::NoEvent => return Ok(None),
                Step::Reconnect => {
                    self.ws = None;
                    self.backoff().await;
                    continue;
                }
            }
        }
    }

    /// Envia Presence Update (op 3).
    pub async fn set_presence(&mut self, presence: &Presence) -> Result<(), ConnectError> {
        let body =
            serde_json::to_value(presence).map_err(|e| ConnectError::Serde(e.to_string()))?;
        self.send_command(Opcode::PresenceUpdate, body).await
    }

    /// Pede chunks de membros (op 8, uma guild por vez para bots).
    pub async fn request_guild_members(
        &mut self,
        guild_id: flush_core::Snowflake,
        query: &str,
        limit: u32,
    ) -> Result<(), ConnectError> {
        let body = serde_json::json!({
            "guild_id": guild_id.to_string(),
            "query": query,
            "limit": limit.min(100),
        });
        self.send_command(Opcode::RequestGuildMembers, body).await
    }

    async fn send_command(
        &mut self,
        op: Opcode,
        data: serde_json::Value,
    ) -> Result<(), ConnectError> {
        let Some(ws) = self.ws.as_mut() else {
            return Ok(());
        };
        let payload = GatewayPayload::command(op, data);
        let text =
            serde_json::to_string(&payload).map_err(|e| ConnectError::Serde(e.to_string()))?;
        ws.send(WsMessage::Text(text.into()))
            .await
            .map_err(|e| ConnectError::Ws(e.to_string()))?;
        Ok(())
    }

    async fn send_heartbeat(&mut self) -> Result<(), ConnectError> {
        let d = match self.seq {
            Some(s) => serde_json::Value::from(s),
            None => serde_json::Value::Null,
        };
        debug!("gateway: heartbeat seq={:?}", self.seq);
        let payload = GatewayPayload {
            op: Opcode::Heartbeat as u8,
            d: Some(d),
            s: None,
            t: None,
        };
        let text =
            serde_json::to_string(&payload).map_err(|e| ConnectError::Serde(e.to_string()))?;
        let Some(ws) = self.ws.as_mut() else {
            return Ok(());
        };
        ws.send(WsMessage::Text(text.into()))
            .await
            .map_err(|e| ConnectError::Ws(e.to_string()))?;
        Ok(())
    }

    async fn connect_once(&mut self) -> Result<(), ConnectError> {
        let url = gateway_url_with_params(&self.config.gateway_url);
        debug!("gateway: conectando {url}");
        #[allow(deprecated)]
        let (ws, _) = connect_async(&url)
            .await
            .map_err(|e| ConnectError::Ws(e.to_string()))?;
        self.ws = Some(ws);
        // Hello é o primeiro frame (op 10).
        let hello = self.read_payload().await?;
        if hello.opcode() != Opcode::Hello {
            return Err(ConnectError::BadHello(format!(
                "op={} esperado=10",
                hello.op
            )));
        }
        let hello_body: HelloPayload = hello
            .d
            .clone()
            .and_then(|v| serde_json::from_value(v).ok())
            .ok_or_else(|| ConnectError::BadHello("sem heartbeat_interval".into()))?;
        self.heartbeat_interval = Duration::from_millis(hello_body.heartbeat_interval.max(5_000));
        info!(
            "gateway: hello, heartbeat a cada {}ms",
            hello_body.heartbeat_interval
        );

        if let Some(session_id) = self.session_id.clone() {
            let resume = ResumePayload {
                token: self.config.token.clone(),
                session_id,
                seq: self.seq.unwrap_or(0),
            };
            let body =
                serde_json::to_value(&resume).map_err(|e| ConnectError::Serde(e.to_string()))?;
            self.send_command(Opcode::Resume, body).await?;
            debug!("gateway: resume enviado");
        } else {
            let identify = IdentifyPayload {
                token: self.config.token.clone(),
                properties: self.config.properties.clone(),
                presence: self.config.presence.clone(),
                ignored_events: self.config.ignored_events.clone(),
                shard: self.config.shard,
            };
            let body =
                serde_json::to_value(&identify).map_err(|e| ConnectError::Serde(e.to_string()))?;
            self.send_command(Opcode::Identify, body).await?;
            debug!("gateway: identify enviado");
        }
        self.backoff_attempts = 0;
        Ok(())
    }

    async fn read_payload(&mut self) -> Result<GatewayPayload, ConnectError> {
        let ws = self.ws.as_mut().expect("ws conectado");
        loop {
            let msg = ws
                .next()
                .await
                .ok_or_else(|| ConnectError::Ws("stream fechado".into()))?;
            let msg = msg.map_err(|e| ConnectError::Ws(e.to_string()))?;
            match msg {
                WsMessage::Text(text) => {
                    let payload: GatewayPayload = serde_json::from_str(&text)
                        .map_err(|e| ConnectError::Ws(format!("decode: {e}")))?;
                    return Ok(payload);
                }
                WsMessage::Ping(data) => {
                    ws.send(WsMessage::Pong(data))
                        .await
                        .map_err(|e| ConnectError::Ws(e.to_string()))?;
                }
                WsMessage::Close(frame) => {
                    return Err(close_to_error(frame.map(|f| f.code.into())));
                }
                _ => {}
            }
        }
    }

    async fn step(&mut self) -> Result<Step, ConnectError> {
        let interval = self.heartbeat_interval;
        let deadline = Instant::now() + interval;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.send_heartbeat().await?;
                return Ok(Step::NoEvent);
            }
            let payload = match async_std::future::timeout(remaining, self.read_payload()).await {
                Ok(Ok(p)) => p,
                Ok(Err(ConnectError::Ws(msg))) if msg.starts_with("stream fechado") => {
                    warn!("gateway: transporte fechado, retomando em 60s de retenção");
                    return Ok(Step::Reconnect);
                }
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    self.send_heartbeat().await?;
                    return Ok(Step::NoEvent);
                }
            };
            match payload.opcode() {
                Opcode::Dispatch => {
                    let kind = payload.t.clone().unwrap_or_default();
                    let seq = payload
                        .s
                        .unwrap_or_else(|| self.seq.map(|s| s + 1).unwrap_or(1));
                    self.seq = Some(seq);
                    let env = DispatchEnvelope {
                        kind: kind.clone(),
                        seq,
                        data: payload.d.clone().unwrap_or(serde_json::Value::Null),
                    };
                    let ev = Event::decode(&env);
                    if matches!(ev, Event::Ready(_)) {
                        self.ready = true;
                        if let Event::Ready(r) = &ev {
                            self.session_id = Some(r.session_id.clone());
                            info!("gateway: ready session={}", r.session_id);
                        }
                    }
                    if matches!(ev, Event::Resumed { .. }) {
                        info!("gateway: resumed seq={seq}");
                    }
                    return Ok(Step::Event(Box::new(ev)));
                }
                Opcode::Heartbeat => {
                    // Servidor pedindo heartbeat imediato (d=null ~90% do intervalo).
                    debug!("gateway: heartbeat solicitado pelo servidor");
                    self.send_heartbeat().await?;
                }
                Opcode::HeartbeatAck => {
                    debug!("gateway: heartbeat ack");
                    return Ok(Step::NoEvent);
                }
                Opcode::Reconnect => {
                    warn!("gateway: reconnect solicitado (op 7)");
                    return Ok(Step::Reconnect);
                }
                Opcode::InvalidSession => {
                    warn!("gateway: sessão inválida (op 9), re-identificando");
                    self.session_id = None;
                    self.ready = false;
                    return Ok(Step::Reconnect);
                }
                Opcode::Hello => {
                    if let Some(d) = payload.d {
                        if let Ok(h) = serde_json::from_value::<HelloPayload>(d) {
                            self.heartbeat_interval =
                                Duration::from_millis(h.heartbeat_interval.max(5_000));
                        }
                    }
                }
                Opcode::GatewayError => {
                    warn!("gateway: erro op12: {:?}", payload.d);
                    return Ok(Step::NoEvent);
                }
                _ => {
                    debug!("gateway: opcode {} ignorado", payload.op);
                    return Ok(Step::NoEvent);
                }
            }
        }
    }

    async fn backoff(&mut self) {
        self.backoff_attempts += 1;
        let exp = 2u64.saturating_pow(self.backoff_attempts.min(6));
        let delay = Duration::from_secs(1)
            .saturating_mul(exp as u32)
            .min(self.config.max_backoff);
        warn!(
            "gateway: reconectando em {delay:?} (tentativa {})",
            self.backoff_attempts
        );
        async_std::task::sleep(delay).await;
    }
}

enum Step {
    Event(Box<Event>),
    NoEvent,
    Reconnect,
}

fn close_to_error(code: Option<u16>) -> ConnectError {
    match code.map(CloseCode::from_u16) {
        Some(CloseCode::AuthFailed) => ConnectError::InvalidToken,
        Some(CloseCode::ShardingRequired) => ConnectError::ShardingRequired,
        Some(CloseCode::InvalidShard) => ConnectError::InvalidShard,
        Some(CloseCode::InvalidApiVersion) => ConnectError::BadVersion,
        Some(c) => ConnectError::Ws(format!("close {c:?}")),
        None => ConnectError::Ws("close sem código".into()),
    }
}
