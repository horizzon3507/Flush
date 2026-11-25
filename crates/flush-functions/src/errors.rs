//! Error types mirroring yuzuha `src/shared/errors.ts` (in English).

use std::fmt;

/// A framework error carrying a stable machine code (mirrors `YuzuhaError`).
#[derive(Debug)]
pub struct FnError {
    pub code: &'static str,
    pub message: String,
}

impl FnError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for FnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FnError {}

/// A script error with source position (mirrors `ScriptError`).
#[derive(Debug)]
pub struct ScriptError {
    pub code: &'static str,
    pub message: String,
    pub source: String,
    pub offset: usize,
    pub file: String,
}

impl ScriptError {
    pub fn new(
        code: &'static str,
        message: impl Into<String>,
        source: &str,
        offset: usize,
        file: &str,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            source: source.to_string(),
            offset,
            file: file.to_string(),
        }
    }

    /// 1-based line of `offset` inside `source` (offsets are char indices).
    pub fn line(&self) -> usize {
        self.source
            .chars()
            .take(self.offset)
            .filter(|c| *c == '\n')
            .count()
            + 1
    }

    /// 1-based column of `offset` inside `source`.
    pub fn column(&self) -> usize {
        if self.offset == 0 {
            return 1;
        }
        let prefix: Vec<char> = self.source.chars().take(self.offset).collect();
        let last_newline = prefix.iter().rposition(|c| *c == '\n');
        match last_newline {
            Some(index) => self.offset - index,
            None => self.offset + 1,
        }
    }

    /// `file:line:col [code] message` plus the offending line and a caret.
    pub fn format(&self) -> String {
        let line = self
            .source
            .split('\n')
            .nth(self.line() - 1)
            .unwrap_or_default();
        format!(
            "{}:{}:{} [{}] {}\n{}\n{}^",
            self.file,
            self.line(),
            self.column(),
            self.code,
            self.message,
            line,
            " ".repeat(self.column() - 1)
        )
    }
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format())
    }
}

impl std::error::Error for ScriptError {}

/// Guard reasons (mirrors `GuardReason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardReason {
    Generic,
    NotInGuild,
    NoAuthor,
    PermissionMissingAuthor,
    PermissionMissingBot,
    Hierarchy,
    AdminImmune,
    SelfTarget,
    BotTarget,
    OwnerTarget,
    MemberNotFound,
    NotBanned,
    InvalidArgument,
    InvalidReason,
    InvalidDuration,
    Cooldown,
    CheckFailed,
    OwnerOnly,
    DevOnly,
}

/// A user-facing guard failure (mirrors `GuardError`).
#[derive(Debug)]
pub struct GuardError {
    pub message: String,
    pub reason: GuardReason,
    pub remaining_ms: Option<u64>,
}

impl GuardError {
    pub fn new(message: impl Into<String>, reason: GuardReason) -> Self {
        Self {
            message: message.into(),
            reason,
            remaining_ms: None,
        }
    }

    pub fn generic(message: impl Into<String>) -> Self {
        Self::new(message, GuardReason::Generic)
    }

    pub fn cooldown(message: impl Into<String>, remaining_ms: u64) -> Self {
        Self {
            message: message.into(),
            reason: GuardReason::Cooldown,
            remaining_ms: Some(remaining_ms),
        }
    }
}

/// `$stop` / `StopExecution`: end the script silently.
#[derive(Debug)]
pub struct StopExecution;

/// `break`/`continue` inside a loop body (mirrors `LoopControl`).
#[derive(Debug)]
pub struct LoopControl {
    pub frame: u64,
    pub action: LoopAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopAction {
    Break,
    Continue,
}

/// The moderation action ran but recording the case failed
/// (mirrors `PartialModerationError`).
#[derive(Debug)]
pub struct PartialModerationError {
    pub action: String,
    pub target_id: String,
    pub message: String,
    pub has_custom_message: bool,
}

/// Every error the evaluator can produce or propagate.
#[derive(Debug)]
pub enum ScriptFailure {
    Fn(FnError),
    Script(ScriptError),
    Guard(GuardError),
    Stop(StopExecution),
    Loop(LoopControl),
    PartialModeration(PartialModerationError),
}

impl ScriptFailure {
    /// Machine code for logs/replies.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Fn(e) => e.code,
            Self::Script(e) => e.code,
            Self::Guard(_) => "GUARD",
            Self::Stop(_) => "STOP",
            Self::Loop(_) => "LOOP_CONTROL",
            Self::PartialModeration(_) => "MODERATION_PARTIAL",
        }
    }

    /// Human-readable message.
    pub fn message(&self) -> &str {
        match self {
            Self::Fn(e) => &e.message,
            Self::Script(e) => &e.message,
            Self::Guard(e) => &e.message,
            Self::Stop(_) => "",
            Self::Loop(e) => match e.action {
                LoopAction::Break => "break",
                LoopAction::Continue => "continue",
            },
            Self::PartialModeration(e) => &e.message,
        }
    }
}

impl From<FnError> for ScriptFailure {
    fn from(e: FnError) -> Self {
        Self::Fn(e)
    }
}

impl From<ScriptError> for ScriptFailure {
    fn from(e: ScriptError) -> Self {
        Self::Script(e)
    }
}

impl From<GuardError> for ScriptFailure {
    fn from(e: GuardError) -> Self {
        Self::Guard(e)
    }
}

impl From<StopExecution> for ScriptFailure {
    fn from(e: StopExecution) -> Self {
        Self::Stop(e)
    }
}

impl From<LoopControl> for ScriptFailure {
    fn from(e: LoopControl) -> Self {
        Self::Loop(e)
    }
}

impl From<PartialModerationError> for ScriptFailure {
    fn from(e: PartialModerationError) -> Self {
        Self::PartialModeration(e)
    }
}

impl From<flush_rest::RestError> for ScriptFailure {
    fn from(e: flush_rest::RestError) -> Self {
        Self::Fn(FnError::new("API_ERROR", format!("{e}")))
    }
}

impl fmt::Display for ScriptFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Script(e) => write!(f, "{e}"),
            other => write!(f, "{}", other.message()),
        }
    }
}

impl std::error::Error for ScriptFailure {}

pub type FnResult<T> = Result<T, ScriptFailure>;

/// Stable machine codes used across the evaluator and builtins.
pub mod codes {
    pub const INVALID_ARGUMENTS: &str = "INVALID_ARGUMENTS";
    pub const ARITY: &str = "ARITY";
    pub const UNKNOWN_FUNCTION: &str = "UNKNOWN_FUNCTION";
    pub const INVALID_FUNCTION: &str = "INVALID_FUNCTION";
    pub const FUNCTION_ERROR: &str = "FUNCTION_ERROR";
    pub const SOURCE_LIMIT: &str = "SOURCE_LIMIT";
    pub const EXECUTION_LIMIT: &str = "EXECUTION_LIMIT";
    pub const OUTPUT_LIMIT: &str = "OUTPUT_LIMIT";
    pub const EXECUTION_ABORTED: &str = "EXECUTION_ABORTED";
    pub const NATIVE_UNAVAILABLE: &str = "NATIVE_UNAVAILABLE";
    pub const API_ERROR: &str = "API_ERROR";
    pub const GUARD: &str = "GUARD";
    pub const STOP: &str = "STOP";
    pub const LOOP_CONTROL: &str = "LOOP_CONTROL";
    pub const MODERATION_PARTIAL: &str = "MODERATION_PARTIAL";
    /// yuzuha script-level codes used by the services builtins.
    pub const INDEX: &str = "INDEX";
    pub const NUMBER: &str = "NUMBER";
    pub const JSON: &str = "JSON";
    pub const DURATION: &str = "DURATION";
    pub const USER_ID: &str = "USER_ID";
    pub const CHANNEL_ID: &str = "CHANNEL_ID";
    pub const VARIABLE_QUERY: &str = "VARIABLE_QUERY";
    pub const VARIABLE: &str = "VARIABLE";
    pub const DATA_TRANSACTION: &str = "DATA_TRANSACTION";
    pub const SCHEDULE: &str = "SCHEDULE";
    pub const HTTP: &str = "HTTP";
    pub const HTTP_HEADER: &str = "HTTP_HEADER";
    pub const HTTP_METHOD: &str = "HTTP_METHOD";
    pub const HTTP_URL: &str = "HTTP_URL";
    pub const HTTP_BODY: &str = "HTTP_BODY";
    pub const HTTP_JSON: &str = "HTTP_JSON";
    pub const HTTP_MULTIPART: &str = "HTTP_MULTIPART";
    pub const VOICE_STATE: &str = "VOICE_STATE";
    pub const USER_ERROR: &str = "USER_ERROR";
    pub const SDK: &str = "SDK";
    pub const COLLECTION: &str = "COLLECTION";
    pub const CDN: &str = "CDN";
    pub const INSTANCE: &str = "INSTANCE";
}

/// Errors that must never be swallowed by `$try` or rethrown as
/// `FUNCTION_ERROR` (guards, limits, partial moderation, loop control).
pub(crate) fn is_uncatchable(error: &ScriptFailure) -> bool {
    match error {
        ScriptFailure::Guard(_)
        | ScriptFailure::Stop(_)
        | ScriptFailure::PartialModeration(_)
        | ScriptFailure::Loop(_) => true,
        ScriptFailure::Script(e) => is_limit_code(e.code),
        ScriptFailure::Fn(e) => is_limit_code(e.code),
    }
}

fn is_limit_code(code: &str) -> bool {
    matches!(
        code,
        codes::EXECUTION_LIMIT | codes::OUTPUT_LIMIT | codes::EXECUTION_ABORTED
    )
}
