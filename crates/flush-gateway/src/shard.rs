//! Sharding: guild → shard routing, validated shard plans and a
//! [`ShardManager`] that runs one [`GatewayClient`] per shard in this process.
//!
//! Mirrors the shard handling of `@fluxerjs/ws` (`shardIds` / `shardCount`)
//! and Yuzuha's `shardIdForGuild`.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::future::{select, Either};
use futures::StreamExt;
use log::{error, info, warn};
use thiserror::Error;

use flush_core::events::Event;
use flush_core::presence::Presence;
use flush_core::Snowflake;

use crate::client::{ClientConfig, ConnectError, GatewayClient};

/// Upper bound on the shard count, as in Yuzuha.
pub const MAX_SHARD_COUNT: u32 = 16_384;

/// Guilds per shard used to size an automatic shard count.
pub const DEFAULT_GUILDS_PER_SHARD: u64 = 1_500;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ShardError {
    #[error("shard count must be an integer between 1 and {MAX_SHARD_COUNT}")]
    InvalidCount,
    #[error("shard ids must be a non-empty list")]
    EmptyIds,
    #[error("shard id {0} is outside the shard range")]
    IdOutOfRange(u32),
    #[error("shard id {0} is listed more than once")]
    DuplicateId(u32),
    #[error("guild id must be a numeric snowflake")]
    InvalidGuildId,
    #[error("shard {0} is not managed here")]
    UnknownShard(u32),
    #[error("{0}")]
    Invalid(String),
}

/// Shard that owns a guild, by the Fluxer gateway formula
/// `(guild_id >> 22) % count`.
pub fn shard_id_for_guild(guild_id: Snowflake, count: u32) -> Result<u32, ShardError> {
    if count == 0 || count > MAX_SHARD_COUNT {
        return Err(ShardError::InvalidCount);
    }
    Ok(((guild_id.get() >> 22) % u64::from(count)) as u32)
}

/// Shard count for `guild_count` guilds at `guilds_per_shard` (0 = default),
/// clamped to `1..=MAX_SHARD_COUNT`.
pub fn recommended_shard_count(guild_count: u64, guilds_per_shard: u64) -> u32 {
    let per = if guilds_per_shard == 0 {
        DEFAULT_GUILDS_PER_SHARD
    } else {
        guilds_per_shard
    };
    guild_count
        .div_ceil(per)
        .clamp(1, u64::from(MAX_SHARD_COUNT)) as u32
}

/// Validated set of shard ids this process runs, out of `count` total.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardPlan {
    ids: Vec<u32>,
    count: u32,
}

impl ShardPlan {
    /// Every shard `0..count`.
    pub fn all(count: u32) -> Result<Self, ShardError> {
        if count == 0 || count > MAX_SHARD_COUNT {
            return Err(ShardError::InvalidCount);
        }
        Ok(Self {
            ids: (0..count).collect(),
            count,
        })
    }

    /// The listed shards out of `count`. Order is kept; duplicates and ids
    /// outside `0..count` are rejected.
    pub fn new(ids: &[u32], count: u32) -> Result<Self, ShardError> {
        if count == 0 || count > MAX_SHARD_COUNT {
            return Err(ShardError::InvalidCount);
        }
        if ids.is_empty() {
            return Err(ShardError::EmptyIds);
        }
        let mut seen = HashSet::new();
        for &id in ids {
            if id >= count {
                return Err(ShardError::IdOutOfRange(id));
            }
            if !seen.insert(id) {
                return Err(ShardError::DuplicateId(id));
            }
        }
        Ok(Self {
            ids: ids.to_vec(),
            count,
        })
    }

    /// The listed shards, with the count inferred as `max(id) + 1`.
    pub fn from_ids(ids: &[u32]) -> Result<Self, ShardError> {
        let max = ids.iter().copied().max().ok_or(ShardError::EmptyIds)?;
        if max >= MAX_SHARD_COUNT {
            return Err(ShardError::IdOutOfRange(max));
        }
        Self::new(ids, max + 1)
    }

    pub fn ids(&self) -> &[u32] {
        &self.ids
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn contains(&self, id: u32) -> bool {
        self.ids.contains(&id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShardStatus {
    /// Waiting for its identify slot.
    Idle,
    Connecting,
    Ready,
    Reconnecting,
    /// Stopped by a fatal error or by [`ShardManager::shutdown`].
    Closed,
}

impl ShardStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Connecting => "connecting",
            Self::Ready => "ready",
            Self::Reconnecting => "reconnecting",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShardInfo {
    pub id: u32,
    pub status: ShardStatus,
}

#[derive(Debug)]
pub enum ShardEvent {
    Dispatch {
        shard_id: u32,
        event: Box<Event>,
    },
    StatusChanged {
        shard_id: u32,
        status: ShardStatus,
    },
    /// The shard stopped for good (bad token, invalid shard, API version).
    Fatal {
        shard_id: u32,
        error: ConnectError,
    },
}

#[derive(Debug, Clone)]
pub struct ShardManagerOptions {
    /// Gap between the first Identify of consecutive shards (default 5s).
    pub identify_delay: Duration,
    /// Wait before retrying a shard whose connection attempt failed.
    pub retry_delay: Duration,
}

impl Default for ShardManagerOptions {
    fn default() -> Self {
        Self {
            identify_delay: Duration::from_secs(5),
            retry_delay: Duration::from_secs(5),
        }
    }
}

enum ShardCommand {
    Presence(Presence),
    RequestMembers {
        guild_id: Snowflake,
        query: String,
        limit: u32,
    },
    Reconnect,
}

type Statuses = Arc<Mutex<BTreeMap<u32, ShardStatus>>>;

/// Serializes first Identifies: held from a shard's connection attempt until
/// it is ready, and remembers when the previous holder let go.
type IdentifyGate = Arc<futures::lock::Mutex<Option<Instant>>>;

struct ShardHandle {
    commands: UnboundedSender<ShardCommand>,
    task: Option<async_std::task::JoinHandle<()>>,
}

/// Runs one gateway connection per shard and merges their events.
pub struct ShardManager {
    plan: ShardPlan,
    shards: BTreeMap<u32, ShardHandle>,
    statuses: Statuses,
    events: UnboundedReceiver<ShardEvent>,
    pending: VecDeque<ShardEvent>,
}

impl ShardManager {
    /// Starts every shard in `plan`. `config.shard` is overwritten per shard.
    pub fn spawn(
        config: ClientConfig,
        plan: ShardPlan,
        options: ShardManagerOptions,
    ) -> Result<Self, ConnectError> {
        let (tx, events) = unbounded();
        let statuses: Statuses = Arc::new(Mutex::new(BTreeMap::new()));
        let mut shards = BTreeMap::new();
        let gate: IdentifyGate = Arc::new(futures::lock::Mutex::new(None));
        for &id in plan.ids() {
            let mut cfg = config.clone();
            cfg.shard = Some((id, plan.count()));
            let client = GatewayClient::new(cfg)?;
            let (cmd_tx, cmd_rx) = unbounded();
            set_status(&statuses, &tx, id, ShardStatus::Idle);
            let task = async_std::task::spawn(run_shard(
                id,
                client,
                cmd_rx,
                tx.clone(),
                statuses.clone(),
                gate.clone(),
                options.identify_delay,
                options.retry_delay,
            ));
            shards.insert(
                id,
                ShardHandle {
                    commands: cmd_tx,
                    task: Some(task),
                },
            );
        }
        Ok(Self {
            plan,
            shards,
            statuses,
            events,
            pending: VecDeque::new(),
        })
    }

    pub fn plan(&self) -> &ShardPlan {
        &self.plan
    }

    /// Next event from any shard; `None` once every shard has stopped.
    pub async fn next(&mut self) -> Option<ShardEvent> {
        if let Some(event) = self.pending.pop_front() {
            return Some(event);
        }
        self.events.next().await
    }

    pub fn statuses(&self) -> Vec<ShardInfo> {
        let map = self.statuses.lock().unwrap_or_else(|e| e.into_inner());
        map.iter()
            .map(|(&id, &status)| ShardInfo { id, status })
            .collect()
    }

    pub fn status(&self, shard_id: u32) -> Option<ShardStatus> {
        let map = self.statuses.lock().unwrap_or_else(|e| e.into_inner());
        map.get(&shard_id).copied()
    }

    pub fn shard_id_for_guild(&self, guild_id: Snowflake) -> u32 {
        ((guild_id.get() >> 22) % u64::from(self.plan.count())) as u32
    }

    /// Sends a Presence Update on every shard.
    pub fn set_presence(&self, presence: &Presence) {
        for shard in self.shards.values() {
            let _ = shard
                .commands
                .unbounded_send(ShardCommand::Presence(presence.clone()));
        }
    }

    /// Requests member chunks on the shard that owns `guild_id`.
    pub fn request_guild_members(
        &self,
        guild_id: Snowflake,
        query: &str,
        limit: u32,
    ) -> Result<(), ShardError> {
        let id = self.shard_id_for_guild(guild_id);
        self.send(
            id,
            ShardCommand::RequestMembers {
                guild_id,
                query: query.to_owned(),
                limit,
            },
        )
    }

    /// Drops one shard's socket; it reconnects and resumes its session.
    pub fn reconnect(&self, shard_id: u32) -> Result<(), ShardError> {
        self.send(shard_id, ShardCommand::Reconnect)
    }

    fn send(&self, shard_id: u32, command: ShardCommand) -> Result<(), ShardError> {
        let shard = self
            .shards
            .get(&shard_id)
            .ok_or(ShardError::UnknownShard(shard_id))?;
        shard
            .commands
            .unbounded_send(command)
            .map_err(|_| ShardError::UnknownShard(shard_id))
    }

    /// Stops every shard and waits for their tasks to end.
    pub async fn shutdown(&mut self) {
        for (&id, shard) in self.shards.iter_mut() {
            if let Some(task) = shard.task.take() {
                task.cancel().await;
            }
            let mut map = self.statuses.lock().unwrap_or_else(|e| e.into_inner());
            if map.insert(id, ShardStatus::Closed) != Some(ShardStatus::Closed) {
                self.pending.push_back(ShardEvent::StatusChanged {
                    shard_id: id,
                    status: ShardStatus::Closed,
                });
            }
        }
    }
}

impl Drop for ShardManager {
    fn drop(&mut self) {
        for shard in self.shards.values_mut() {
            if let Some(task) = shard.task.take() {
                async_std::task::spawn(task.cancel());
            }
        }
    }
}

fn set_status(
    statuses: &Statuses,
    events: &UnboundedSender<ShardEvent>,
    shard_id: u32,
    status: ShardStatus,
) {
    let previous = {
        let mut map = statuses.lock().unwrap_or_else(|e| e.into_inner());
        map.insert(shard_id, status)
    };
    if previous != Some(status) {
        let _ = events.unbounded_send(ShardEvent::StatusChanged { shard_id, status });
    }
}

fn release(slot: &mut Option<futures::lock::OwnedMutexGuard<Option<Instant>>>) {
    if let Some(mut guard) = slot.take() {
        *guard = Some(Instant::now());
    }
}

fn retryable(error: &ConnectError) -> bool {
    matches!(error, ConnectError::Ws(_) | ConnectError::BadHello(_))
}

#[allow(clippy::too_many_arguments)]
async fn run_shard(
    id: u32,
    mut client: GatewayClient,
    mut commands: UnboundedReceiver<ShardCommand>,
    events: UnboundedSender<ShardEvent>,
    statuses: Statuses,
    gate: IdentifyGate,
    identify_delay: Duration,
    retry_delay: Duration,
) {
    // Held until this shard's first Identify is answered (or fails), so the
    // next shard identifies at least `identify_delay` after that.
    let mut slot = Some(gate.lock_owned().await);
    if let Some(Some(last)) = slot.as_deref() {
        let wait = identify_delay.saturating_sub(last.elapsed());
        if !wait.is_zero() {
            async_std::task::sleep(wait).await;
        }
    }
    set_status(&statuses, &events, id, ShardStatus::Connecting);
    let mut commands_open = true;
    loop {
        let current = statuses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .copied();
        if current == Some(ShardStatus::Ready) && !client.is_connected() {
            set_status(&statuses, &events, id, ShardStatus::Reconnecting);
        }
        // Commands wait for a live session: sending on a missing socket is a
        // silent no-op, and racing them against the handshake would cancel it.
        let outcome = if commands_open && client.is_connected() && client.is_ready() {
            let step = Box::pin(client.next_event());
            match select(step, commands.next()).await {
                Either::Left((result, _)) => Either::Left(result),
                Either::Right((command, _)) => Either::Right(command),
            }
        } else {
            Either::Left(client.next_event().await)
        };
        match outcome {
            Either::Left(Ok(Some(event))) => {
                match &event {
                    Event::Ready(_) | Event::Resumed { .. } => {
                        info!("gateway: shard {id} ready");
                        set_status(&statuses, &events, id, ShardStatus::Ready);
                        release(&mut slot);
                    }
                    _ => {}
                }
                let message = ShardEvent::Dispatch {
                    shard_id: id,
                    event: Box::new(event),
                };
                if events.unbounded_send(message).is_err() {
                    break;
                }
            }
            Either::Left(Ok(None)) => {}
            Either::Left(Err(error)) if retryable(&error) => {
                warn!("gateway: shard {id} dropped ({error}), retrying in {retry_delay:?}");
                set_status(&statuses, &events, id, ShardStatus::Reconnecting);
                client.disconnect();
                release(&mut slot);
                async_std::task::sleep(retry_delay).await;
            }
            Either::Left(Err(error)) => {
                error!("gateway: shard {id} stopped: {error}");
                set_status(&statuses, &events, id, ShardStatus::Closed);
                let _ = events.unbounded_send(ShardEvent::Fatal {
                    shard_id: id,
                    error,
                });
                break;
            }
            Either::Right(Some(command)) => {
                let sent = match command {
                    ShardCommand::Presence(presence) => client.set_presence(&presence).await,
                    ShardCommand::RequestMembers {
                        guild_id,
                        query,
                        limit,
                    } => client.request_guild_members(guild_id, &query, limit).await,
                    ShardCommand::Reconnect => {
                        set_status(&statuses, &events, id, ShardStatus::Reconnecting);
                        client.disconnect();
                        Ok(())
                    }
                };
                if let Err(error) = sent {
                    warn!("gateway: shard {id} command failed: {error}");
                    client.disconnect();
                }
            }
            Either::Right(None) => commands_open = false,
        }
    }
}
