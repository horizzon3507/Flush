//! Testing helpers: `mock_track` and `MockMusicBackend`, like yuzuha's
//! `testing/music.ts`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Map;

use crate::backend::{
    BoxMusicFuture, EventListener, LifecycleListener, MusicBackend, Subscription,
};
use crate::domain::{
    MusicEvent, MusicNodeEvent, MusicPlayerState, MusicPlayerUpdate, MusicTrack,
    MusicVoiceState, RemoteMusicPlayer, TrackLoadResult,
};
use crate::error::MusicError;

/// A deterministic track for tests.
pub fn mock_track(identifier: &str) -> MusicTrack {
    MusicTrack {
        encoded: identifier.to_string(),
        info: crate::domain::TrackInfo {
            identifier: identifier.to_string(),
            is_seekable: true,
            author: "Mock artist".to_string(),
            length: 180_000,
            is_stream: false,
            position: 0,
            title: identifier.to_string(),
            uri: Some(format!("https://example.com/{identifier}")),
            artwork_url: None,
            isrc: None,
            source_name: "mock".to_string(),
        },
        plugin_info: Map::new(),
        user_data: Map::new(),
    }
}

pub struct RecordedUpdate {
    pub guild_id: String,
    pub update: MusicPlayerUpdate,
}

/// In-memory `MusicBackend`: records calls, serves `results` for
/// `load_tracks`, and keeps remote player state per guild.
/// Called before each `update_player` is recorded.
pub type BeforeUpdateHook = Arc<dyn Fn(&str, &MusicPlayerUpdate) + Send + Sync>;

pub struct MockMusicBackend {
    pub available: Mutex<bool>,
    pub penalty: Mutex<f64>,
    pub updates: Mutex<Vec<RecordedUpdate>>,
    pub destroyed: Mutex<Vec<String>>,
    pub loads: Mutex<Vec<String>>,
    pub results: Mutex<HashMap<String, TrackLoadResult>>,
    pub players: Mutex<HashMap<String, RemoteMusicPlayer>>,
    listeners: Mutex<Vec<(u64, EventListener)>>,
    lifecycle_listeners: Mutex<Vec<(u64, LifecycleListener)>>,
    next_listener: AtomicUsize,
    /// Fail the next `update_player` with this error, once.
    pub fail_next_update: Mutex<Option<MusicError>>,
    /// Fail every `destroy_player` with this error.
    pub fail_destroy: Mutex<Option<MusicError>>,
    /// Called before each `update_player` is recorded.
    pub before_update: Mutex<Option<BeforeUpdateHook>>,
    me: std::sync::Weak<MockMusicBackend>,
}

impl MockMusicBackend {
    pub fn new() -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            available: Mutex::new(true),
            penalty: Mutex::new(0.0),
            updates: Mutex::new(Vec::new()),
            destroyed: Mutex::new(Vec::new()),
            loads: Mutex::new(Vec::new()),
            results: Mutex::new(HashMap::new()),
            players: Mutex::new(HashMap::new()),
            listeners: Mutex::new(Vec::new()),
            lifecycle_listeners: Mutex::new(Vec::new()),
            next_listener: AtomicUsize::new(0),
            fail_next_update: Mutex::new(None),
            fail_destroy: Mutex::new(None),
            before_update: Mutex::new(None),
        })
    }

    pub fn subscription_count(&self) -> usize {
        self.listeners.lock().unwrap().len()
    }

    pub fn lifecycle_subscription_count(&self) -> usize {
        self.lifecycle_listeners.lock().unwrap().len()
    }

    /// Deliver an event to every subscriber (as the node would).
    pub fn emit(&self, event: MusicEvent) {
        let listeners = self.listeners.lock().unwrap().clone();
        for (_, listener) in listeners {
            listener(event.clone());
        }
    }

    /// Deliver a lifecycle event; `Ready { resumed: false }` marks the node
    /// available and clears remote players.
    pub fn emit_lifecycle(&self, event: MusicNodeEvent) {
        match &event {
            MusicNodeEvent::Ready { resumed: false, .. } => {
                *self.available.lock().unwrap() = true;
                self.players.lock().unwrap().clear();
            }
            MusicNodeEvent::Ready { .. } => {
                *self.available.lock().unwrap() = true;
            }
            _ => {}
        }
        let listeners = self.lifecycle_listeners.lock().unwrap().clone();
        for (_, listener) in listeners {
            listener(event.clone());
        }
    }

    fn remote_player(&self, guild_id: &str) -> RemoteMusicPlayer {
        RemoteMusicPlayer {
            guild_id: guild_id.to_string(),
            track: None,
            volume: 100,
            paused: false,
            state: MusicPlayerState {
                time: 0,
                position: 0,
                connected: true,
                ping: 0,
            },
            voice: MusicVoiceState {
                token: "mock".to_string(),
                endpoint: "mock".to_string(),
                session_id: "mock".to_string(),
                channel_id: None,
            },
            filters: Default::default(),
        }
    }
}

impl MusicBackend for MockMusicBackend {
    fn available(&self) -> Option<bool> {
        Some(*self.available.lock().unwrap())
    }

    fn penalty(&self) -> f64 {
        *self.penalty.lock().unwrap()
    }

    fn load_tracks<'a>(&'a self, identifier: &'a str) -> BoxMusicFuture<'a, TrackLoadResult> {
        self.loads.lock().unwrap().push(identifier.to_string());
        let result = self
            .results
            .lock()
            .unwrap()
            .get(identifier)
            .cloned()
            .unwrap_or(TrackLoadResult::Empty(None));
        Box::pin(async move { Ok(result) })
    }

    fn update_player<'a>(
        &'a self,
        guild_id: &'a str,
        update: MusicPlayerUpdate,
        no_replace: bool,
    ) -> BoxMusicFuture<'a, RemoteMusicPlayer> {
        if let Some(before) = self.before_update.lock().unwrap().clone() {
            before(guild_id, &update);
        }
        if let Some(error) = self.fail_next_update.lock().unwrap().take() {
            return Box::pin(async move { Err(error) });
        }
        self.updates.lock().unwrap().push(RecordedUpdate {
            guild_id: guild_id.to_string(),
            update: update.clone(),
        });
        let mut players = self.players.lock().unwrap();
        let mut player = players
            .get(guild_id)
            .cloned()
            .unwrap_or_else(|| self.remote_player(guild_id));
        if let Some(track) = &update.track {
            if !no_replace || player.track.is_none() {
                match track {
                    crate::domain::UpdateTrack::Encoded {
                        encoded, user_data,
                    } => {
                        player.track = encoded.as_ref().map(|encoded| {
                            let mut track = mock_track(encoded);
                            if let Some(user_data) = user_data {
                                track.user_data = user_data.clone();
                            }
                            track
                        });
                    }
                    crate::domain::UpdateTrack::Identifier {
                        identifier,
                        user_data,
                    } => {
                        let mut track = mock_track(identifier);
                        if let Some(user_data) = user_data {
                            track.user_data = user_data.clone();
                        }
                        player.track = Some(track);
                    }
                }
                player.state.position = 0;
            }
        }
        if let Some(volume) = update.volume {
            player.volume = volume;
        }
        if let Some(paused) = update.paused {
            player.paused = paused;
        }
        if let Some(voice) = &update.voice {
            player.voice = voice.clone();
        }
        if let Some(filters) = &update.filters {
            player.filters = filters.clone();
        }
        if let Some(position) = update.position {
            player.state.position = position;
        }
        players.insert(guild_id.to_string(), player.clone());
        Box::pin(async move { Ok(player) })
    }

    fn destroy_player<'a>(&'a self, guild_id: &'a str) -> BoxMusicFuture<'a, ()> {
        if let Some(error) = self.fail_destroy.lock().unwrap().clone() {
            return Box::pin(async move { Err(error) });
        }
        self.destroyed.lock().unwrap().push(guild_id.to_string());
        self.players.lock().unwrap().remove(guild_id);
        Box::pin(async move { Ok(()) })
    }

    fn subscribe(&self, listener: EventListener) -> Subscription {
        let id = self.next_listener.fetch_add(1, Ordering::SeqCst) as u64;
        self.listeners.lock().unwrap().push((id, listener));
        let me = self.me.clone();
        Subscription::new(move || {
            if let Some(me) = me.upgrade() {
                me.listeners
                    .lock()
                    .unwrap()
                    .retain(|(listener_id, _)| *listener_id != id);
            }
        })
    }

    fn subscribe_lifecycle(&self, listener: LifecycleListener) -> Option<Subscription> {
        let id = self.next_listener.fetch_add(1, Ordering::SeqCst) as u64;
        self.lifecycle_listeners.lock().unwrap().push((id, listener));
        let me = self.me.clone();
        Some(Subscription::new(move || {
            if let Some(me) = me.upgrade() {
                me.lifecycle_listeners
                    .lock()
                    .unwrap()
                    .retain(|(listener_id, _)| *listener_id != id);
            }
        }))
    }
}
