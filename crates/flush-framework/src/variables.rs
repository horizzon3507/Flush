//! Scoped variables (mirrors Yuzuha `services/variables.ts`).
//!
//! A [`VariableService`] reads and writes JSON values under three scopes
//! (`global`, `guild`, `user`) with optional TTL, prefix queries with a
//! cursor and atomic transactions. It runs over any [`VariableStore`]:
//! [`MemoryVariableStore`] here, `SqliteStore` behind the `sqlite` feature.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Largest timestamp Yuzuha accepts (`Number.MAX_SAFE_INTEGER`).
pub const MAX_SAFE_MS: u64 = (1 << 53) - 1;
/// Most operations [`execute_operations`] accepts in one transaction.
pub const DATA_TRANSACTION_MAX: usize = 100;
const NAME_MAX: usize = 128;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(
        "stored state is invalid; keep the data and restore a backup, nothing was overwritten"
    )]
    Corrupt,
    #[error("the store is closed")]
    Closed,
    #[error("storage failure: {0}")]
    Backend(String),
}

#[derive(Debug, thiserror::Error)]
pub enum VariableError {
    #[error("this only works inside a server")]
    NotInGuild,
    #[error("no author for a user-scoped variable")]
    NoAuthor,
    #[error("a variable name must have 1 to 128 characters")]
    Name,
    #[error("a user id must have 1 to 128 characters and no surrounding whitespace")]
    UserId,
    #[error("TTL must be a millisecond count inside the safe integer range")]
    Ttl,
    #[error("prefix and cursor take up to 128 characters; limit must be 1 to 100")]
    Query,
    #[error("a variable value nests at most 64 levels deep")]
    Value,
    #[error("{0}")]
    Operation(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VariableScope {
    Global,
    Guild,
    User,
}

impl VariableScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Guild => "guild",
            Self::User => "user",
        }
    }
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Hex UTF-16 code units: sorts like JavaScript strings in SQLite's binary index.
pub fn name_key(name: &str) -> String {
    name.encode_utf16().map(|u| format!("{u:04x}")).collect()
}

/// Identity of one variable: scope, subject (guild or user id, empty for
/// global) and name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VariableKey {
    pub scope: VariableScope,
    pub subject: String,
    pub name: String,
}

impl VariableKey {
    pub fn new(scope: VariableScope, subject: &str, name: &str) -> Result<Self, VariableError> {
        if name.is_empty() || utf16_len(name) > NAME_MAX {
            return Err(VariableError::Name);
        }
        Ok(Self {
            scope,
            subject: subject.to_owned(),
            name: name.to_owned(),
        })
    }

    pub fn name_key(&self) -> String {
        name_key(&self.name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredVariable {
    pub value: Value,
    /// Absolute expiry in epoch milliseconds; `None` is persistent.
    pub expires_at: Option<u64>,
}

impl StoredVariable {
    fn live(&self, now: u64) -> bool {
        self.expires_at.is_none_or(|at| at > now)
    }
}

/// Remaining lifetime of a variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ttl {
    Missing,
    Persistent,
    Remaining(u64),
}

impl Ttl {
    /// Yuzuha's numeric form: remaining ms, `-1` persistent, `-2` missing.
    pub fn as_millis(self) -> i64 {
        match self {
            Self::Missing => -2,
            Self::Persistent => -1,
            Self::Remaining(ms) => ms as i64,
        }
    }

    fn of(entry: Option<&StoredVariable>, now: u64) -> Self {
        match entry {
            None => Self::Missing,
            Some(StoredVariable {
                expires_at: None, ..
            }) => Self::Persistent,
            Some(StoredVariable {
                expires_at: Some(at),
                ..
            }) => Self::Remaining(at - now),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct VariableQueryOptions {
    /// Literal name prefix (default empty).
    pub prefix: Option<String>,
    /// Exclusive cursor: the `next_cursor` of the previous page.
    pub after: Option<String>,
    /// Page size, 1 to 100 (default 50).
    pub limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct VariableQuery {
    pub scope: VariableScope,
    pub subject: String,
    pub prefix: String,
    pub after: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariableEntry {
    pub name: String,
    pub value: Value,
    pub ttl: Ttl,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct VariablePage {
    pub entries: Vec<VariableEntry>,
    pub next_cursor: Option<String>,
}

impl VariablePage {
    /// Builds a page from up to `limit + 1` ordered entries.
    pub fn from_entries(mut entries: Vec<VariableEntry>, limit: usize) -> Self {
        let more = entries.len() > limit;
        entries.truncate(limit);
        let next_cursor = more
            .then(|| entries.last().map(|e| e.name.clone()))
            .flatten();
        Self {
            entries,
            next_cursor,
        }
    }
}

/// Point access handed to a transaction; expired rows are still visible here
/// so the service can drop them.
pub trait VariableAccess {
    fn get(&mut self, key: &VariableKey) -> Result<Option<StoredVariable>, StoreError>;
    fn set(
        &mut self,
        key: &VariableKey,
        value: &Value,
        expires_at: Option<u64>,
    ) -> Result<(), StoreError>;
    fn delete(&mut self, key: &VariableKey) -> Result<(), StoreError>;
}

/// Callback run inside one store transaction.
pub type TransactionBody<'a> = dyn FnMut(&mut dyn VariableAccess) -> Result<(), VariableError> + 'a;

/// Storage port for variables (the variable half of Yuzuha's `Store`).
pub trait VariableStore {
    /// Reads one entry; missing and expired both return `None`.
    fn read_entry(&self, key: &VariableKey, now: u64)
        -> Result<Option<StoredVariable>, StoreError>;
    /// Live entries ordered by name (UTF-16 code unit order).
    fn query(&self, query: &VariableQuery, now: u64) -> Result<VariablePage, StoreError>;
    /// Runs `body` atomically: an error leaves the store untouched.
    fn transact(&self, body: &mut TransactionBody<'_>) -> Result<(), VariableError>;
    /// Deletes up to `limit` expired entries and returns how many went away.
    fn sweep_expired(&self, now: u64, limit: usize) -> Result<usize, StoreError>;
}

impl<S: VariableStore + ?Sized> VariableStore for Arc<S> {
    fn read_entry(
        &self,
        key: &VariableKey,
        now: u64,
    ) -> Result<Option<StoredVariable>, StoreError> {
        (**self).read_entry(key, now)
    }
    fn query(&self, query: &VariableQuery, now: u64) -> Result<VariablePage, StoreError> {
        (**self).query(query, now)
    }
    fn transact(&self, body: &mut TransactionBody<'_>) -> Result<(), VariableError> {
        (**self).transact(body)
    }
    fn sweep_expired(&self, now: u64, limit: usize) -> Result<usize, StoreError> {
        (**self).sweep_expired(now, limit)
    }
}

type MemoryMap = BTreeMap<(VariableScope, String, String), (String, StoredVariable)>;

fn memory_key(key: &VariableKey) -> (VariableScope, String, String) {
    (key.scope, key.subject.clone(), key.name_key())
}

/// In-memory [`VariableStore`] for tests and bots without persistence.
#[derive(Debug, Default)]
pub struct MemoryVariableStore {
    map: Mutex<MemoryMap>,
}

impl MemoryVariableStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MemoryMap> {
        self.map.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl VariableAccess for MemoryMap {
    fn get(&mut self, key: &VariableKey) -> Result<Option<StoredVariable>, StoreError> {
        Ok(BTreeMap::get(self, &memory_key(key)).map(|(_, v)| v.clone()))
    }
    fn set(
        &mut self,
        key: &VariableKey,
        value: &Value,
        expires_at: Option<u64>,
    ) -> Result<(), StoreError> {
        let stored = StoredVariable {
            value: value.clone(),
            expires_at,
        };
        self.insert(memory_key(key), (key.name.clone(), stored));
        Ok(())
    }
    fn delete(&mut self, key: &VariableKey) -> Result<(), StoreError> {
        self.remove(&memory_key(key));
        Ok(())
    }
}

impl VariableStore for MemoryVariableStore {
    fn read_entry(
        &self,
        key: &VariableKey,
        now: u64,
    ) -> Result<Option<StoredVariable>, StoreError> {
        Ok(VariableAccess::get(&mut *self.lock(), key)?.filter(|v| v.live(now)))
    }

    fn query(&self, query: &VariableQuery, now: u64) -> Result<VariablePage, StoreError> {
        let prefix = name_key(&query.prefix);
        let after = query.after.as_deref().map(name_key);
        let entries = self
            .lock()
            .iter()
            .filter(|((scope, subject, key), (_, stored))| {
                *scope == query.scope
                    && *subject == query.subject
                    && key.starts_with(&prefix)
                    && after.as_ref().is_none_or(|a| key > a)
                    && stored.live(now)
            })
            .take(query.limit + 1)
            .map(|(_, (name, stored))| VariableEntry {
                name: name.clone(),
                value: stored.value.clone(),
                ttl: Ttl::of(Some(stored), now),
            })
            .collect();
        Ok(VariablePage::from_entries(entries, query.limit))
    }

    fn transact(&self, body: &mut TransactionBody<'_>) -> Result<(), VariableError> {
        let mut guard = self.lock();
        let mut draft = guard.clone();
        body(&mut draft)?;
        *guard = draft;
        Ok(())
    }

    fn sweep_expired(&self, now: u64, limit: usize) -> Result<usize, StoreError> {
        let mut guard = self.lock();
        let dead: Vec<_> = guard
            .iter()
            .filter(|(_, (_, stored))| !stored.live(now))
            .take(limit)
            .map(|(key, _)| key.clone())
            .collect();
        for key in &dead {
            guard.remove(key);
        }
        Ok(dead.len())
    }
}

fn system_now() -> u64 {
    crate::cases::now_ms()
}

fn user_id(id: &str) -> Result<&str, VariableError> {
    if id.is_empty() || id != id.trim() || utf16_len(id) > NAME_MAX {
        return Err(VariableError::UserId);
    }
    Ok(id)
}

/// Deepest nesting a stored value may have; keeps it inside what the JSON
/// parser reads back.
pub const VALUE_DEPTH_MAX: usize = 64;

fn check_depth(value: &Value) -> Result<(), VariableError> {
    let mut stack = vec![(value, 1usize)];
    while let Some((value, depth)) = stack.pop() {
        let children: Box<dyn Iterator<Item = &Value>> = match value {
            Value::Array(items) => Box::new(items.iter()),
            Value::Object(map) => Box::new(map.values()),
            _ => continue,
        };
        if depth > VALUE_DEPTH_MAX {
            return Err(VariableError::Value);
        }
        stack.extend(children.map(|child| (child, depth + 1)));
    }
    Ok(())
}

fn expiry(ttl_ms: Option<u64>, now: u64) -> Result<Option<u64>, VariableError> {
    match ttl_ms {
        None => Ok(None),
        Some(ttl) => now
            .checked_add(ttl)
            .filter(|at| *at <= MAX_SAFE_MS)
            .map(Some)
            .ok_or(VariableError::Ttl),
    }
}

#[derive(Debug, Clone)]
struct Actor {
    guild_id: Option<String>,
    author_id: Option<String>,
}

impl Actor {
    fn subject(&self, scope: VariableScope, target: Option<&str>) -> Result<String, VariableError> {
        let guild = || {
            self.guild_id
                .clone()
                .filter(|g| !g.is_empty())
                .ok_or(VariableError::NotInGuild)
        };
        match scope {
            VariableScope::Global => Ok(String::new()),
            VariableScope::Guild => guild(),
            VariableScope::User => match target {
                Some(id) => Ok(id.to_owned()),
                None => self
                    .author_id
                    .clone()
                    .filter(|a| !a.is_empty())
                    .ok_or(VariableError::NoAuthor),
            },
        }
    }

    fn key(
        &self,
        scope: VariableScope,
        name: &str,
        target: Option<&str>,
    ) -> Result<VariableKey, VariableError> {
        VariableKey::new(scope, &self.subject(scope, target)?, name)
    }
}

/// Variables as seen by one command invocation: `guild` is the current
/// server, `user` is the author, `global` is shared by the whole bot.
#[derive(Clone)]
pub struct VariableService<S> {
    store: S,
    actor: Actor,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl<S: VariableStore> VariableService<S> {
    pub fn new(store: S, guild_id: Option<&str>, author_id: Option<&str>) -> Self {
        Self {
            store,
            actor: Actor {
                guild_id: guild_id.map(str::to_owned),
                author_id: author_id.map(str::to_owned),
            },
            clock: Arc::new(system_now),
        }
    }

    /// Replaces the clock (epoch milliseconds); meant for tests.
    pub fn with_clock(mut self, clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    /// Same store with `user` pointing at another member; needs a server.
    pub fn for_user(&self, id: &str) -> Result<Self, VariableError>
    where
        S: Clone,
    {
        self.actor.subject(VariableScope::Guild, None)?;
        let mut next = self.clone();
        next.actor.author_id = Some(user_id(id)?.to_owned());
        Ok(next)
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn get(&self, scope: VariableScope, name: &str) -> Result<Option<Value>, VariableError> {
        let key = self.actor.key(scope, name, None)?;
        Ok(self.read_live(&key)?.0.map(|v| v.value))
    }

    /// Reads an entry and the time it was live at. The clock is read again
    /// after the store answers: a value that expired while the read waited
    /// must not look live.
    fn read_live(&self, key: &VariableKey) -> Result<(Option<StoredVariable>, u64), VariableError> {
        let entry = self.store.read_entry(key, (self.clock)())?;
        let now = (self.clock)();
        Ok((entry.filter(|v| v.live(now)), now))
    }

    pub fn get_or(
        &self,
        scope: VariableScope,
        name: &str,
        fallback: Value,
    ) -> Result<Value, VariableError> {
        Ok(self.get(scope, name)?.unwrap_or(fallback))
    }

    pub fn query(
        &self,
        scope: VariableScope,
        options: VariableQueryOptions,
    ) -> Result<VariablePage, VariableError> {
        let prefix = options.prefix.unwrap_or_default();
        let limit = options.limit.unwrap_or(50);
        if utf16_len(&prefix) > NAME_MAX
            || options
                .after
                .as_deref()
                .is_some_and(|a| utf16_len(a) > NAME_MAX)
            || !(1..=100).contains(&limit)
        {
            return Err(VariableError::Query);
        }
        let query = VariableQuery {
            scope,
            subject: self.actor.subject(scope, None)?,
            prefix,
            after: options.after,
            limit,
        };
        let asked = (self.clock)();
        let mut page = self.store.query(&query, asked)?;
        // Same rule as `read_live`: drop what expired while the query waited.
        let waited = (self.clock)().saturating_sub(asked);
        if waited > 0 {
            page.entries.retain_mut(|entry| match &mut entry.ttl {
                Ttl::Remaining(left) if *left <= waited => false,
                Ttl::Remaining(left) => {
                    *left -= waited;
                    true
                }
                _ => true,
            });
        }
        Ok(page)
    }

    pub fn set(&self, scope: VariableScope, name: &str, value: Value) -> Result<(), VariableError> {
        self.transaction(|tx| tx.scope(scope).set(name, value.clone(), None))
    }

    /// Stores a value that expires `ttl_ms` milliseconds from now.
    pub fn set_with_ttl(
        &self,
        scope: VariableScope,
        name: &str,
        value: Value,
        ttl_ms: u64,
    ) -> Result<(), VariableError> {
        self.transaction(|tx| tx.scope(scope).set(name, value.clone(), Some(ttl_ms)))
    }

    pub fn delete(&self, scope: VariableScope, name: &str) -> Result<(), VariableError> {
        self.transaction(|tx| tx.scope(scope).delete(name))
    }

    /// Atomic read-modify-write; a missing value starts from `fallback` and
    /// an existing TTL is kept.
    pub fn update(
        &self,
        scope: VariableScope,
        name: &str,
        fallback: Value,
        mut updater: impl FnMut(Value) -> Result<Value, VariableError>,
    ) -> Result<Value, VariableError> {
        self.transaction(|tx| tx.scope(scope).update(name, fallback.clone(), &mut updater))
    }

    /// Adds `delta` to a numeric variable (missing counts as 0).
    pub fn increment(
        &self,
        scope: VariableScope,
        name: &str,
        delta: f64,
    ) -> Result<Value, VariableError> {
        self.transaction(|tx| tx.scope(scope).increment(name, delta, 0.0))
    }

    /// Sets (`Some`) or clears (`None`) the TTL; `false` when the variable is missing.
    pub fn expire(
        &self,
        scope: VariableScope,
        name: &str,
        ttl_ms: Option<u64>,
    ) -> Result<bool, VariableError> {
        self.transaction(|tx| tx.scope(scope).expire(name, ttl_ms))
    }

    pub fn ttl(&self, scope: VariableScope, name: &str) -> Result<Ttl, VariableError> {
        let key = self.actor.key(scope, name, None)?;
        let (entry, now) = self.read_live(&key)?;
        Ok(Ttl::of(entry.as_ref(), now))
    }

    /// Runs several operations atomically. Returning an error rolls every
    /// write back.
    ///
    /// Use only the transaction handle inside `body`: calling this service
    /// (or another one on the same store) from there waits on the lock the
    /// transaction already holds.
    pub fn transaction<T>(
        &self,
        mut body: impl FnMut(&mut VariableTransaction<'_>) -> Result<T, VariableError>,
    ) -> Result<T, VariableError> {
        let mut out = None;
        self.store.transact(&mut |access| {
            // Read the clock once the store lock is held: a value that
            // expired while waiting must not look live.
            let now = (self.clock)();
            let mut tx = VariableTransaction {
                access,
                actor: &self.actor,
                now,
            };
            out = Some(body(&mut tx)?);
            Ok(())
        })?;
        out.ok_or(VariableError::Store(StoreError::Backend(
            "the store did not run the transaction".into(),
        )))
    }

    /// Deletes up to `limit` expired variables (Yuzuha `sweepVariables`).
    pub fn sweep_expired(&self, limit: usize) -> Result<usize, VariableError> {
        Ok(self.store.sweep_expired((self.clock)(), limit)?)
    }
}

/// Open transaction handed to [`VariableService::transaction`].
pub struct VariableTransaction<'a> {
    access: &'a mut dyn VariableAccess,
    actor: &'a Actor,
    now: u64,
}

impl<'a> VariableTransaction<'a> {
    pub fn scope(&mut self, scope: VariableScope) -> ScopedVariables<'_, 'a> {
        ScopedVariables {
            tx: self,
            scope,
            target: None,
        }
    }

    pub fn guild(&mut self) -> ScopedVariables<'_, 'a> {
        self.scope(VariableScope::Guild)
    }

    pub fn user(&mut self) -> ScopedVariables<'_, 'a> {
        self.scope(VariableScope::User)
    }

    pub fn global(&mut self) -> ScopedVariables<'_, 'a> {
        self.scope(VariableScope::Global)
    }

    /// User scope of another member; needs a server.
    pub fn user_for(&mut self, id: &str) -> Result<ScopedVariables<'_, 'a>, VariableError> {
        self.actor.subject(VariableScope::Guild, None)?;
        let target = Some(user_id(id)?.to_owned());
        Ok(ScopedVariables {
            tx: self,
            scope: VariableScope::User,
            target,
        })
    }
}

/// One scope inside a transaction.
pub struct ScopedVariables<'t, 'a> {
    tx: &'t mut VariableTransaction<'a>,
    scope: VariableScope,
    target: Option<String>,
}

impl ScopedVariables<'_, '_> {
    fn key(&self, name: &str) -> Result<VariableKey, VariableError> {
        self.tx.actor.key(self.scope, name, self.target.as_deref())
    }

    /// Live entry; an expired one is deleted on sight.
    fn entry(&mut self, key: &VariableKey) -> Result<Option<StoredVariable>, VariableError> {
        match self.tx.access.get(key)? {
            Some(stored) if !stored.live(self.tx.now) => {
                self.tx.access.delete(key)?;
                Ok(None)
            }
            other => Ok(other),
        }
    }

    pub fn get(&mut self, name: &str) -> Result<Option<Value>, VariableError> {
        let key = self.key(name)?;
        Ok(self.entry(&key)?.map(|v| v.value))
    }

    pub fn set(
        &mut self,
        name: &str,
        value: Value,
        ttl_ms: Option<u64>,
    ) -> Result<(), VariableError> {
        let key = self.key(name)?;
        let at = expiry(ttl_ms, self.tx.now)?;
        check_depth(&value)?;
        Ok(self.tx.access.set(&key, &value, at)?)
    }

    pub fn delete(&mut self, name: &str) -> Result<(), VariableError> {
        let key = self.key(name)?;
        Ok(self.tx.access.delete(&key)?)
    }

    pub fn update(
        &mut self,
        name: &str,
        fallback: Value,
        updater: &mut dyn FnMut(Value) -> Result<Value, VariableError>,
    ) -> Result<Value, VariableError> {
        let key = self.key(name)?;
        let current = self.entry(&key)?;
        let at = current.as_ref().and_then(|c| c.expires_at);
        let value = updater(current.map(|c| c.value).unwrap_or(fallback))?;
        check_depth(&value)?;
        self.tx.access.set(&key, &value, at)?;
        Ok(value)
    }

    pub fn increment(
        &mut self,
        name: &str,
        delta: f64,
        fallback: f64,
    ) -> Result<Value, VariableError> {
        self.update(name, number(fallback)?, &mut |current| {
            let current = current.as_f64().ok_or_else(|| {
                VariableError::Operation("increment needs a numeric variable".into())
            })?;
            number(current + delta)
        })
    }

    pub fn expire(&mut self, name: &str, ttl_ms: Option<u64>) -> Result<bool, VariableError> {
        let at = expiry(ttl_ms, self.tx.now)?;
        let key = self.key(name)?;
        match self.entry(&key)? {
            None => Ok(false),
            Some(current) => {
                self.tx.access.set(&key, &current.value, at)?;
                Ok(true)
            }
        }
    }

    pub fn ttl(&mut self, name: &str) -> Result<Ttl, VariableError> {
        let key = self.key(name)?;
        let now = self.tx.now;
        Ok(Ttl::of(self.entry(&key)?.as_ref(), now))
    }
}

/// Whole numbers stay JSON integers so `1 + 1` stores `2`, not `2.0`.
fn number(value: f64) -> Result<Value, VariableError> {
    if !value.is_finite() {
        return Err(VariableError::Operation(
            "the increment result is not finite".into(),
        ));
    }
    if value.fract() == 0.0 && value.abs() <= MAX_SAFE_MS as f64 {
        return Ok(Value::from(value as i64));
    }
    Ok(Value::from(value))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum OperationKind {
    Get {
        #[serde(skip_serializing_if = "Option::is_none")]
        fallback: Option<Value>,
    },
    Set {
        value: Value,
        #[serde(rename = "ttlMs", skip_serializing_if = "Option::is_none")]
        ttl_ms: Option<u64>,
    },
    Delete,
    Ttl,
    Expire {
        #[serde(rename = "ttlMs")]
        ttl_ms: Option<u64>,
    },
    Increment {
        #[serde(skip_serializing_if = "Option::is_none")]
        delta: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        fallback: Option<f64>,
    },
}

/// One step of a data transaction, in Yuzuha's JSON shape:
/// `{"op":"set","scope":"guild","name":"n","value":1,"ttlMs":500}`.
///
/// Parsing is strict, as in Yuzuha's `dataOperations`: a field the operation
/// does not take, a missing `ttlMs` on `expire` and a `null` where a value is
/// expected are all errors.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VariableOperation {
    pub scope: VariableScope,
    pub name: String,
    #[serde(rename = "userID", skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(flatten)]
    pub kind: OperationKind,
}

impl<'de> Deserialize<'de> for VariableOperation {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        // A plain `Map` keeps the last of two equal keys; a repeated field is
        // an error here, so an operation cannot be redirected by one.
        struct Fields;
        impl<'de> serde::de::Visitor<'de> for Fields {
            type Value = serde_json::Map<String, Value>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a variable operation object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut access: A,
            ) -> Result<Self::Value, A::Error> {
                let mut map = serde_json::Map::new();
                while let Some((key, value)) = access.next_entry::<String, Value>()? {
                    if map.contains_key(&key) {
                        return Err(A::Error::custom(format!("the field `{key}` is repeated")));
                    }
                    map.insert(key, value);
                }
                Ok(map)
            }
        }
        let mut map = deserializer.deserialize_map(Fields)?;
        fn required<T: serde::de::DeserializeOwned, E: Error>(
            map: &mut serde_json::Map<String, Value>,
            field: &'static str,
        ) -> Result<T, E> {
            let value = map.remove(field).ok_or_else(|| E::missing_field(field))?;
            serde_json::from_value(value).map_err(|e| E::custom(format!("{field}: {e}")))
        }
        // Absent is `None`; an explicit `null` is rejected by `T`.
        fn optional<T: serde::de::DeserializeOwned, E: Error>(
            map: &mut serde_json::Map<String, Value>,
            field: &'static str,
        ) -> Result<Option<T>, E> {
            match map.remove(field) {
                None => Ok(None),
                Some(value) => serde_json::from_value(value)
                    .map(Some)
                    .map_err(|e| E::custom(format!("{field}: {e}"))),
            }
        }
        fn finite<E: Error>(value: Option<f64>, field: &str) -> Result<Option<f64>, E> {
            match value {
                Some(number) if !number.is_finite() => {
                    Err(E::custom(format!("{field} must be a finite number")))
                }
                other => Ok(other),
            }
        }

        let op: String = required(&mut map, "op")?;
        let scope = required(&mut map, "scope")?;
        let name = required(&mut map, "name")?;
        let user_id = optional(&mut map, "userID")?;
        let kind = match op.as_str() {
            "get" => OperationKind::Get {
                // Any JSON value is a fallback, `null` included.
                fallback: map.remove("fallback"),
            },
            "set" => OperationKind::Set {
                value: map
                    .remove("value")
                    .ok_or_else(|| D::Error::missing_field("value"))?,
                // Yuzuha takes `ttlMs: null` on `set` as "no expiry".
                ttl_ms: optional::<Option<u64>, _>(&mut map, "ttlMs")?.flatten(),
            },
            "delete" => OperationKind::Delete,
            "ttl" => OperationKind::Ttl,
            "expire" => OperationKind::Expire {
                ttl_ms: required(&mut map, "ttlMs")?,
            },
            "increment" => OperationKind::Increment {
                delta: finite(optional(&mut map, "delta")?, "delta")?,
                fallback: finite(optional(&mut map, "fallback")?, "fallback")?,
            },
            other => {
                return Err(D::Error::unknown_variant(
                    other,
                    &["get", "set", "delete", "ttl", "expire", "increment"],
                ))
            }
        };
        if let Some(field) = map.keys().next() {
            return Err(D::Error::custom(format!(
                "`{op}` does not take the field `{field}`"
            )));
        }
        Ok(Self {
            scope,
            name,
            user_id,
            kind,
        })
    }
}

/// Runs up to [`DATA_TRANSACTION_MAX`] operations and returns one JSON result
/// per operation (`null` for writes, `-1`/`-2` style numbers for `ttl`).
pub fn execute_operations(
    tx: &mut VariableTransaction<'_>,
    operations: &[VariableOperation],
) -> Result<Vec<Value>, VariableError> {
    if operations.is_empty() || operations.len() > DATA_TRANSACTION_MAX {
        return Err(VariableError::Operation(format!(
            "a transaction takes 1 to {DATA_TRANSACTION_MAX} operations"
        )));
    }
    let mut results = Vec::with_capacity(operations.len());
    for operation in operations {
        let mut scope = match &operation.user_id {
            Some(_) if operation.scope != VariableScope::User => {
                return Err(VariableError::Operation(
                    "userID needs the user scope".into(),
                ))
            }
            Some(id) => tx.user_for(id)?,
            None => tx.scope(operation.scope),
        };
        let name = operation.name.as_str();
        results.push(match &operation.kind {
            OperationKind::Get { fallback } => scope
                .get(name)?
                .or_else(|| fallback.clone())
                .unwrap_or(Value::Null),
            OperationKind::Set { value, ttl_ms } => {
                scope.set(name, value.clone(), *ttl_ms)?;
                Value::Null
            }
            OperationKind::Delete => {
                scope.delete(name)?;
                Value::Null
            }
            OperationKind::Ttl => Value::from(scope.ttl(name)?.as_millis()),
            OperationKind::Expire { ttl_ms } => Value::Bool(scope.expire(name, *ttl_ms)?),
            OperationKind::Increment { delta, fallback } => {
                scope.increment(name, delta.unwrap_or(1.0), fallback.unwrap_or(0.0))?
            }
        });
    }
    Ok(results)
}
