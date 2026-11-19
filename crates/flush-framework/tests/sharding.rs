use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use flush_core::Snowflake;
use flush_framework::{
    should_start_sharded_jobs, BotShard, Cluster, ClusterError, ClusterEvent, CommandError,
    RuntimeFuture, RuntimeStatus, Scheduler, ShardEnv, ShardSchedulerMode, ShardingOptions,
};
use flush_gateway::{ShardError, ShardPlan};

fn env(pairs: &[(&str, &str)]) -> Result<Option<ShardEnv>, ShardError> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    ShardEnv::from_lookup(|key| map.get(key).cloned())
}

#[test]
fn env_assigns_shards_to_the_process() {
    assert_eq!(env(&[]), Ok(None));
    assert_eq!(env(&[("FLUXER_SHARD_IDS", "  ")]), Ok(None));
    let parsed = env(&[("FLUXER_SHARD_IDS", "2, 3"), ("FLUXER_SHARD_COUNT", "8")])
        .unwrap()
        .unwrap();
    assert_eq!(
        (parsed.ids.as_slice(), parsed.count, parsed.process_id),
        (&[2, 3][..], 8, 2)
    );
    let inferred = env(&[("FLUXER_SHARD_IDS", "0,4")]).unwrap().unwrap();
    assert_eq!(inferred.count, 5);
    let count_only = env(&[
        ("FLUXER_SHARD_COUNT", "4"),
        ("FLUXER_SHARD_PROCESS_ID", "9"),
    ])
    .unwrap()
    .unwrap();
    assert_eq!(
        (count_only.ids.as_slice(), count_only.process_id),
        (&[0][..], 9)
    );
}

#[test]
fn env_rejects_malformed_values() {
    assert!(env(&[("FLUXER_SHARD_IDS", "1,x")]).is_err());
    assert!(env(&[("FLUXER_SHARD_IDS", "-1")]).is_err());
    assert_eq!(
        env(&[("FLUXER_SHARD_COUNT", "0")]),
        Err(ShardError::InvalidCount)
    );
    assert_eq!(
        env(&[("FLUXER_SHARD_COUNT", "99999")]),
        Err(ShardError::InvalidCount)
    );
    assert_eq!(
        env(&[("FLUXER_SHARD_IDS", "4"), ("FLUXER_SHARD_COUNT", "4")]),
        Err(ShardError::IdOutOfRange(4))
    );
    assert!(env(&[("FLUXER_SHARD_PROCESS_ID", "abc")]).is_err());
}

#[test]
fn options_resolve_explicit_then_env_then_single() {
    let from_env = env(&[("FLUXER_SHARD_IDS", "1"), ("FLUXER_SHARD_COUNT", "2")])
        .unwrap()
        .unwrap();
    let default = ShardingOptions::default();
    assert_eq!(default.resolve(None, None), ShardPlan::all(1));
    assert_eq!(
        default.resolve(Some(&from_env), None),
        ShardPlan::new(&[1], 2)
    );
    assert_eq!(
        ShardingOptions::fixed(3).resolve(Some(&from_env), None),
        ShardPlan::all(3)
    );
    assert_eq!(
        ShardingOptions::subset([1, 2], 4).resolve(None, None),
        ShardPlan::new(&[1, 2], 4)
    );
    let auto = ShardingOptions::auto();
    assert!(auto.needs_guild_count());
    assert_eq!(auto.resolve(None, Some(3_001)), ShardPlan::all(3));
    assert_eq!(auto.resolve(None, None), ShardPlan::all(1));
    assert_eq!(
        ShardingOptions::fixed(0).resolve(None, None),
        Err(ShardError::InvalidCount)
    );
    assert!(ShardingOptions::fixed(2)
        .leader_shard_id(2)
        .resolve(None, None)
        .is_err());
    assert!(ShardingOptions::fixed(2)
        .leader_shard_id(20_000)
        .validate()
        .is_err());
}

#[test]
fn bot_shard_routes_guilds_and_gates_jobs() {
    let single = BotShard::single();
    assert_eq!(
        (single.id(), single.count(), single.is_leader()),
        (0, 1, true)
    );
    assert!(single.should_start_jobs());

    let plan = ShardPlan::new(&[1], 2).unwrap();
    let follower = BotShard::new(plan.clone(), 0, ShardSchedulerMode::Leader);
    assert!(!follower.is_leader());
    assert!(!follower.should_start_jobs());
    assert!(should_start_sharded_jobs(
        Some(&follower),
        ShardSchedulerMode::All
    ));
    assert!(!should_start_sharded_jobs(
        Some(&follower),
        ShardSchedulerMode::Disabled
    ));
    assert!(!should_start_sharded_jobs(
        None,
        ShardSchedulerMode::Disabled
    ));
    assert!(should_start_sharded_jobs(None, ShardSchedulerMode::Leader));
    assert!(BotShard::new(plan, 1, ShardSchedulerMode::Leader).should_start_jobs());
    // A disabled scheduler stays off even on an unsharded bot.
    let off = BotShard::new(ShardPlan::all(1).unwrap(), 0, ShardSchedulerMode::Disabled);
    assert!(!off.should_start_jobs());

    assert_eq!(follower.shard_id_for_guild(Snowflake::new(3 << 22)), 1);
    assert!(follower.owns_guild(Snowflake::new(3 << 22)));
    assert!(!follower.owns_guild(Snowflake::new(2 << 22)));
    assert_eq!(follower.broadcast_eval(|shard| shard.id), vec![1]);
}

#[async_std::test]
async fn paused_scheduler_skips_jobs() {
    let scheduler = Scheduler::new();
    let count = Arc::new(AtomicU32::new(0));
    let c = count.clone();
    scheduler.set_paused(true);
    let handle = scheduler.every(Duration::from_millis(10), move || {
        let c = c.clone();
        async move {
            c.fetch_add(1, Ordering::SeqCst);
        }
    });
    async_std::task::sleep(Duration::from_millis(60)).await;
    assert_eq!(count.load(Ordering::SeqCst), 0);
    scheduler.set_paused(false);
    async_std::task::sleep(Duration::from_millis(60)).await;
    handle.cancel();
    assert!(count.load(Ordering::SeqCst) >= 1);
}

fn forever(starts: Arc<AtomicU32>) -> impl Fn() -> Result<RuntimeFuture, CommandError> {
    move || {
        starts.fetch_add(1, Ordering::SeqCst);
        Ok(Box::pin(futures::future::pending()) as RuntimeFuture)
    }
}

#[async_std::test]
async fn cluster_adds_restarts_and_removes_runtimes() {
    let mut cluster: Cluster<&'static str> = Cluster::new();
    let starts = Arc::new(AtomicU32::new(0));
    cluster
        .add_runner("main", "primary", forever(starts.clone()))
        .unwrap();
    assert_eq!(
        cluster.add_runner("main", "again", forever(starts.clone())),
        Err(ClusterError::Duplicate("main".into()))
    );
    assert_eq!(
        cluster.add_runner(" ", "blank", forever(starts.clone())),
        Err(ClusterError::EmptyId)
    );
    let entry = cluster.get("main").unwrap();
    assert_eq!(
        (entry.status, entry.meta),
        (RuntimeStatus::Running, "primary")
    );
    assert_eq!(cluster.info().size, 1);

    cluster.restart("main").await.unwrap();
    assert_eq!(starts.load(Ordering::SeqCst), 2);
    assert_eq!(cluster.get("main").unwrap().meta, "primary");
    assert_eq!(
        cluster.restart("ghost").await,
        Err(ClusterError::Unknown("ghost".into()))
    );

    assert!(cluster.remove("main").await);
    assert!(!cluster.remove("main").await);
    assert!(cluster.is_empty());
    let mut events = Vec::new();
    while let Some(event) = cluster.try_next_event() {
        events.push(event);
    }
    let id = || "main".to_string();
    assert_eq!(
        events,
        vec![
            ClusterEvent::RuntimeAdded { id: id() },
            ClusterEvent::RuntimeRemoved { id: id() },
            ClusterEvent::RuntimeAdded { id: id() },
            ClusterEvent::RuntimeRemoved { id: id() },
        ]
    );
}

#[async_std::test]
async fn cluster_reports_failures_and_start_errors() {
    let mut cluster: Cluster = Cluster::new();
    cluster
        .add_runner("crashy", (), || {
            Ok(
                Box::pin(async { Err(CommandError::Internal("gateway down".into())) })
                    as RuntimeFuture,
            )
        })
        .unwrap();
    cluster
        .add_runner("done", (), || {
            Ok(Box::pin(async { Ok(()) }) as RuntimeFuture)
        })
        .unwrap();
    let started = cluster.add_runner("broken", (), || Err(CommandError::User("no token".into())));
    assert!(matches!(started, Err(ClusterError::Start { .. })));
    assert!(!cluster.has("broken"));

    let mut seen_error = false;
    let mut seen_stop = false;
    while !(seen_error && seen_stop) {
        let event = async_std::future::timeout(Duration::from_secs(5), cluster.next_event())
            .await
            .unwrap()
            .unwrap();
        match event {
            ClusterEvent::RuntimeError { id, message } => {
                assert_eq!(id, "crashy");
                assert!(message.contains("gateway down"));
                seen_error = true;
            }
            ClusterEvent::RuntimeStopped { id } => {
                assert_eq!(id, "done");
                seen_stop = true;
            }
            _ => {}
        }
    }
    let crashy = cluster.get("crashy").unwrap();
    assert_eq!(crashy.status, RuntimeStatus::Failed);
    assert!(crashy.last_error.unwrap().contains("gateway down"));
    assert_eq!(cluster.get("done").unwrap().status, RuntimeStatus::Stopped);
    cluster.destroy().await;
    assert!(cluster.is_empty());
}
