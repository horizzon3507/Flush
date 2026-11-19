//! Sharding for [`Bot`](crate::Bot): options, process environment and the
//! [`BotShard`] view handlers use to route by guild and gate background jobs.
//!
//! Mirrors Yuzuha's `sharding` option (`leaderShardID`, `schedulerMode`,
//! `ids`, `count`) and the `FLUXER_SHARD_*` variables a supervisor sets on
//! each shard process.

use std::time::Duration;

use flush_core::Snowflake;
use flush_gateway::{ShardError, ShardInfo, ShardPlan, ShardStatus, MAX_SHARD_COUNT};

/// Which shard processes run background jobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShardSchedulerMode {
    /// Only the process that owns the leader shard.
    #[default]
    Leader,
    /// Every process.
    All,
    /// None.
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShardCount {
    Fixed(u32),
    /// Sized from the bot's guild count at startup.
    Auto,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardingOptions {
    /// Shards this process runs. `None` = environment, else every shard.
    pub ids: Option<Vec<u32>>,
    /// Total shards. `None` = environment, else inferred from `ids`.
    pub count: Option<ShardCount>,
    /// Shard whose process runs leader-only jobs (default 0).
    pub leader_shard_id: u32,
    pub scheduler_mode: ShardSchedulerMode,
    /// Guilds per shard for [`ShardCount::Auto`] (0 = 1500).
    pub guilds_per_shard: u64,
    /// Gap between the Identify of consecutive shards.
    pub identify_delay: Duration,
    /// Read `FLUXER_SHARD_IDS` / `FLUXER_SHARD_COUNT` when `ids` and `count`
    /// are unset.
    pub use_env: bool,
}

impl Default for ShardingOptions {
    fn default() -> Self {
        Self {
            ids: None,
            count: None,
            leader_shard_id: 0,
            scheduler_mode: ShardSchedulerMode::Leader,
            guilds_per_shard: 0,
            identify_delay: Duration::from_secs(5),
            use_env: true,
        }
    }
}

impl ShardingOptions {
    /// Every shard of a fixed total, in this process.
    pub fn fixed(count: u32) -> Self {
        Self {
            count: Some(ShardCount::Fixed(count)),
            ..Self::default()
        }
    }

    /// Every shard, with the total sized from the guild count.
    pub fn auto() -> Self {
        Self {
            count: Some(ShardCount::Auto),
            ..Self::default()
        }
    }

    /// A subset of shards out of `count`, for one process of many.
    pub fn subset(ids: impl Into<Vec<u32>>, count: u32) -> Self {
        Self {
            ids: Some(ids.into()),
            count: Some(ShardCount::Fixed(count)),
            ..Self::default()
        }
    }

    pub fn leader_shard_id(mut self, id: u32) -> Self {
        self.leader_shard_id = id;
        self
    }

    pub fn scheduler_mode(mut self, mode: ShardSchedulerMode) -> Self {
        self.scheduler_mode = mode;
        self
    }

    pub fn validate(&self) -> Result<(), ShardError> {
        if self.leader_shard_id >= MAX_SHARD_COUNT {
            return Err(ShardError::Invalid(format!(
                "leader_shard_id must be between 0 and {}",
                MAX_SHARD_COUNT - 1
            )));
        }
        if let Some(ShardCount::Fixed(count)) = self.count {
            if count == 0 || count > MAX_SHARD_COUNT {
                return Err(ShardError::InvalidCount);
            }
        }
        if matches!(&self.ids, Some(ids) if ids.is_empty()) {
            return Err(ShardError::EmptyIds);
        }
        Ok(())
    }

    /// Whether the plan needs the guild count ([`ShardCount::Auto`]).
    pub fn needs_guild_count(&self) -> bool {
        self.count == Some(ShardCount::Auto)
    }

    /// Resolves the shards to run. Explicit `ids` / `count` win over `env`,
    /// which wins over the single-shard default. `guild_count` sizes
    /// [`ShardCount::Auto`].
    pub fn resolve(
        &self,
        env: Option<&ShardEnv>,
        guild_count: Option<u64>,
    ) -> Result<ShardPlan, ShardError> {
        self.validate()?;
        let count = match self.count {
            Some(ShardCount::Fixed(count)) => Some(count),
            Some(ShardCount::Auto) => Some(flush_gateway::recommended_shard_count(
                guild_count.unwrap_or(0),
                self.guilds_per_shard,
            )),
            None => None,
        };
        match (&self.ids, count) {
            (Some(ids), Some(count)) => ShardPlan::new(ids, count),
            (Some(ids), None) => ShardPlan::from_ids(ids),
            (None, Some(count)) => ShardPlan::all(count),
            (None, None) => match env {
                Some(env) => env.plan(),
                None => ShardPlan::all(1),
            },
        }
    }
}

/// Shard assignment from the process environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardEnv {
    pub ids: Vec<u32>,
    pub count: u32,
    /// `FLUXER_SHARD_PROCESS_ID`, defaulting to the first shard id.
    pub process_id: u32,
}

impl ShardEnv {
    pub const IDS: &'static str = "FLUXER_SHARD_IDS";
    pub const COUNT: &'static str = "FLUXER_SHARD_COUNT";
    pub const PROCESS_ID: &'static str = "FLUXER_SHARD_PROCESS_ID";

    /// Reads the process environment. `Ok(None)` when no variable is set.
    pub fn from_env() -> Result<Option<Self>, ShardError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Same as [`ShardEnv::from_env`] over any key lookup.
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Option<Self>, ShardError> {
        let get = |key: &str| {
            lookup(key)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        let (raw_ids, raw_count, raw_process) =
            (get(Self::IDS), get(Self::COUNT), get(Self::PROCESS_ID));
        if raw_ids.is_none() && raw_count.is_none() && raw_process.is_none() {
            return Ok(None);
        }
        let count = match raw_count {
            Some(raw) => {
                let count = parse_digits(&raw).ok_or(ShardError::InvalidCount)?;
                if count == 0 || count > MAX_SHARD_COUNT {
                    return Err(ShardError::InvalidCount);
                }
                Some(count)
            }
            None => None,
        };
        let ids = match raw_ids {
            Some(raw) => raw
                .split(',')
                .map(|part| parse_digits(part.trim()))
                .collect::<Option<Vec<u32>>>()
                .ok_or_else(|| {
                    ShardError::Invalid(format!("{} must hold comma-separated integers", Self::IDS))
                })?,
            None => vec![0],
        };
        let plan = match count {
            Some(count) => ShardPlan::new(&ids, count)?,
            None => ShardPlan::from_ids(&ids)?,
        };
        let process_id = match raw_process {
            Some(raw) => parse_digits(&raw)
                .filter(|id| *id < MAX_SHARD_COUNT)
                .ok_or_else(|| {
                    ShardError::Invalid(format!(
                        "{} must be a non-negative integer below {MAX_SHARD_COUNT}",
                        Self::PROCESS_ID
                    ))
                })?,
            None => plan.ids()[0],
        };
        Ok(Some(Self {
            ids: plan.ids().to_vec(),
            count: plan.count(),
            process_id,
        }))
    }

    pub fn plan(&self) -> Result<ShardPlan, ShardError> {
        ShardPlan::new(&self.ids, self.count)
    }
}

fn parse_digits(raw: &str) -> Option<u32> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    raw.parse().ok()
}

/// The shards this process runs, as seen by commands, hooks and jobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotShard {
    plan: ShardPlan,
    leader_shard_id: u32,
    scheduler_mode: ShardSchedulerMode,
    statuses: Vec<ShardInfo>,
}

impl BotShard {
    pub fn new(plan: ShardPlan, leader_shard_id: u32, scheduler_mode: ShardSchedulerMode) -> Self {
        let statuses = plan
            .ids()
            .iter()
            .map(|&id| ShardInfo {
                id,
                status: ShardStatus::Idle,
            })
            .collect();
        Self {
            plan,
            leader_shard_id,
            scheduler_mode,
            statuses,
        }
    }

    /// Unsharded bot: shard 0 of 1.
    pub fn single() -> Self {
        let plan = ShardPlan::all(1).expect("one shard is a valid plan");
        Self::new(plan, 0, ShardSchedulerMode::Leader)
    }

    /// First shard id of this process.
    pub fn id(&self) -> u32 {
        self.plan.ids()[0]
    }

    pub fn ids(&self) -> &[u32] {
        self.plan.ids()
    }

    pub fn count(&self) -> u32 {
        self.plan.count()
    }

    pub fn is_leader(&self) -> bool {
        self.plan.contains(self.leader_shard_id)
    }

    pub fn scheduler_mode(&self) -> ShardSchedulerMode {
        self.scheduler_mode
    }

    pub fn shard_id_for_guild(&self, guild_id: Snowflake) -> u32 {
        ((guild_id.get() >> 22) % u64::from(self.count())) as u32
    }

    /// Whether this process holds the gateway connection for `guild_id`.
    pub fn owns_guild(&self, guild_id: Snowflake) -> bool {
        self.plan.contains(self.shard_id_for_guild(guild_id))
    }

    /// Whether background jobs run in this process.
    pub fn should_start_jobs(&self) -> bool {
        should_start_sharded_jobs(Some(self), self.scheduler_mode)
    }

    /// Connection status of each local shard.
    pub fn statuses(&self) -> &[ShardInfo] {
        &self.statuses
    }

    pub fn status(&self, shard_id: u32) -> Option<ShardStatus> {
        self.statuses
            .iter()
            .find(|s| s.id == shard_id)
            .map(|s| s.status)
    }

    pub(crate) fn set_status(&mut self, shard_id: u32, status: ShardStatus) {
        if let Some(entry) = self.statuses.iter_mut().find(|s| s.id == shard_id) {
            entry.status = status;
        }
    }

    /// Runs `f` once per local shard and collects the results in shard order.
    /// The shards of one process share one bot, so this covers this process
    /// only.
    pub fn broadcast_eval<T>(&self, f: impl FnMut(&ShardInfo) -> T) -> Vec<T> {
        self.statuses.iter().map(f).collect()
    }
}

/// Yuzuha's `shouldStartShardedJobs`: jobs run unless disabled, and on a
/// sharded bot in `Leader` mode only where the leader shard lives.
pub fn should_start_sharded_jobs(shard: Option<&BotShard>, mode: ShardSchedulerMode) -> bool {
    match (mode, shard) {
        (ShardSchedulerMode::Disabled, _) => false,
        (_, None) => true,
        (_, Some(shard)) if shard.count() <= 1 => true,
        (ShardSchedulerMode::All, _) => true,
        (ShardSchedulerMode::Leader, Some(shard)) => shard.is_leader(),
    }
}
