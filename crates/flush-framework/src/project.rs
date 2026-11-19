//! Project config shared by bots and the `flush` CLI (mirrors Yuzuha's
//! `yuzuha.config.*` discovery and `.env` loading).
//!
//! A project is a directory with a `flush.toml` and an optional `.env` next
//! to it. Values already present in the process environment win over `.env`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::router::{Bot, BotConfig};

/// File name looked up when no explicit config path is given.
pub const CONFIG_FILE: &str = "flush.toml";

/// Config failure with a stable machine-readable code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ConfigError {
    /// `CONFIG_MISSING`, `CONFIG_INVALID` or `TOKEN_MISSING`.
    pub code: &'static str,
    pub message: String,
}

impl ConfigError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

fn default_prefix() -> String {
    "!".into()
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("data")
}

/// Contents of `flush.toml`. Unknown keys are rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    /// Bot name, used in logs and by the CLI.
    pub name: String,
    /// Default prefix; the per-guild store value wins.
    #[serde(default = "default_prefix")]
    pub prefix: String,
    /// Directory of the JSON stores, relative to the config file.
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    /// Presence lines rotated every 45s. Empty = no rotation.
    #[serde(default)]
    pub presence: Vec<String>,
    /// Default per-command cooldown in ms (0 = off).
    #[serde(default)]
    pub default_cooldown_ms: u64,
    /// Origin used for discovery (default `https://fluxer.app`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// Explicit API base (skips discovery).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
    /// Explicit gateway URL (skips discovery).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_url: Option<String>,
}

fn check_url(field: &str, value: &str, schemes: &[&str]) -> Result<(), ConfigError> {
    let ok = schemes.iter().any(|scheme| {
        value
            .strip_prefix(scheme)
            .and_then(|rest| rest.strip_prefix("://"))
            .is_some_and(|host| !host.is_empty() && !host.contains(char::is_whitespace))
    });
    if ok {
        Ok(())
    } else {
        Err(ConfigError::new(
            "CONFIG_INVALID",
            format!("{field} must be a {} URL.", schemes.join("/")),
        ))
    }
}

impl ProjectConfig {
    /// Parses and validates the text of a `flush.toml`.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(text)
            .map_err(|e| ConfigError::new("CONFIG_INVALID", e.message().to_owned()))?;
        config.validate()?;
        Ok(config)
    }

    /// Reads and validates a config file.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            let code = if e.kind() == std::io::ErrorKind::NotFound {
                "CONFIG_MISSING"
            } else {
                "CONFIG_INVALID"
            };
            ConfigError::new(code, format!("Cannot read {}: {e}.", path.display()))
        })?;
        Self::parse(&text)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |message: &str| Err(ConfigError::new("CONFIG_INVALID", message));
        if self.name.trim().is_empty() {
            return invalid("name must not be empty.");
        }
        if self.prefix.is_empty()
            || self.prefix.chars().count() > 32
            || self.prefix.contains(char::is_whitespace)
        {
            return invalid("prefix must be 1 to 32 characters without whitespace.");
        }
        if self.data_dir.as_os_str().is_empty() {
            return invalid("data_dir must not be empty.");
        }
        if let Some(origin) = &self.origin {
            check_url("origin", origin, &["https", "http"])?;
        }
        if let Some(api_base) = &self.api_base {
            check_url("api_base", api_base, &["https", "http"])?;
        }
        if let Some(gateway_url) = &self.gateway_url {
            check_url("gateway_url", gateway_url, &["wss", "ws"])?;
        }
        Ok(())
    }

    /// Absolute data directory for a config living in `dir`.
    pub fn data_path(&self, dir: &Path) -> PathBuf {
        dir.join(&self.data_dir)
    }

    /// Builds the router config; `dir` is the directory of the config file.
    pub fn bot_config(&self, dir: &Path, token: impl Into<String>) -> BotConfig {
        let mut config = BotConfig::new(token);
        config.default_prefix = self.prefix.clone();
        config.origin = self.origin.clone();
        config.api_base = self.api_base.clone();
        config.gateway_url = self.gateway_url.clone();
        config.data_path = Some(self.data_path(dir).join("guilds.json"));
        config.presence_lines = self.presence.clone();
        config.default_cooldown_ms = self.default_cooldown_ms;
        config
    }
}

/// Resolves the config path: an explicit file, a directory holding
/// `flush.toml`, or `flush.toml` in the current directory.
pub fn find_config(input: Option<&Path>) -> Result<PathBuf, ConfigError> {
    let candidate = match input {
        Some(path) if path.is_dir() => path.join(CONFIG_FILE),
        Some(path) => path.to_path_buf(),
        None => PathBuf::from(CONFIG_FILE),
    };
    if !candidate.is_file() {
        return Err(ConfigError::new(
            "CONFIG_MISSING",
            format!(
                "Config not found at {}. Run `flush init my-bot` or pass the path.",
                candidate.display()
            ),
        ));
    }
    Ok(std::path::absolute(&candidate).unwrap_or(candidate))
}

/// Parses `.env` text: `KEY=value` lines, `#` comments, optional `export`,
/// single or double quotes (double quotes understand `\n`, `\"` and `\\`).
pub fn parse_env(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, raw)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
        {
            continue;
        }
        let raw = raw.trim();
        let value = if let Some(inner) = raw.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
            inner
                .replace("\\n", "\n")
                .replace("\\\"", "\"")
                .replace("\\\\", "\\")
        } else if let Some(inner) = raw.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
            inner.to_owned()
        } else {
            match raw.find(" #") {
                Some(at) => raw[..at].trim_end().to_owned(),
                None => raw.to_owned(),
            }
        };
        out.push((key.to_owned(), value));
    }
    out
}

/// Reads `dir/.env` without touching the process environment.
/// A missing file is an empty list.
pub fn read_env_file(dir: &Path) -> std::io::Result<Vec<(String, String)>> {
    match std::fs::read_to_string(dir.join(".env")) {
        Ok(text) => Ok(parse_env(&text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// Loads `dir/.env` into the process environment. Variables that are already
/// set keep their value. Call it before spawning tasks or threads.
pub fn load_env(dir: &Path) -> std::io::Result<()> {
    for (key, value) in read_env_file(dir)? {
        if std::env::var_os(&key).is_none() {
            std::env::set_var(key, value);
        }
    }
    Ok(())
}

/// Environment variable naming the config file. `flush run`/`dev` set it so
/// the bot reads the same file the CLI validated.
pub const CONFIG_ENV: &str = "FLUSH_CONFIG";

/// Start-up path of a scaffolded bot: finds the config (`input`, then
/// [`CONFIG_ENV`], then `flush.toml` in the current directory), loads `.env`,
/// points `FLUSH_DATA_DIR` at the configured data directory (unless already
/// set) and builds the [`Bot`] with the token from `FLUXER_BOT_TOKEN`.
pub fn bot_from_project(input: Option<&Path>) -> Result<(Bot, ProjectConfig), ConfigError> {
    let from_env = std::env::var_os(CONFIG_ENV)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    let path = find_config(input.or(from_env.as_deref()))?;
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let config = ProjectConfig::load(&path)?;
    load_env(&dir)
        .map_err(|e| ConfigError::new("CONFIG_INVALID", format!("Cannot read .env: {e}.")))?;
    if std::env::var_os("FLUSH_DATA_DIR").is_none() {
        std::env::set_var("FLUSH_DATA_DIR", config.data_path(&dir));
    }
    let token = std::env::var("FLUXER_BOT_TOKEN").unwrap_or_default();
    if token.trim().is_empty() {
        return Err(ConfigError::new(
            "TOKEN_MISSING",
            "FLUXER_BOT_TOKEN is not set. Copy .env.example to .env and fill it in.",
        ));
    }
    let bot = Bot::new(config.bot_config(&dir, token))
        .map_err(|e| ConfigError::new("CONFIG_INVALID", e.to_string()))?;
    Ok((bot, config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config_with_defaults() {
        let config = ProjectConfig::parse("name = \"demo\"\n").unwrap();
        assert_eq!(config.prefix, "!");
        assert_eq!(config.data_dir, PathBuf::from("data"));
        assert!(config.presence.is_empty());
    }

    #[test]
    fn rejects_unknown_keys_and_bad_values() {
        let unknown = ProjectConfig::parse("name = \"demo\"\nnope = 1\n").unwrap_err();
        assert_eq!(unknown.code, "CONFIG_INVALID");
        assert!(ProjectConfig::parse("name = \"demo\"\nprefix = \"a b\"\n").is_err());
        assert!(ProjectConfig::parse("name = \" \"\n").is_err());
        assert!(ProjectConfig::parse("name = \"demo\"\norigin = \"ftp://x\"\n").is_err());
        assert!(
            ProjectConfig::parse("name = \"demo\"\ngateway_url = \"wss://gw.example\"\n").is_ok()
        );
    }

    #[test]
    fn bot_config_resolves_paths_against_the_config_dir() {
        let config = ProjectConfig::parse(
            "name = \"demo\"\nprefix = \"?\"\ndata_dir = \"state\"\npresence = [\"hi\"]\ndefault_cooldown_ms = 500\n",
        )
        .unwrap();
        let bot = config.bot_config(Path::new("/srv/bot"), "token");
        assert_eq!(bot.default_prefix, "?");
        assert_eq!(
            bot.data_path,
            Some(PathBuf::from("/srv/bot/state/guilds.json"))
        );
        assert_eq!(bot.presence_lines, vec!["hi".to_owned()]);
        assert_eq!(bot.default_cooldown_ms, 500);
    }

    #[test]
    fn parses_env_text() {
        let env = parse_env(
            "# comment\nFLUXER_BOT_TOKEN=abc\nexport PREFIX=\"?\"\nA='x y' \nB=plain # note\nC=\"l1\\nl2\"\nbad line\n=x\n",
        );
        assert_eq!(
            env,
            vec![
                ("FLUXER_BOT_TOKEN".to_owned(), "abc".to_owned()),
                ("PREFIX".to_owned(), "?".to_owned()),
                ("A".to_owned(), "x y".to_owned()),
                ("B".to_owned(), "plain".to_owned()),
                ("C".to_owned(), "l1\nl2".to_owned()),
            ]
        );
    }

    #[test]
    fn find_config_reports_missing() {
        let dir = std::env::temp_dir().join(format!("flush-project-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(find_config(Some(&dir)).unwrap_err().code, "CONFIG_MISSING");
        std::fs::write(dir.join(CONFIG_FILE), "name = \"demo\"\n").unwrap();
        assert!(find_config(Some(&dir)).unwrap().ends_with(CONFIG_FILE));
        assert_eq!(read_env_file(&dir).unwrap(), Vec::new());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
