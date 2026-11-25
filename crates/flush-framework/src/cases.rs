//! Moderation cases (mirrors `Sour lib/cases.ts`).
//!
//! Local JSON with atomic writes, sequential Case # per server,
//! active warn counting and warn reversal.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::store_lock::StoreLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaseType {
    Warn,
    Unwarn,
    Kick,
    Ban,
    Unban,
    Softban,
    Timeout,
    Untimeout,
    Clear,
    Lock,
    Unlock,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModCase {
    pub id: u64,
    #[serde(rename = "type")]
    pub kind: CaseType,
    pub target_id: String,
    pub moderator_id: String,
    pub reason: String,
    pub timestamp: u64,
    #[serde(default)]
    pub reverted: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct GuildCases {
    next_id: u64,
    #[serde(default)]
    cases: Vec<ModCase>,
}

impl CaseType {
    /// Lowercase wire name, as stored in JSON and SQLite.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warn => "warn",
            Self::Unwarn => "unwarn",
            Self::Kick => "kick",
            Self::Ban => "ban",
            Self::Unban => "unban",
            Self::Softban => "softban",
            Self::Timeout => "timeout",
            Self::Untimeout => "untimeout",
            Self::Clear => "clear",
            Self::Lock => "lock",
            Self::Unlock => "unlock",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(name.to_owned())).ok()
    }
}

impl GuildCases {
    fn next(&self) -> u64 {
        self.next_id.max(1)
    }
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn default_path() -> PathBuf {
    let dir = std::env::var("FLUSH_DATA_DIR")
        .or_else(|_| std::env::var("SOUR_DATA_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data"));
    dir.join("cases.json")
}

#[derive(Debug)]
pub struct CaseStore {
    path: PathBuf,
    cache: Option<HashMap<String, GuildCases>>,
    _lock: Option<StoreLock>,
}

impl CaseStore {
    /// Opens the store and marks it as in use (see [`StoreLock`]) until it is
    /// dropped. Waits while a tool is replacing the file. A lock that cannot
    /// be taken (a read-only directory, say) leaves the store unlocked.
    pub fn new(path: PathBuf) -> Self {
        let lock = StoreLock::shared(&path).ok();
        Self {
            path,
            cache: None,
            _lock: lock,
        }
    }

    pub fn with_default_path() -> Self {
        Self::new(default_path())
    }

    /// Records a case. `Err` means nothing was saved, so the id must not be
    /// announced.
    pub fn add(
        &mut self,
        guild_id: &str,
        kind: CaseType,
        target_id: &str,
        moderator_id: &str,
        reason: &str,
    ) -> std::io::Result<ModCase> {
        let mut store = self.load().clone();
        let entry = store.entry(guild_id.to_owned()).or_default();
        let id = entry.next();
        entry.next_id = id + 1;
        let case = ModCase {
            id,
            kind,
            target_id: target_id.to_owned(),
            moderator_id: moderator_id.to_owned(),
            reason: reason.to_owned(),
            timestamp: now_ms(),
            reverted: false,
        };
        entry.cases.push(case.clone());
        self.save(&store)?;
        Ok(case)
    }

    pub fn for_user(&mut self, guild_id: &str, user_id: &str, limit: usize) -> Vec<ModCase> {
        let store = self.load();
        let mut out: Vec<ModCase> = store
            .get(guild_id)
            .map(|g| {
                g.cases
                    .iter()
                    .filter(|c| c.target_id == user_id)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by_key(|c| c.id);
        if out.len() > limit {
            out = out[out.len() - limit..].to_vec();
        }
        out
    }

    pub fn get(&mut self, guild_id: &str, case_id: u64) -> Option<ModCase> {
        self.load()
            .get(guild_id)?
            .cases
            .iter()
            .find(|c| c.id == case_id)
            .cloned()
    }

    pub fn active_warns(&mut self, guild_id: &str, user_id: &str) -> usize {
        self.load()
            .get(guild_id)
            .map(|g| {
                g.cases
                    .iter()
                    .filter(|c| c.target_id == user_id && c.kind == CaseType::Warn && !c.reverted)
                    .count()
            })
            .unwrap_or(0)
    }

    /// Reverts an active warn (mirrors `revertWarnCase`). `Ok(None)` when
    /// there is no such active warn; `Err` when the change could not be saved.
    pub fn revert_warn(
        &mut self,
        guild_id: &str,
        user_id: &str,
        case_id: u64,
    ) -> std::io::Result<Option<ModCase>> {
        let mut store = self.load().clone();
        let found = store.get_mut(guild_id).and_then(|entry| {
            entry.cases.iter_mut().find(|c| {
                c.id == case_id && c.target_id == user_id && c.kind == CaseType::Warn && !c.reverted
            })
        });
        let Some(case) = found else {
            return Ok(None);
        };
        case.reverted = true;
        let out = case.clone();
        self.save(&store)?;
        Ok(Some(out))
    }

    fn load(&mut self) -> &HashMap<String, GuildCases> {
        if self.cache.is_none() {
            let mut map = HashMap::new();
            if let Ok(text) = std::fs::read_to_string(&self.path) {
                if let Ok(parsed) = serde_json::from_str::<HashMap<String, GuildCases>>(&text) {
                    map = parsed;
                }
            }
            // Normalize next_id to len+1 when absent.
            for g in map.values_mut() {
                if g.next_id == 0 {
                    g.next_id = g.cases.iter().map(|c| c.id).max().unwrap_or(0) + 1;
                }
            }
            self.cache = Some(map);
        }
        self.cache.as_ref().expect("cache loaded")
    }

    /// Writes the file atomically; the cache only changes once it is on disk.
    fn save(&mut self, store: &HashMap<String, GuildCases>) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self
            .path
            .with_extension(format!("{}.tmp", std::process::id()));
        let text = serde_json::to_string_pretty(store)?;
        let written = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, &self.path));
        if let Err(error) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(error);
        }
        self.cache = Some(store.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_ids_and_revert() {
        let dir = std::env::temp_dir().join(format!("flush-cases-{}", std::process::id()));
        let mut store = CaseStore::new(dir.join("cases.json"));
        let c1 = store.add("g", CaseType::Warn, "u", "m", "spam").unwrap();
        let c2 = store.add("g", CaseType::Warn, "u", "m", "spam2").unwrap();
        assert_eq!((c1.id, c2.id), (1, 2));
        assert_eq!(store.active_warns("g", "u"), 2);
        assert!(store.revert_warn("g", "u", 1).unwrap().is_some());
        assert_eq!(store.active_warns("g", "u"), 1);
        assert_eq!(store.for_user("g", "u", 15).len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_write_is_an_error_and_keeps_no_case() {
        let dir = std::env::temp_dir().join(format!("flush-cases-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The parent "directory" is a file, so nothing can be written under it.
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, "x").unwrap();
        let mut store = CaseStore::new(blocker.join("cases.json"));
        assert!(store.add("g", CaseType::Warn, "u", "m", "spam").is_err());
        assert_eq!(store.active_warns("g", "u"), 0);
        assert_eq!(store.get("g", 1), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
