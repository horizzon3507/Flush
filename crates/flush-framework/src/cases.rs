//! Cases de moderação (espelho de `Sour lib/cases.ts`).
//!
//! JSON local com escrita atômica, Case # sequencial por servidor,
//! contagem de warns ativos e reversão de warn.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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

impl GuildCases {
    fn next(&self) -> u64 {
        self.next_id.max(1)
    }
}

fn now_ms() -> u64 {
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
}

impl CaseStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path, cache: None }
    }

    pub fn with_default_path() -> Self {
        Self::new(default_path())
    }

    pub fn add(
        &mut self,
        guild_id: &str,
        kind: CaseType,
        target_id: &str,
        moderator_id: &str,
        reason: &str,
    ) -> ModCase {
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
        self.save(&store);
        case
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

    /// Reverte um warn ativo (espelho de `revertWarnCase`).
    pub fn revert_warn(&mut self, guild_id: &str, user_id: &str, case_id: u64) -> Option<ModCase> {
        let mut store = self.load().clone();
        let entry = store.get_mut(guild_id)?;
        let case = entry.cases.iter_mut().find(|c| {
            c.id == case_id && c.target_id == user_id && c.kind == CaseType::Warn && !c.reverted
        })?;
        case.reverted = true;
        let out = case.clone();
        self.save(&store);
        Some(out)
    }

    fn load(&mut self) -> &HashMap<String, GuildCases> {
        if self.cache.is_none() {
            let mut map = HashMap::new();
            if let Ok(text) = std::fs::read_to_string(&self.path) {
                if let Ok(parsed) = serde_json::from_str::<HashMap<String, GuildCases>>(&text) {
                    map = parsed;
                }
            }
            // Normaliza next_id para len+1 quando ausente.
            for g in map.values_mut() {
                if g.next_id == 0 {
                    g.next_id = g.cases.iter().map(|c| c.id).max().unwrap_or(0) + 1;
                }
            }
            self.cache = Some(map);
        }
        self.cache.as_ref().expect("cache carregado")
    }

    fn save(&mut self, store: &HashMap<String, GuildCases>) {
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
    fn sequential_ids_and_revert() {
        let dir = std::env::temp_dir().join(format!("flush-cases-{}", std::process::id()));
        let mut store = CaseStore::new(dir.join("cases.json"));
        let c1 = store.add("g", CaseType::Warn, "u", "m", "spam");
        let c2 = store.add("g", CaseType::Warn, "u", "m", "spam2");
        assert_eq!((c1.id, c2.id), (1, 2));
        assert_eq!(store.active_warns("g", "u"), 2);
        assert!(store.revert_warn("g", "u", 1).is_some());
        assert_eq!(store.active_warns("g", "u"), 1);
        assert_eq!(store.for_user("g", "u", 15).len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
