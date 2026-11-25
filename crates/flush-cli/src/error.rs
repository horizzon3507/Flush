use flush_framework::ConfigError;

/// CLI failure with a stable machine-readable code (`USAGE`, `UNSUPPORTED`,
/// `CONFIG_MISSING`, `CONFIG_INVALID`, `INIT_NAME`, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    pub code: String,
    pub message: String,
}

impl CliError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for CliError {}

impl From<ConfigError> for CliError {
    fn from(error: ConfigError) -> Self {
        Self::new(error.code, error.message)
    }
}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        Self::new("IO", error.to_string())
    }
}

impl From<serde_json::Error> for CliError {
    fn from(error: serde_json::Error) -> Self {
        Self::new("JSON", error.to_string())
    }
}
