use thiserror::Error;

/// Core Flush errors (local parsing; HTTP/WS have their own types).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CoreError {
    #[error("INVALID_SNOWFLAKE_FORMAT: {0}")]
    InvalidSnowflake(String),
    #[error("SNOWFLAKE_OUT_OF_RANGE: {0}")]
    SnowflakeOutOfRange(String),
    #[error("embed inválido: {0}")]
    InvalidEmbed(String),
    #[error("mensagem inválida: {0}")]
    InvalidMessage(String),
}
