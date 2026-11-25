//! Music and Lavalink errors carrying a stable code, mirroring `YuzuhaError`.

use thiserror::Error;

/// A music/Lavalink failure. `code` is the stable machine-readable code
/// (`LAVALINK_*` / `MUSIC_*`); `message` is human-readable detail.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{code}: {message}")]
pub struct MusicError {
    pub code: &'static str,
    pub message: String,
}

impl MusicError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn cancelled() -> Self {
        Self::new("CANCELLED", "The operation was cancelled.")
    }
}

