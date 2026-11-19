use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_std::net::{TcpListener, TcpStream};
use async_tungstenite::tungstenite::Message;
use futures::StreamExt;
use serde_json::{json, Value};

use flush_core::events::Event;
use flush_core::presence::{Presence, PresenceStatus};
use flush_core::Snowflake;
use flush_gateway::{
    recommended_shard_count, shard_id_for_guild, ClientConfig, ConnectError, ShardError,
    ShardEvent, ShardManager, ShardManagerOptions, ShardPlan, ShardStatus, MAX_SHARD_COUNT,
};

#[test]
fn guild_routing_follows_the_gateway_formula() {
    let guild = Snowflake::new(5 << 22);
    assert_eq!(shard_id_for_guild(guild, 1), Ok(0));
    assert_eq!(shard_id_for_guild(guild, 2), Ok(1));
    assert_eq!(shard_id_for_guild(guild, 4), Ok(1));
    assert_eq!(
        shard_id_for_guild(Snowflake::new((7 << 22) | 0x3f_ffff), 4),
        Ok(3)
    );
    assert_eq!(shard_id_for_guild(guild, 0), Err(ShardError::InvalidCount));
    assert_eq!(
        shard_id_for_guild(guild, MAX_SHARD_COUNT + 1),
        Err(ShardError::InvalidCount)
    );
}

#[test]
fn recommended_count_rounds_up_and_clamps() {
    assert_eq!(recommended_shard_count(0, 0), 1);
    assert_eq!(recommended_shard_count(1_500, 0), 1);
    assert_eq!(recommended_shard_count(1_501, 0), 2);
    assert_eq!(recommended_shard_count(10, 3), 4);
    assert_eq!(recommended_shard_count(u64::MAX, 1), MAX_SHARD_COUNT);
}

#[test]
fn plans_validate_ids_and_count() {
    assert_eq!(ShardPlan::all(3).unwrap().ids(), &[0, 1, 2]);
    assert_eq!(ShardPlan::all(0), Err(ShardError::InvalidCount));
    assert_eq!(
        ShardPlan::all(MAX_SHARD_COUNT + 1),
        Err(ShardError::InvalidCount)
    );
    let plan = ShardPlan::new(&[2, 0], 4).unwrap();
    assert_eq!((plan.ids(), plan.count()), (&[2, 0][..], 4));
    assert!(plan.contains(2) && !plan.contains(1));
    assert_eq!(ShardPlan::new(&[], 4), Err(ShardError::EmptyIds));
    assert_eq!(ShardPlan::new(&[4], 4), Err(ShardError::IdOutOfRange(4)));
    assert_eq!(ShardPlan::new(&[1, 1], 4), Err(ShardError::DuplicateId(1)));
    assert_eq!(ShardPlan::from_ids(&[1, 3]).unwrap().count(), 4);
    assert_eq!(ShardPlan::from_ids(&[]), Err(ShardError::EmptyIds));
}

/// What the fake gateway saw: `(op, d)` per client frame, heartbeats left out.
type Seen = Arc<Mutex<Vec<(u64, Value)>>>;

async fn serve(stream: TcpStream, seen: Seen, close_with: Option<u16>) {
    let mut ws = async_tungstenite::accept_async(stream).await.unwrap();
    let hello = json!({"op": 10, "d": {"heartbeat_interval": 45_000}});
    ws.send(Message::Text(hello.to_string().into()))
        .await
        .unwrap();
    while let Some(Ok(Message::Text(text))) = ws.next().await {
        let frame: Value = serde_json::from_str(&text).unwrap();
        let op = frame["op"].as_u64().unwrap();
        if op == 1 {
            continue;
        }
        seen.lock().unwrap().push((op, frame["d"].clone()));
        let reply = match op {
            2 => {
                if let Some(code) = close_with {
                    use async_tungstenite::tungstenite::protocol::CloseFrame;
                    let frame = CloseFrame {
                        code: code.into(),
                        reason: "".into(),
                    };
                    let _ = ws.send(Message::Close(Some(frame))).await;
                    return;
                }
                let shard = frame["d"]["shard"][0].as_u64().unwrap();
                json!({"op": 0, "t": "READY", "s": 1, "d": {
                    "session_id": format!("session-{shard}"),
                    "guilds": [],
                }})
            }
            6 => json!({"op": 0, "t": "RESUMED", "s": 2, "d": {}}),
            _ => continue,
        };
        ws.send(Message::Text(reply.to_string().into()))
            .await
            .unwrap();
    }
}

async fn fake_gateway(close_with: Option<u16>) -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    async_std::task::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            async_std::task::spawn(serve(stream, log.clone(), close_with));
        }
    });
    (url, seen)
}

fn options() -> ShardManagerOptions {
    ShardManagerOptions {
        identify_delay: Duration::from_millis(20),
        retry_delay: Duration::from_millis(20),
    }
}

async fn next(manager: &mut ShardManager) -> ShardEvent {
    async_std::future::timeout(Duration::from_secs(10), manager.next())
        .await
        .expect("shard event within 10s")
        .expect("shards still running")
}

async fn wait_for(seen: &Seen, op: u64, count: usize) -> Vec<Value> {
    for _ in 0..500 {
        let found: Vec<Value> = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(o, _)| *o == op)
            .map(|(_, d)| d.clone())
            .collect();
        if found.len() >= count {
            return found;
        }
        async_std::task::sleep(Duration::from_millis(10)).await;
    }
    panic!("gateway never saw {count} frames with op {op}");
}

#[async_std::test]
async fn manager_identifies_each_shard_and_merges_events() {
    let (url, seen) = fake_gateway(None).await;
    let plan = ShardPlan::new(&[0, 2], 3).unwrap();
    let mut manager =
        ShardManager::spawn(ClientConfig::new("token", url), plan, options()).unwrap();

    let mut ready = Vec::new();
    while ready.len() < 2 {
        if let ShardEvent::Dispatch { shard_id, event } = next(&mut manager).await {
            if let Event::Ready(payload) = *event {
                assert_eq!(payload.session_id, format!("session-{shard_id}"));
                ready.push(shard_id);
            }
        }
    }
    ready.sort_unstable();
    assert_eq!(ready, vec![0, 2]);
    assert_eq!(manager.status(0), Some(ShardStatus::Ready));
    assert_eq!(manager.status(2), Some(ShardStatus::Ready));
    assert_eq!(manager.status(1), None);

    let mut shards: Vec<Value> = wait_for(&seen, 2, 2)
        .await
        .into_iter()
        .map(|d| d["shard"].clone())
        .collect();
    shards.sort_by_key(|s| s[0].as_u64());
    assert_eq!(shards, vec![json!([0, 3]), json!([2, 3])]);

    let presence = Presence {
        status: PresenceStatus::Idle,
        ..Default::default()
    };
    manager.set_presence(&presence);
    assert_eq!(wait_for(&seen, 3, 2).await.len(), 2);

    // Guild (4 << 22) belongs to shard 1 of 3, which this process does not run.
    assert_eq!(
        manager.request_guild_members(Snowflake::new(4 << 22), "", 10),
        Err(ShardError::UnknownShard(1))
    );
    manager
        .request_guild_members(Snowflake::new(5 << 22), "", 10)
        .unwrap();
    assert_eq!(wait_for(&seen, 8, 1).await.len(), 1);

    manager.shutdown().await;
    assert_eq!(manager.status(0), Some(ShardStatus::Closed));
}

#[async_std::test]
async fn reconnect_resumes_the_shard_session() {
    let (url, seen) = fake_gateway(None).await;
    let plan = ShardPlan::all(1).unwrap();
    let mut manager =
        ShardManager::spawn(ClientConfig::new("token", url), plan, options()).unwrap();
    loop {
        if let ShardEvent::Dispatch { event, .. } = next(&mut manager).await {
            if matches!(*event, Event::Ready(_)) {
                break;
            }
        }
    }
    assert_eq!(manager.reconnect(7), Err(ShardError::UnknownShard(7)));
    manager.reconnect(0).unwrap();
    let mut statuses = Vec::new();
    loop {
        match next(&mut manager).await {
            ShardEvent::StatusChanged { status, .. } => statuses.push(status),
            ShardEvent::Dispatch { event, .. } if matches!(*event, Event::Resumed { .. }) => break,
            _ => {}
        }
    }
    assert_eq!(
        statuses,
        vec![ShardStatus::Reconnecting, ShardStatus::Ready]
    );
    let resumes = wait_for(&seen, 6, 1).await;
    assert_eq!(resumes[0]["session_id"], "session-0");
    manager.shutdown().await;
}

#[async_std::test]
async fn invalid_shard_close_is_fatal() {
    let (url, _seen) = fake_gateway(Some(4010)).await;
    let plan = ShardPlan::all(1).unwrap();
    let mut manager =
        ShardManager::spawn(ClientConfig::new("token", url), plan, options()).unwrap();
    loop {
        if let ShardEvent::Fatal { shard_id, error } = next(&mut manager).await {
            assert_eq!(shard_id, 0);
            assert!(matches!(error, ConnectError::InvalidShard), "{error:?}");
            break;
        }
    }
    assert_eq!(manager.status(0), Some(ShardStatus::Closed));
    assert!(manager.next().await.is_none());
}
