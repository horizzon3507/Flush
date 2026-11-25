//! Voice runtime tests: mock driver/connection, normalized event
//! delivery, sanitization and the voice-state tracker (yuzuha
//! `voice-events.test.ts` semantics).

use std::future::Future;
use std::io::Read;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use flush_core::{Channel, Snowflake};
use flush_framework::{
    SelfVoiceState, Voice, VoiceConnCallback, VoiceConnEvent, VoiceConnection, VoiceDriver,
    VoiceError, VoiceEvent, VoiceEventName, VoiceSource, VoiceStateTracker, VOICE_ERROR_MESSAGE,
};

fn channel(id: u64, guild: u64) -> Channel {
    serde_json::from_str(&format!(
        r#"{{"id":"{id}","type":2,"guild_id":"{guild}","name":"voz"}}"#
    ))
    .expect("channel")
}

fn voice_state(user: u64, guild: u64, channel: Option<u64>) -> flush_core::VoiceState {
    serde_json::from_value(serde_json::json!({
        "user_id": user.to_string(),
        "guild_id": guild.to_string(),
        "channel_id": channel.map(|c| c.to_string()),
    }))
    .expect("voice state")
}

struct MockConn {
    listeners: Mutex<Vec<(u64, VoiceConnCallback)>>,
    next: AtomicU64,
    stopped: AtomicUsize,
    volume: AtomicUsize,
    play_error: Mutex<Option<VoiceError>>,
}

impl MockConn {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            listeners: Mutex::new(Vec::new()),
            next: AtomicU64::new(1),
            stopped: AtomicUsize::new(0),
            volume: AtomicUsize::new(0),
            play_error: Mutex::new(None),
        })
    }

    fn emit(&self, ev: VoiceConnEvent) {
        let listeners: Vec<VoiceConnCallback> = self
            .listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(_, cb)| cb.clone())
            .collect();
        for cb in listeners {
            cb(ev.clone());
        }
    }

    fn listener_count(&self) -> usize {
        self.listeners.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

impl VoiceConnection for MockConn {
    fn get_volume(&self) -> Option<u32> {
        Some(self.volume.load(Ordering::SeqCst) as u32)
    }

    fn play<'a>(
        &'a self,
        _source: VoiceSource,
    ) -> Pin<Box<dyn Future<Output = Result<(), VoiceError>> + Send + 'a>> {
        Box::pin(async move {
            match self
                .play_error
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
            {
                Some(e) => Err(e),
                None => Ok(()),
            }
        })
    }

    fn stop(&self) {
        self.stopped.fetch_add(1, Ordering::SeqCst);
    }

    fn set_volume(&self, volume: u32) -> Result<(), VoiceError> {
        self.volume.store(volume as usize, Ordering::SeqCst);
        Ok(())
    }

    fn subscribe(&self, cb: VoiceConnCallback) -> u64 {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        self.listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((id, cb));
        id
    }

    fn unsubscribe(&self, id: u64) {
        self.listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(l, _)| *l != id);
    }
}

struct MockDriver {
    conn: Arc<MockConn>,
    join_error: Mutex<Option<VoiceError>>,
    left: AtomicUsize,
    updated: Mutex<Vec<SelfVoiceState>>,
}

impl MockDriver {
    fn new(conn: Arc<MockConn>) -> Self {
        Self {
            conn,
            join_error: Mutex::new(None),
            left: AtomicUsize::new(0),
            updated: Mutex::new(Vec::new()),
        }
    }

    fn fail_next_join(&self, error: VoiceError) {
        *self.join_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(error);
    }
}

impl VoiceDriver for MockDriver {
    fn join(
        &self,
        _channel: Channel,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn VoiceConnection>, VoiceError>> + Send>> {
        let error = self
            .join_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        let conn: Arc<dyn VoiceConnection> = self.conn.clone();
        Box::pin(async move {
            match error {
                Some(e) => Err(e),
                None => Ok(conn),
            }
        })
    }

    fn leave(&self, _guild_id: Snowflake) {
        self.left.fetch_add(1, Ordering::SeqCst);
    }

    fn update_voice_state(
        &self,
        _channel_id: Snowflake,
        state: SelfVoiceState,
    ) -> Result<(), VoiceError> {
        self.updated
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(state);
        Ok(())
    }
}

struct Collector {
    events: Mutex<Vec<VoiceEvent>>,
}

fn collect(voice: &Voice) -> (flush_framework::VoiceEventGuard, Arc<Collector>) {
    let collector = Arc::new(Collector {
        events: Mutex::new(Vec::new()),
    });
    let captured = collector.clone();
    let guard = voice.subscribe(move |ev| {
        captured
            .events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(ev.clone());
    });
    (guard, collector)
}

fn names(events: &[VoiceEvent]) -> Vec<VoiceEventName> {
    events.iter().map(|e| e.name).collect()
}

#[async_std::test]
async fn driver_events_are_normalized_and_sanitized() {
    let conn = MockConn::new();
    let voice = Voice::new(Arc::new(MockDriver::new(conn.clone())));
    let (_guard, collected) = collect(&voice);

    voice
        .join(channel(300, 100), Some(Snowflake(100)))
        .await
        .expect("join");
    conn.emit(VoiceConnEvent::Error {
        code: Some("ECONNRESET".into()),
    });
    conn.emit(VoiceConnEvent::ServerLeave);
    conn.emit(VoiceConnEvent::SpeakerStart {
        participant_id: "2".into(),
    });
    conn.emit(VoiceConnEvent::SpeakerStop {
        participant_id: "2".into(),
    });
    conn.emit(VoiceConnEvent::Disconnect);

    let events = collected
        .events
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    assert_eq!(
        names(&events),
        vec![
            VoiceEventName::Ready,
            VoiceEventName::Error,
            VoiceEventName::ServerLeave,
            VoiceEventName::SpeakerStart,
            VoiceEventName::SpeakerStop,
            VoiceEventName::Disconnect,
        ]
    );
    let error = &events[1];
    assert_eq!(error.error_code.as_deref(), Some("ECONNRESET"));
    assert_eq!(error.error_message.as_deref(), Some(VOICE_ERROR_MESSAGE));
    assert_eq!(events[3].user_id, Some(Snowflake(2)));
    assert_eq!(events[4].user_id, Some(Snowflake(2)));
    for ev in &events {
        assert_eq!(ev.guild_id, Snowflake(100));
        assert_eq!(ev.channel_id, Snowflake(300));
    }
    // Disconnect evicted the entry and unsubscribed the connection callback.
    assert!(voice.get(Snowflake(100)).is_none());
    assert_eq!(conn.listener_count(), 0);
}

#[async_std::test]
async fn server_leave_before_disconnect_still_arrives() {
    let conn = MockConn::new();
    let voice = Voice::new(Arc::new(MockDriver::new(conn.clone())));
    let (_guard, collected) = collect(&voice);

    voice.join(channel(300, 100), None).await.expect("join");
    conn.emit(VoiceConnEvent::ServerLeave);
    conn.emit(VoiceConnEvent::Disconnect);
    assert_eq!(conn.listener_count(), 0);
    // Nothing is delivered after cleanup.
    conn.emit(VoiceConnEvent::ServerLeave);

    let events = collected
        .events
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    assert_eq!(
        names(&events),
        vec![
            VoiceEventName::Ready,
            VoiceEventName::ServerLeave,
            VoiceEventName::Disconnect,
        ]
    );
}

#[async_std::test]
async fn unknown_error_codes_are_replaced() {
    let conn = MockConn::new();
    let voice = Voice::new(Arc::new(MockDriver::new(conn.clone())));
    let (_guard, collected) = collect(&voice);

    voice.join(channel(300, 100), None).await.expect("join");
    conn.emit(VoiceConnEvent::Error {
        code: Some("token-secreto".into()),
    });

    let events = collected
        .events
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].error_code.as_deref(), Some("VOICE_STATE"));
    assert_eq!(events[1].error_message.as_deref(), Some(VOICE_ERROR_MESSAGE));
}

#[async_std::test]
async fn speaker_events_are_capped_and_validated() {
    let conn = MockConn::new();
    let voice = Voice::new(Arc::new(MockDriver::new(conn.clone())));
    let (_guard, collected) = collect(&voice);

    voice.join(channel(300, 100), None).await.expect("join");
    // Invalid participant ids never reach listeners.
    conn.emit(VoiceConnEvent::SpeakerStart {
        participant_id: "token=secreto".into(),
    });
    conn.emit(VoiceConnEvent::SpeakerStart {
        participant_id: "1".repeat(129),
    });
    // 80 alternating start/stop events inside one window: 20 delivered.
    for i in 0..80 {
        conn.emit(if i % 2 == 1 {
            VoiceConnEvent::SpeakerStart {
                participant_id: "2".into(),
            }
        } else {
            VoiceConnEvent::SpeakerStop {
                participant_id: "2".into(),
            }
        });
    }

    let events = collected
        .events
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    assert_eq!(events.len(), 21); // ready + 20 speakers
    assert!(
        events[1..]
            .iter()
            .all(|e| e.user_id == Some(Snowflake(2)))
    );
}

#[async_std::test]
async fn join_failure_publishes_voice_error() {
    let conn = MockConn::new();
    let driver = MockDriver::new(conn);
    driver.fail_next_join(VoiceError::Driver(
        "endpoint=wss://secreto token=segredo".into(),
    ));
    let voice = Voice::new(Arc::new(driver));
    let (_guard, collected) = collect(&voice);

    let error = voice.join(channel(300, 100), None).await.err().expect("join error");
    assert_eq!(error.code(), "VOICE_STATE");

    let events = collected
        .events
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    assert_eq!(
        names(&events),
        vec![VoiceEventName::Error],
        "no ready before a failed join"
    );
    assert_eq!(events[0].error_code.as_deref(), Some("VOICE_STATE"));
    assert_eq!(
        events[0].error_message.as_deref(),
        Some(VOICE_ERROR_MESSAGE)
    );
    assert_eq!(voice.size(), 0);
}

#[async_std::test]
async fn join_dedup_and_leave() {
    let conn = MockConn::new();
    let driver = Arc::new(MockDriver::new(conn.clone()));
    let voice = Voice::new(driver.clone());

    let first = voice.join(channel(300, 100), None).await.expect("join");
    // Rejoining the same channel returns the live connection.
    let again = voice.join(channel(300, 100), None).await.expect("rejoin");
    assert!(Arc::ptr_eq(&first, &again));
    // A different channel in the same guild is rejected.
    let error = voice.join(channel(301, 100), None).await.err().expect("join error");
    assert_eq!(error.code(), "VOICE_CHANNEL");
    // A mismatched expected guild is rejected.
    let error = voice
        .join(channel(400, 200), Some(Snowflake(100)))
        .await
        .err()
        .expect("join error");
    assert_eq!(error.code(), "VOICE_GUILD");
    // A non-voice channel is rejected.
    let text: Channel = serde_json::from_str(r#"{"id":"400","type":0,"guild_id":"100"}"#)
        .expect("text channel");
    let error = voice.join(text, None).await.err().expect("join error");
    assert_eq!(error.code(), "VOICE_CHANNEL");

    assert!(voice.leave(Snowflake(100)).await.expect("leave"));
    assert_eq!(voice.size(), 0);
    assert!(!voice.leave(Snowflake(100)).await.expect("no-op leave"));
    assert_eq!(driver.left.load(Ordering::SeqCst), 1);
}

#[async_std::test]
async fn self_state_merges_and_volume_is_bounded() {
    let conn = MockConn::new();
    let driver = Arc::new(MockDriver::new(conn.clone()));
    let voice = Voice::new(driver.clone());

    voice.join(channel(300, 100), None).await.expect("join");
    voice
        .update_voice_state(Snowflake(100), Some(true), None)
        .await
        .expect("mute");
    voice
        .update_voice_state(Snowflake(100), None, Some(true))
        .await
        .expect("deaf");
    let updates = driver
        .updated
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    assert_eq!(
        updates,
        vec![
            SelfVoiceState {
                self_mute: true,
                self_deaf: false
            },
            SelfVoiceState {
                self_mute: true,
                self_deaf: true
            },
        ]
    );

    voice
        .set_volume(Snowflake(100), 120)
        .expect("volume in range");
    assert_eq!(conn.get_volume(), Some(120));
    let error = voice.set_volume(Snowflake(100), 250).unwrap_err();
    assert_eq!(error.code(), "VOICE_VOLUME");

    let error = voice
        .update_voice_state(Snowflake(999), Some(true), None)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "VOICE_NOT_CONNECTED");
}

#[async_std::test]
async fn play_surfaces_driver_errors() {
    let conn = MockConn::new();
    let voice = Voice::new(Arc::new(MockDriver::new(conn.clone())));
    voice.join(channel(300, 100), None).await.expect("join");

    *conn.play_error.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(VoiceError::Driver("broken pipe".into()));
    let error = voice
        .play(Snowflake(100), VoiceSource::Uri("https://a/b.opus".into()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "VOICE_STATE");

    voice.stop(Snowflake(100)).expect("stop");
    assert_eq!(conn.stopped.load(Ordering::SeqCst), 1);

    let error = voice
        .play(
            Snowflake(999),
            VoiceSource::Reader(Box::new(std::io::empty()) as Box<dyn Read + Send>),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), "VOICE_NOT_CONNECTED");
}

#[async_std::test]
async fn close_unsubscribes_and_join_errors() {
    let conn = MockConn::new();
    let voice = Voice::new(Arc::new(MockDriver::new(conn.clone())));
    let (_guard, _collected) = collect(&voice);

    voice.join(channel(300, 100), None).await.expect("join");
    voice.close().await.expect("close");
    assert_eq!(voice.size(), 0);
    assert_eq!(conn.listener_count(), 0);

    let error = voice.join(channel(300, 100), None).await.err().expect("join error");
    assert_eq!(error.code(), "VOICE_CLOSED");
    // Subscribing after close registers nothing.
    let (_late, late) = collect(&voice);
    assert!(
        late.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    );
    voice.close().await.expect("idempotent close");
}

#[test]
fn tracker_applies_syncs_and_evicts() {
    let tracker = VoiceStateTracker::new();
    tracker.apply(&voice_state(2, 100, Some(300)));
    tracker.apply(&voice_state(3, 100, Some(301)));
    tracker.apply(&voice_state(4, 200, Some(400)));
    assert_eq!(tracker.size(), 3);
    assert_eq!(
        tracker.user_channel(Snowflake(100), Snowflake(2)),
        Some(Snowflake(300))
    );

    // A null channel means the user left.
    tracker.apply(&voice_state(2, 100, None));
    assert_eq!(tracker.size(), 2);
    assert_eq!(tracker.user_channel(Snowflake(100), Snowflake(2)), None);

    // Guild sync replaces the guild's snapshot.
    tracker.sync_guild(Snowflake(100), &[voice_state(5, 100, Some(500))]);
    assert_eq!(tracker.size(), 2);
    assert_eq!(tracker.guild(Snowflake(100)).len(), 1);

    // Channel delete evicts that channel's states.
    tracker.apply(&voice_state(6, 100, Some(500)));
    tracker.clear_channel(Snowflake(500), Some(Snowflake(100)));
    assert_eq!(tracker.guild(Snowflake(100)).len(), 0);

    // Guild delete drops everything of that guild.
    tracker.apply(&voice_state(7, 100, Some(700)));
    tracker.clear(Some(Snowflake(100)));
    assert_eq!(tracker.size(), 1);
    tracker.clear(None);
    assert_eq!(tracker.size(), 0);
}
