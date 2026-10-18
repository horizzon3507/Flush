use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Corpo de erro padrão da API (`code` + `message` + `errors?`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiErrorBody {
    pub code: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Error)]
pub enum RestError {
    #[error("HTTP {status}: {code} {message}")]
    Api {
        status: u16,
        code: String,
        message: String,
    },
    #[error("rate limited, retry em {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },
    #[error("rede/http: {0}")]
    Transport(String),
    #[error("resposta inválida: {0}")]
    BadResponse(String),
    #[error("config: {0}")]
    Config(String),
}

impl RestError {
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Api { status: 404, .. })
    }

    pub fn is_forbidden(&self) -> bool {
        matches!(self, Self::Api { status: 403, .. })
    }
}
