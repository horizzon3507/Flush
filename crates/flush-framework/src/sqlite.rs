//! SQLite store (mirrors Yuzuha `infrastructure/storage/sqlite.ts`).
//!
//! One database file holds scoped variables, moderation cases and guild
//! configuration in normalized tables. Writes run in `BEGIN IMMEDIATE`
//! transactions, the journal is WAL and variable reads and queries go
//! straight to the index instead of loading the whole state.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::Value;

use crate::cases::{now_ms, CaseType, ModCase};
use crate::guild_store::{default_data_dir, GuildConfig, GuildConfigPatch};
use crate::variables::{
    name_key, StoreError, StoredVariable, TransactionBody, Ttl, VariableAccess, VariableEntry,
    VariableError, VariableKey, VariablePage, VariableQuery, VariableStore,
};

/// Version written to `flush_meta`; a newer file is refused, not downgraded.
pub const SCHEMA_VERSION: i64 = 1;

const DDL: &str = "
CREATE TABLE IF NOT EXISTS flush_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS flush_variables (
  scope TEXT NOT NULL,
  subject TEXT NOT NULL,
  name_key TEXT NOT NULL,
  name TEXT NOT NULL,
  value_json TEXT NOT NULL,
  expires_at INTEGER,
  PRIMARY KEY (scope, subject, name_key)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS flush_variables_expiry
  ON flush_variables (expires_at) WHERE expires_at IS NOT NULL;
CREATE TABLE IF NOT EXISTS flush_cases (
  guild_id TEXT NOT NULL,
  id INTEGER NOT NULL,
  type TEXT NOT NULL,
  target_id TEXT NOT NULL,
  moderator_id TEXT NOT NULL,
  reason TEXT NOT NULL,
  timestamp INTEGER NOT NULL,
  reverted INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (guild_id, id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS flush_cases_target
  ON flush_cases (guild_id, target_id, id);
CREATE TABLE IF NOT EXISTS flush_case_counters (
  guild_id TEXT PRIMARY KEY,
  next_id INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS flush_guilds (
  guild_id TEXT PRIMARY KEY,
  config_json TEXT NOT NULL
) WITHOUT ROWID;
";

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Backend(error.to_string())
    }
}

fn io(error: std::io::Error) -> StoreError {
    StoreError::Backend(error.to_string())
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(io)
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) -> Result<(), StoreError> {
    Ok(())
}

fn stored(value_json: String, expires_at: Option<i64>) -> Result<StoredVariable, StoreError> {
    Ok(StoredVariable {
        value: serde_json::from_str(&value_json).map_err(|_| StoreError::Corrupt)?,
        expires_at: match expires_at {
            None => None,
            Some(at) => Some(u64::try_from(at).map_err(|_| StoreError::Corrupt)?),
        },
    })
}

fn case_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<(ModCase, String)> {
    let kind: String = row.get(1)?;
    Ok((
        ModCase {
            id: row.get::<_, i64>(0)? as u64,
            kind: CaseType::Warn,
            target_id: row.get(2)?,
            moderator_id: row.get(3)?,
            reason: row.get(4)?,
            timestamp: row.get::<_, i64>(5)? as u64,
            reverted: row.get(6)?,
        },
        kind,
    ))
}

fn typed((mut case, kind): (ModCase, String)) -> Result<ModCase, StoreError> {
    case.kind = CaseType::parse(&kind).ok_or(StoreError::Corrupt)?;
    Ok(case)
}

const CASE_COLUMNS: &str = "id, type, target_id, moderator_id, reason, timestamp, reverted";

/// SQLite-backed store for variables, cases and guild configuration.
///
/// Methods take `&self`; wrap the store in an `Arc` to share it between a
/// [`crate::VariableService`] and command handlers.
#[derive(Debug)]
pub struct SqliteStore {
    path: Option<PathBuf>,
    conn: Mutex<Option<Connection>>,
}

struct Access<'c>(&'c Connection);

impl VariableAccess for Access<'_> {
    fn get(&mut self, key: &VariableKey) -> Result<Option<StoredVariable>, StoreError> {
        self.0
            .prepare_cached(
                "SELECT value_json, expires_at FROM flush_variables
                 WHERE scope = ? AND subject = ? AND name_key = ?",
            )?
            .query_row(
                params![key.scope.as_str(), key.subject, key.name_key()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .map(|(json, at)| stored(json, at))
            .transpose()
    }

    fn set(
        &mut self,
        key: &VariableKey,
        value: &Value,
        expires_at: Option<u64>,
    ) -> Result<(), StoreError> {
        let json = serde_json::to_string(value).map_err(|e| StoreError::Backend(e.to_string()))?;
        self.0
            .prepare_cached(
                "INSERT INTO flush_variables (scope, subject, name_key, name, value_json, expires_at)
                 VALUES (?, ?, ?, ?, ?, ?)
                 ON CONFLICT (scope, subject, name_key) DO UPDATE SET
                   value_json = excluded.value_json, expires_at = excluded.expires_at",
            )?
            .execute(params![
                key.scope.as_str(),
                key.subject,
                key.name_key(),
                key.name,
                json,
                expires_at.map(|at| at as i64),
            ])?;
        Ok(())
    }

    fn delete(&mut self, key: &VariableKey) -> Result<(), StoreError> {
        self.0
            .prepare_cached(
                "DELETE FROM flush_variables WHERE scope = ? AND subject = ? AND name_key = ?",
            )?
            .execute(params![key.scope.as_str(), key.subject, key.name_key()])?;
        Ok(())
    }
}

impl SqliteStore {
    /// Opens (creating if needed) the database at `path`. On Unix the parent
    /// directory is created as `0700` and the file is kept at `0600`.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let path = path.into();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            if !parent.exists() {
                std::fs::create_dir_all(parent).map_err(io)?;
                restrict(parent, 0o700)?;
            }
        }
        let created = !path.exists();
        let conn = Connection::open(&path)?;
        if created {
            restrict(&path, 0o600)?;
        }
        conn.busy_timeout(Duration::from_secs(5))?;
        // The schema check comes first so a refused file keeps its journal
        // mode and permissions.
        let store = Self::init(conn, Some(path))?;
        store.read(|conn| {
            conn.pragma_update(None, "journal_mode", "WAL")?;
            conn.pragma_update(None, "synchronous", "FULL")?;
            Ok(())
        })?;
        if let Some(path) = store.path() {
            restrict(path, 0o600)?;
        }
        Ok(store)
    }

    /// `data/flush.sqlite` under the same data directory as the JSON stores.
    pub fn default_path() -> PathBuf {
        default_data_dir().join("flush.sqlite")
    }

    pub fn open_default() -> Result<Self, StoreError> {
        Self::open(Self::default_path())
    }

    /// Private in-memory database; gone when the store is dropped.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?, None)
    }

    fn init(mut conn: Connection, path: Option<PathBuf>) -> Result<Self, StoreError> {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let has_meta: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'flush_meta')",
            [],
            |row| row.get(0),
        )?;
        if has_meta {
            let version: Option<String> = tx
                .query_row(
                    "SELECT value FROM flush_meta WHERE key = 'schema'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            match version.as_deref().map(str::parse::<i64>) {
                Some(Ok(SCHEMA_VERSION)) => {}
                _ => return Err(StoreError::Corrupt),
            }
        }
        tx.execute_batch(DDL)?;
        tx.execute(
            "INSERT OR IGNORE INTO flush_meta (key, value) VALUES ('schema', ?)",
            [SCHEMA_VERSION.to_string()],
        )?;
        tx.commit()?;
        Ok(Self {
            path,
            conn: Mutex::new(Some(conn)),
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Closes the database; every later call returns [`StoreError::Closed`].
    pub fn close(&self) -> Result<(), StoreError> {
        match self.lock().take() {
            Some(conn) => conn.close().map_err(|(_, e)| e.into()),
            None => Ok(()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<Connection>> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn read<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        f(self.lock().as_ref().ok_or(StoreError::Closed)?)
    }

    fn write<T, E: From<StoreError>>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, E>,
    ) -> Result<T, E> {
        let mut guard = self.lock();
        let conn = guard.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::from)?;
        // Dropping `tx` on the error path rolls back.
        let out = f(&tx)?;
        tx.commit().map_err(StoreError::from)?;
        Ok(out)
    }

    // --- cases (same surface as `CaseStore`) ---

    /// Appends a case with the guild's next sequential id.
    pub fn add_case(
        &self,
        guild_id: &str,
        kind: CaseType,
        target_id: &str,
        moderator_id: &str,
        reason: &str,
    ) -> Result<ModCase, StoreError> {
        self.write(|conn| {
            let id: i64 = conn.query_row(
                "INSERT INTO flush_case_counters (guild_id, next_id) VALUES (?, 2)
                 ON CONFLICT (guild_id) DO UPDATE SET next_id = next_id + 1
                 RETURNING next_id - 1",
                [guild_id],
                |row| row.get(0),
            )?;
            let case = ModCase {
                id: id as u64,
                kind,
                target_id: target_id.to_owned(),
                moderator_id: moderator_id.to_owned(),
                reason: reason.to_owned(),
                timestamp: now_ms(),
                reverted: false,
            };
            insert_case(conn, guild_id, &case)?;
            Ok(case)
        })
    }

    pub fn case(&self, guild_id: &str, case_id: u64) -> Result<Option<ModCase>, StoreError> {
        self.read(|conn| {
            conn.query_row(
                &format!("SELECT {CASE_COLUMNS} FROM flush_cases WHERE guild_id = ? AND id = ?"),
                params![guild_id, case_id as i64],
                case_from,
            )
            .optional()?
            .map(typed)
            .transpose()
        })
    }

    /// The user's most recent `limit` cases, oldest first.
    pub fn cases_for_user(
        &self,
        guild_id: &str,
        user_id: &str,
        limit: usize,
    ) -> Result<Vec<ModCase>, StoreError> {
        self.read(|conn| {
            let mut statement = conn.prepare_cached(&format!(
                "SELECT {CASE_COLUMNS} FROM flush_cases
                 WHERE guild_id = ? AND target_id = ? ORDER BY id DESC LIMIT ?"
            ))?;
            let rows = statement.query_map(
                params![guild_id, user_id, i64::try_from(limit).unwrap_or(i64::MAX)],
                case_from,
            )?;
            let mut out = Vec::new();
            for row in rows {
                out.push(typed(row?)?);
            }
            out.reverse();
            Ok(out)
        })
    }

    pub fn active_warns(&self, guild_id: &str, user_id: &str) -> Result<usize, StoreError> {
        self.read(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM flush_cases
                 WHERE guild_id = ? AND target_id = ? AND type = 'warn' AND reverted = 0",
                [guild_id, user_id],
                |row| row.get(0),
            )?;
            Ok(count as usize)
        })
    }

    /// Reverts an active warn; `None` when it is missing or already reverted.
    pub fn revert_warn(
        &self,
        guild_id: &str,
        user_id: &str,
        case_id: u64,
    ) -> Result<Option<ModCase>, StoreError> {
        self.write(|conn| {
            conn.query_row(
                &format!(
                    "UPDATE flush_cases SET reverted = 1
                     WHERE guild_id = ? AND id = ? AND target_id = ? AND type = 'warn' AND reverted = 0
                     RETURNING {CASE_COLUMNS}"
                ),
                params![guild_id, case_id as i64, user_id],
                case_from,
            )
            .optional()?
            .map(typed)
            .transpose()
        })
    }

    // --- guild configuration (same surface as `GuildStore`) ---

    pub fn guild_config(&self, guild_id: &str) -> Result<GuildConfig, StoreError> {
        self.read(|conn| read_guild(conn, guild_id))
    }

    pub fn prefix(&self, guild_id: Option<&str>) -> Result<String, StoreError> {
        match guild_id {
            Some(id) => Ok(self.guild_config(id)?.prefix),
            None => Ok(String::new()),
        }
    }

    pub fn update_guild_config(
        &self,
        guild_id: &str,
        patch: GuildConfigPatch,
    ) -> Result<GuildConfig, StoreError> {
        self.write(|conn| {
            let mut config = read_guild(conn, guild_id)?;
            config.apply(patch);
            write_guild(conn, guild_id, &config)?;
            Ok(config)
        })
    }

    /// Replaces a guild's whole configuration (automod included).
    pub fn set_guild_config(&self, guild_id: &str, config: &GuildConfig) -> Result<(), StoreError> {
        self.write(|conn| write_guild(conn, guild_id, config))
    }

    // --- migration from the JSON stores ---

    /// Copies `guilds.json` (the [`crate::GuildStore`] file) into this store
    /// in one transaction and returns how many guilds were written. Existing
    /// rows for the same guilds are replaced; a missing file imports nothing.
    /// A case store file is refused: its entries would read as default
    /// settings and reset those guilds.
    pub fn import_guilds_json(&self, path: &Path) -> Result<usize, StoreError> {
        let Some(guilds) = read_store_file::<GuildConfig>(
            path,
            CASE_FIELDS,
            "moderation cases",
            "guild settings",
        )?
        else {
            return Ok(0);
        };
        self.write(|conn| {
            for (guild_id, config) in &guilds {
                write_guild(conn, guild_id, config)?;
            }
            Ok(guilds.len())
        })
    }

    /// Copies `cases.json` (the [`crate::CaseStore`] file) into this store in
    /// one transaction, keeping ids, and returns how many cases were written.
    /// Cases whose id already exists here are left alone.
    pub fn import_cases_json(&self, path: &Path) -> Result<usize, StoreError> {
        #[derive(serde::Deserialize)]
        struct Guild {
            #[serde(default)]
            cases: Vec<ModCase>,
            #[serde(default)]
            next_id: u64,
        }
        let Some(guilds) =
            read_store_file::<Guild>(path, GUILD_FIELDS, "guild settings", "moderation cases")?
        else {
            return Ok(0);
        };
        self.write(|conn| {
            let mut written = 0;
            for (guild_id, guild) in &guilds {
                let mut next = guild.next_id.max(1);
                for case in &guild.cases {
                    written += insert_case(conn, guild_id, case)?;
                    next = next.max(case.id + 1);
                }
                conn.execute(
                    "INSERT INTO flush_case_counters (guild_id, next_id) VALUES (?1, ?2)
                     ON CONFLICT (guild_id) DO UPDATE SET next_id = MAX(next_id, ?2)",
                    params![guild_id, next as i64],
                )?;
            }
            Ok(written)
        })
    }
}

const CASE_FIELDS: &[&str] = &["cases", "next_id"];
const GUILD_FIELDS: &[&str] = &["prefix", "mod_log_channel_id", "bot_color", "automod"];

/// Reads a JSON store file: a map from guild id to `T`. Both stores use that
/// shape with every field defaulted, so each file parses as the other kind;
/// `other_fields` are the fields that give the other kind away. A repeated
/// guild id or field is refused instead of keeping the last one.
fn read_store_file<T: serde::de::DeserializeOwned>(
    path: &Path,
    other_fields: &[&str],
    found: &str,
    wanted: &str,
) -> Result<Option<Vec<(String, T)>>, StoreError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io(error)),
    };
    let Guilds(raw) = serde_json::from_str::<Guilds<serde_json::Value>>(&text)
        .map_err(|_| StoreError::Corrupt)?;
    let other_kind = raw
        .iter()
        .filter_map(|(_, entry)| entry.as_object())
        .any(|entry| other_fields.iter().any(|field| entry.contains_key(*field)));
    if other_kind {
        return Err(StoreError::Import(format!(
            "{} holds {found}, not {wanted}",
            path.display()
        )));
    }
    // Parsed from the text again, not from `raw`: a `Value` has already
    // dropped repeated fields, the typed parse refuses them.
    let Guilds(guilds) =
        serde_json::from_str::<Guilds<T>>(&text).map_err(|_| StoreError::Corrupt)?;
    Ok(Some(guilds))
}

/// A JSON object read as its entries in order, refusing a repeated key.
struct Guilds<T>(Vec<(String, T)>);

impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for Guilds<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entries<T>(std::marker::PhantomData<T>);
        impl<'de, T: serde::Deserialize<'de>> serde::de::Visitor<'de> for Entries<T> {
            type Value = Guilds<T>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object keyed by guild id")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut access: A,
            ) -> Result<Self::Value, A::Error> {
                let mut seen = std::collections::HashSet::new();
                let mut entries = Vec::new();
                while let Some((key, value)) = access.next_entry::<String, T>()? {
                    if !seen.insert(key.clone()) {
                        return Err(serde::de::Error::custom("a guild id is repeated"));
                    }
                    entries.push((key, value));
                }
                Ok(Guilds(entries))
            }
        }
        deserializer.deserialize_map(Entries(std::marker::PhantomData))
    }
}

fn insert_case(conn: &Connection, guild_id: &str, case: &ModCase) -> Result<usize, StoreError> {
    Ok(conn
        .prepare_cached(
            "INSERT OR IGNORE INTO flush_cases
             (guild_id, id, type, target_id, moderator_id, reason, timestamp, reverted)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )?
        .execute(params![
            guild_id,
            case.id as i64,
            case.kind.as_str(),
            case.target_id,
            case.moderator_id,
            case.reason,
            case.timestamp as i64,
            case.reverted,
        ])?)
}

fn read_guild(conn: &Connection, guild_id: &str) -> Result<GuildConfig, StoreError> {
    let json: Option<String> = conn
        .query_row(
            "SELECT config_json FROM flush_guilds WHERE guild_id = ?",
            [guild_id],
            |row| row.get(0),
        )
        .optional()?;
    match json {
        None => Ok(GuildConfig::default()),
        Some(json) => serde_json::from_str(&json).map_err(|_| StoreError::Corrupt),
    }
}

fn write_guild(conn: &Connection, guild_id: &str, config: &GuildConfig) -> Result<(), StoreError> {
    let json = serde_json::to_string(config).map_err(|e| StoreError::Backend(e.to_string()))?;
    conn.execute(
        "INSERT INTO flush_guilds (guild_id, config_json) VALUES (?, ?)
         ON CONFLICT (guild_id) DO UPDATE SET config_json = excluded.config_json",
        params![guild_id, json],
    )?;
    Ok(())
}

impl VariableStore for SqliteStore {
    fn read_entry(
        &self,
        key: &VariableKey,
        now: u64,
    ) -> Result<Option<StoredVariable>, StoreError> {
        self.read(|conn| {
            Ok(Access(conn)
                .get(key)?
                .filter(|v| v.expires_at.is_none_or(|at| at > now)))
        })
    }

    fn query(&self, query: &VariableQuery, now: u64) -> Result<VariablePage, StoreError> {
        self.read(|conn| {
            let prefix = name_key(&query.prefix);
            // Keys are lowercase hex, so `prefix + "g"` bounds the prefix range.
            let upper = format!("{prefix}g");
            let after = query.after.as_deref().map(name_key).unwrap_or_default();
            let mut statement = conn.prepare_cached(
                "SELECT name, value_json, expires_at FROM flush_variables
                 WHERE scope = ?1 AND subject = ?2
                   AND name_key >= ?3 AND name_key < ?4 AND name_key > ?5
                   AND (expires_at IS NULL OR expires_at > ?6)
                 ORDER BY name_key LIMIT ?7",
            )?;
            let mut rows = statement.query(params![
                query.scope.as_str(),
                query.subject,
                prefix,
                upper,
                after,
                now as i64,
                (query.limit + 1) as i64,
            ])?;
            let mut entries = Vec::new();
            while let Some(row) = rows.next()? {
                let entry = stored(row.get(1)?, row.get(2)?)?;
                entries.push(VariableEntry {
                    name: row.get(0)?,
                    ttl: match entry.expires_at {
                        None => Ttl::Persistent,
                        Some(at) => Ttl::Remaining(at - now),
                    },
                    value: entry.value,
                });
            }
            Ok(VariablePage::from_entries(entries, query.limit))
        })
    }

    fn transact(&self, body: &mut TransactionBody<'_>) -> Result<(), VariableError> {
        self.write(|conn| body(&mut Access(conn)))
    }

    fn sweep_expired(&self, now: u64, limit: usize) -> Result<usize, StoreError> {
        self.write(|conn| {
            Ok(conn.execute(
                "DELETE FROM flush_variables WHERE (scope, subject, name_key) IN (
                   SELECT scope, subject, name_key FROM flush_variables
                   WHERE expires_at IS NOT NULL AND expires_at <= ? LIMIT ?)",
                params![now as i64, i64::try_from(limit).unwrap_or(i64::MAX)],
            )?)
        })
    }
}
