//! Error type with the same code vocabulary as `YuzuhaError`.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CanvasError {
    /// `CANVAS_OPTIONS` — invalid caller-supplied options.
    #[error("{0}")]
    Options(String),
    /// `CANVAS_INPUT` — invalid input data (image/font/source).
    #[error("{0}")]
    Input(String),
    /// `CANVAS_ASSET` — an `asset:` or remote source could not be resolved.
    #[error("{0}")]
    Asset(String),
    /// `CANVAS_STATE` — wrong object for this service (e.g. no transport).
    #[error("{0}")]
    State(String),
    /// `CANVAS_PROCESS` — the renderer/encoder failed.
    #[error("{0}")]
    Process(String),
    /// `EXECUTION_LIMIT` — size, count, bytes or timeout budget exceeded.
    #[error("{0}")]
    ExecutionLimit(String),
    /// `EXECUTION_ABORTED` — cancelled before finishing.
    #[error("{0}")]
    Aborted(String),
    /// `REST` — transport failure while sending the image.
    #[error("rest: {0}")]
    Rest(#[from] flush_rest::RestError),
}

impl CanvasError {
    /// The yuzuha error code this variant maps to.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Options(_) => "CANVAS_OPTIONS",
            Self::Input(_) => "CANVAS_INPUT",
            Self::Asset(_) => "CANVAS_ASSET",
            Self::State(_) => "CANVAS_STATE",
            Self::Process(_) => "CANVAS_PROCESS",
            Self::ExecutionLimit(_) => "EXECUTION_LIMIT",
            Self::Aborted(_) => "EXECUTION_ABORTED",
            Self::Rest(_) => "REST",
        }
    }
}
