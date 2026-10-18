//! Config por servidor (espelho de `Sour lib/guildConfig.ts`).
//!
//! JSON local com escrita atômica (`guilds.json.<pid>.tmp` + rename),
//! normalização defensiva e leitura via env (`PREFIX`, `MOD_LOG_CHANNEL_ID`).

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoModConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "t")]
    pub delete_messages: bool,
    #[serde(default = "t")]
    pub block_invites: bool,
    #[serde(default)]
    pub block_links: bool,
    #[serde(default = "t")]
    pub block_caps: bool,
    #[serde(default = "t")]
    pub block_mention_spam: bool,
    #[serde(default = "five")]
    pub max_mentions: u32,
    #[serde(default = "eight_k")]
    pub spam_window_ms: u64,
    #[serde(default = "five")]
    pub spam_max_messages: u32,
    #[serde(default)]
    pub ignored_channels: Vec<String>,
    #[serde(default)]
    pub ignored_roles: Vec<String>,
    #[serde(default)]
    pub raid_protection: bool,
    #[serde(default = "thirty_k")]
    pub raid_window_ms: u64,
    #[serde(default = "five")]
    pub raid_max_joins: u32,
    #[serde(default)]
    pub quarantine_role_id: Option<String>,
}

fn t() -> bool {
    true
}
fn five() -> u32 {
    5
}
fn eight_k() -> u64 {
    8_000
}
fn thirty_k() -> u64 {
    30_000
}

impl Default for AutoModConfig {
    fn default() -> Self {
        serde_json::from_value(serde_json::json!({})).unwrap()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildConfig {
    #[serde(default = "default_prefix")]
    pub prefix: String,
    #[serde(default)]
    pub mod_log_channel_id: Option<String>,
    #[serde(default)]
    pub bot_color: Option<u32>,
    #[serde(default)]
    pub automod: AutoModConfig,
}

fn default_prefix() -> String {
    std::env::var("PREFIX")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "!".into())
}

impl Default for GuildConfig {
    fn default() -> Self {
        Self {
            prefix: default_prefix(),
            mod_log_channel_id: std::env::var("MOD_LOG_CHANNEL_ID")
                .ok()
                .filter(|s| !s.trim().is_empty()),
            bot_color: None,
            automod: AutoModConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct GuildConfigPatch {
    pub prefix: Option<String>,
    pub mod_log_channel_id: Option<Option<String>>,
    pub bot_color: Option<Option<u32>>,
}

fn default_data_dir() -> PathBuf {
    std::env::var("FLUSH_DATA_DIR")
        .or_else(|_| std::env::var("SOUR_DATA_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data"))
}

/// Store com cache em memória + escrita atômica (espelho do Sour).
#[derive(Debug)]
pub struct GuildStore {
    path: PathBuf,
    cache: Option<HashMap<String, GuildConfig>>,
}

impl GuildStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path, cache: None }
    }

    pub fn default_path() -> PathBuf {
        default_data_dir().join("guilds.json")
    }

    pub fn with_default_path() -> Self {
        Self::new(Self::default_path())
    }

    pub fn get(&mut self, guild_id: &str) -> GuildConfig {
        let store = self.load();
        store.get(guild_id).cloned().unwrap_or_default()
    }

    pub fn prefix(&mut self, guild_id: Option<&str>) -> String {
        guild_id.map(|g| self.get(g).prefix).unwrap_or_default()
    }

    pub fn update(&mut self, guild_id: &str, patch: GuildConfigPatch) -> GuildConfig {
        let mut cfg = self.get(guild_id);
        if let Some(p) = patch.prefix {
            let p = p.trim().to_owned();
            if !p.is_empty() {
                cfg.prefix = p.chars().take(3).collect();
            }
        }
        if let Some(id) = patch.mod_log_channel_id {
            cfg.mod_log_channel_id = id.filter(|s| !s.is_empty());
        }
        if let Some(c) = patch.bot_color {
            cfg.bot_color = c;
        }
        let mut store = self.load().clone();
        store.insert(guild_id.to_owned(), cfg.clone());
        self.save(&store);
        cfg
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    fn load(&mut self) -> &HashMap<String, GuildConfig> {
        if self.cache.is_none() {
            let mut map = HashMap::new();
            if let Ok(text) = std::fs::read_to_string(&self.path) {
                if let Ok(parsed) = serde_json::from_str::<HashMap<String, GuildConfig>>(&text) {
                    map = parsed;
                }
            }
            self.cache = Some(map);
        }
        self.cache.as_ref().expect("cache carregado")
    }

    fn save(&mut self, store: &HashMap<String, GuildConfig>) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = self
            .path
            .with_extension(format!("{}.tmp", std::process::id()));
        if serde_json::to_string_pretty(store)
            .ok()
            .and_then(|t| std::fs::write(&tmp, t).ok())
            .is_some()
        {
            let _ = std::fs::rename(&tmp, &self.path);
        }
        self.cache = Some(store.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_roundtrip_atomic() {
        let dir = std::env::temp_dir().join(format!("flush-test-{}", std::process::id()));
        let path = dir.join("guilds.json");
        let mut store = GuildStore::new(path.clone());
        let cfg = store.update(
            "123",
            GuildConfigPatch {
                prefix: Some("$".into()),
                mod_log_channel_id: Some(Some("9".into())),
                bot_color: Some(Some(0xFCE300)),
            },
        );
        assert_eq!(cfg.prefix, "$");
        assert_eq!(cfg.bot_color, Some(0xFCE300));
        let mut reopened = GuildStore::new(path);
        assert_eq!(reopened.get("123").prefix, "$");
        let _ = std::fs::remove_dir_all(dir);
    }
}
