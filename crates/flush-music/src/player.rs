//! `MusicPlayer`: per-guild playback state with serialized control.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use async_std::task;
use futures::channel::{mpsc, oneshot};
use futures::StreamExt;

use crate::backend::{
    BoxMusicFuture, MusicAudioOutput, MusicBackend, MusicResult, MusicVoiceAdapter, Subscription,
};
use crate::domain::{
    MusicEvent, MusicFilters, MusicPlayerState, MusicPlayerUpdate, MusicTrack, MusicVoiceState,
    TrackEndReason, TrackLoadResult, UpdateTrack,
};
use crate::error::MusicError;
use crate::queue::TrackQueue;

/// Marker stored in `track.userData` so events can be tied to a playback.
pub const PLAYBACK_MARKER: &str = "__flushPlayback";

type Reply<T> = oneshot::Sender<MusicResult<T>>;

enum Op {
    Play(Reply<Option<MusicTrack>>),
    Skip(Reply<Option<MusicTrack>>),
    Stop {
        clear_queue: bool,
        reply: Reply<()>,
    },
    Pause {
        paused: bool,
        reply: Reply<()>,
    },
    SetVolume {
        volume: i64,
        reply: Reply<()>,
    },
    Seek {
        position: i64,
        reply: Reply<()>,
    },
    SetFilters {
        filters: MusicFilters,
        reply: Reply<()>,
    },
    Connect {
        channel_id: String,
        reply: Reply<()>,
    },
    Recover(Reply<()>),
    Event(MusicEvent),
    AudioState {
        connected: bool,
        error: Option<MusicError>,
    },
    Drain(oneshot::Sender<()>),
    Destroy(Reply<Vec<MusicError>>),
}

/// Events a `MusicPlayer` emits to `subscribe` listeners.
#[derive(Debug, Clone)]
pub enum PlayerEvent {
    TrackStart(MusicTrack),
    TrackEnd(MusicTrack, TrackEndReason),
    QueueEnd,
    PlayerError(Arc<MusicError>),
    VoiceClose(f64),
    NodeDisconnect(Option<u16>),
    Recovered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopMode {
    Off,
    Track,
    Queue,
}

type ListenerList = Arc<Mutex<Vec<(u64, Arc<dyn Fn(PlayerEvent) + Send + Sync>)>>>;

#[derive(Debug, Clone)]
struct SharedState {
    current: Option<MusicTrack>,
    state: Option<MusicPlayerState>,
    state_received_at: Instant,
    filters: MusicFilters,
    channel: Option<String>,
    paused: bool,
    volume: i64,
    repeat: LoopMode,
    recovery_required: bool,
    audio_connected: bool,
    playback_id: Option<String>,
    playback_prefix: String,
    sequence: u64,
}

/// One guild's player. All controls are serialized FIFO like yuzuha's `tail`.
pub struct MusicPlayer {
    guild_id: String,
    node: Arc<dyn MusicBackend>,
    shared: RwLock<SharedState>,
    tracks: Mutex<TrackQueue<MusicTrack>>,
    tx: mpsc::UnboundedSender<Op>,
    pending: Arc<AtomicUsize>,
    max_pending_controls: usize,
    closing: AtomicBool,
    listeners: ListenerList,
    next_listener: AtomicU64,
}

impl MusicPlayer {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        guild_id: String,
        node: Arc<dyn MusicBackend>,
        queue_capacity: usize,
        voice: Option<Arc<dyn MusicVoiceAdapter>>,
        audio: Option<Arc<dyn MusicAudioOutput>>,
        on_error: Arc<dyn Fn(MusicError) + Send + Sync>,
        release: Box<dyn FnOnce() + Send>,
        max_pending_controls: usize,
    ) -> Result<Arc<Self>, MusicError> {
        if !(1..=10_000).contains(&max_pending_controls) {
            return Err(MusicError::new(
                "MUSIC_OPTIONS",
                "max_pending_controls must be between 1 and 10000.",
            ));
        }
        let prefix = new_prefix();
        let (tx, rx) = mpsc::unbounded();
        let player = Arc::new(Self {
            guild_id: guild_id.clone(),
            node: node.clone(),
            shared: RwLock::new(SharedState {
                current: None,
                state: None,
                state_received_at: Instant::now(),
                filters: MusicFilters::default(),
                channel: None,
                paused: false,
                volume: 100,
                repeat: LoopMode::Off,
                recovery_required: false,
                audio_connected: false,
                playback_id: None,
                playback_prefix: prefix.clone(),
                sequence: 0,
            }),
            tracks: Mutex::new(TrackQueue::new(queue_capacity)?),
            tx,
            pending: Arc::new(AtomicUsize::new(0)),
            max_pending_controls,
            closing: AtomicBool::new(false),
            listeners: Arc::new(Mutex::new(Vec::new())),
            next_listener: AtomicU64::new(0),
        });
        let actor = PlayerActor {
            player: player.clone(),
            rx,
            advance_target: None,
            playback_id: None,
            playback_prefix: prefix,
            sequence: 0,
            voice_state: None,
            session_version: 0,
            pending_events: Vec::new(),
            voice_joined: false,
            node,
            voice,
            audio,
            on_error,
            release: Mutex::new(Some(release)),
        };
        task::spawn(actor.run());
        Ok(player)
    }

    pub fn guild_id(&self) -> &str {
        &self.guild_id
    }

    pub fn node(&self) -> &Arc<dyn MusicBackend> {
        &self.node
    }

    fn shared(&self) -> std::sync::RwLockReadGuard<'_, SharedState> {
        self.shared.read().unwrap()
    }

    pub fn current(&self) -> Option<MusicTrack> {
        self.shared().current.clone()
    }

    pub fn queue(&self) -> Vec<MusicTrack> {
        self.tracks.lock().unwrap().to_vec()
    }

    pub fn queue_size(&self) -> usize {
        self.tracks.lock().unwrap().len()
    }

    pub fn state(&self) -> Option<MusicPlayerState> {
        self.shared().state
    }

    /// Estimated playback position in milliseconds.
    pub fn position(&self) -> i64 {
        position_of(&self.shared())
    }

    pub fn is_recovering(&self) -> bool {
        self.shared().recovery_required
    }

    pub fn pending_controls(&self) -> usize {
        self.pending.load(Ordering::SeqCst)
    }

    pub fn active_filters(&self) -> MusicFilters {
        self.shared().filters.clone()
    }

    pub fn voice_channel_id(&self) -> Option<String> {
        self.shared().channel.clone()
    }

    pub fn is_paused(&self) -> bool {
        self.shared().paused
    }

    pub fn volume(&self) -> i64 {
        self.shared().volume
    }

    pub fn loop_mode(&self) -> LoopMode {
        self.shared().repeat
    }

    /// Repeat mode: off, the current track, or the whole queue.
    pub fn set_loop(&self, mode: LoopMode) -> MusicResult<()> {
        self.assert_open()?;
        self.shared.write().unwrap().repeat = mode;
        Ok(())
    }

    /// Enqueue one or many tracks. Returns the new queue size.
    pub fn enqueue(&self, tracks: Vec<MusicTrack>) -> MusicResult<usize> {
        self.assert_open()?;
        for track in &tracks {
            validate_track(track)?;
        }
        self.tracks.lock().unwrap().add(tracks)
    }

    pub fn enqueue_result(
        &self,
        result: &TrackLoadResult,
        search_index: usize,
    ) -> MusicResult<usize> {
        self.assert_open()?;
        match result {
            TrackLoadResult::Track(track) => self.enqueue(vec![track.clone()]),
            TrackLoadResult::Playlist(data) => self.enqueue(data.tracks.clone()),
            TrackLoadResult::Search(tracks) => {
                if search_index >= tracks.len() {
                    if tracks.is_empty() && search_index == 0 {
                        return Ok(self.queue_size());
                    }
                    return Err(MusicError::new(
                        "MUSIC_SEARCH_INDEX",
                        "Search result index out of range.",
                    ));
                }
                self.enqueue(vec![tracks[search_index].clone()])
            }
            TrackLoadResult::Empty(_) => Ok(self.queue_size()),
            TrackLoadResult::Error(error) => Err(MusicError::new(
                "MUSIC_LOAD",
                format!(
                    "The track failed to load: {}.",
                    error.message.clone().unwrap_or_else(|| error.cause.clone())
                ),
            )),
        }
    }

    pub fn clear_queue(&self) -> MusicResult<()> {
        self.assert_open()?;
        self.tracks.lock().unwrap().clear();
        Ok(())
    }

    /// A page of up to `count` tracks starting at `start`.
    pub fn queue_page(&self, start: usize, count: usize) -> MusicResult<Vec<MusicTrack>> {
        let tracks = self.tracks.lock().unwrap();
        let remaining = tracks.len().saturating_sub(start);
        if remaining == 0 {
            return Ok(Vec::new());
        }
        tracks.slice(start, count.min(remaining))
    }

    pub fn remove(&self, index: usize, count: usize) -> MusicResult<Vec<MusicTrack>> {
        self.assert_open()?;
        self.tracks.lock().unwrap().remove(index, count)
    }

    pub fn move_track(&self, from: usize, to: usize) -> MusicResult<()> {
        self.assert_open()?;
        self.tracks.lock().unwrap().move_item(from, to)
    }

    /// Fisher–Yates shuffle; `random` must return a value in [0, 1).
    pub fn shuffle<F>(&self, random: F) -> MusicResult<()>
    where
        F: FnMut() -> f64,
    {
        self.assert_open()?;
        self.tracks.lock().unwrap().shuffle(random)
    }

    /// Subscribe to player events. Unsubscribes when the subscription drops.
    pub fn subscribe(&self, listener: Arc<dyn Fn(PlayerEvent) + Send + Sync>) -> Subscription {
        let id = self.next_listener.fetch_add(1, Ordering::SeqCst);
        self.listeners.lock().unwrap().push((id, listener));
        let listeners = self.listeners.clone();
        Subscription::new(move || {
            listeners
                .lock()
                .unwrap()
                .retain(|(listener_id, _)| *listener_id != id);
        })
    }

    pub(crate) fn emit(&self, event: PlayerEvent) {
        let listeners = self.listeners.lock().unwrap().clone();
        for (_, listener) in listeners {
            listener(event.clone());
        }
    }

    fn assert_open(&self) -> MusicResult<()> {
        if self.closing.load(Ordering::SeqCst) {
            return Err(MusicError::new("MUSIC_CLOSED", "The player is closed."));
        }
        Ok(())
    }

    fn send<T: Send + 'static>(
        &self,
        make: impl FnOnce(Reply<T>) -> Op,
    ) -> BoxMusicFuture<'static, T> {
        if let Err(error) = self.assert_open() {
            return Box::pin(async move { Err(error) });
        }
        if self.pending.load(Ordering::SeqCst) >= self.max_pending_controls {
            return Box::pin(async move {
                Err(MusicError::new(
                    "MUSIC_CONTROL_LIMIT",
                    "Player pending-control limit exceeded.",
                ))
            });
        }
        self.pending.fetch_add(1, Ordering::SeqCst);
        let (reply, rx) = oneshot::channel();
        if self.tx.unbounded_send(make(reply)).is_err() {
            self.pending.fetch_sub(1, Ordering::SeqCst);
            return Box::pin(async move {
                Err(MusicError::new("MUSIC_CLOSED", "The player is closed."))
            });
        }
        // The pending slot is released by the actor when the operation
        // finishes, so a dropped reply future cannot leak capacity.
        Box::pin(async move {
            match rx.await {
                Ok(value) => value,
                Err(_) => Err(MusicError::new("MUSIC_CLOSED", "The player is closed.")),
            }
        })
    }

    /// Start the next queued track (or the pending advance); idempotent while
    /// something is already playing.
    pub fn play(&self) -> BoxMusicFuture<'static, Option<MusicTrack>> {
        self.send(Op::Play)
    }

    /// Skip to the next track, or stop the current one when the queue is empty.
    pub fn skip(&self) -> BoxMusicFuture<'static, Option<MusicTrack>> {
        self.send(Op::Skip)
    }

    /// Stop playback; clears the queue unless `clear_queue` is false.
    pub fn stop(&self, clear_queue: bool) -> BoxMusicFuture<'static, ()> {
        self.send(|reply| Op::Stop {
            clear_queue,
            reply,
        })
    }

    pub fn pause(&self, paused: bool) -> BoxMusicFuture<'static, ()> {
        self.send(|reply| Op::Pause { paused, reply })
    }

    /// Volume between 0 and 1000 (100 is unchanged).
    pub fn set_volume(&self, volume: i64) -> BoxMusicFuture<'static, ()> {
        self.send(|reply| Op::SetVolume { volume, reply })
    }

    /// Seek to `position` milliseconds on the current seekable track.
    pub fn seek(&self, position: i64) -> BoxMusicFuture<'static, ()> {
        self.send(|reply| Op::Seek { position, reply })
    }

    pub fn set_filters(&self, filters: MusicFilters) -> BoxMusicFuture<'static, ()> {
        self.send(|reply| Op::SetFilters { filters, reply })
    }

    /// Join `channel_id` through the configured audio output or voice adapter.
    pub fn connect(&self, channel_id: String) -> BoxMusicFuture<'static, ()> {
        self.send(|reply| Op::Connect { channel_id, reply })
    }

    /// Re-assert player state on the backend after a session change.
    pub fn recover(&self) -> BoxMusicFuture<'static, ()> {
        if self.closing.load(Ordering::SeqCst) {
            return Box::pin(async { Err(MusicError::new("MUSIC_CLOSED", "The player is closed.")) });
        }
        let (reply, rx) = oneshot::channel();
        if self.tx.unbounded_send(Op::Recover(reply)).is_err() {
            return Box::pin(async { Err(MusicError::new("MUSIC_CLOSED", "The player is closed.")) });
        }
        Box::pin(async move {
            match rx.await {
                Ok(value) => value,
                Err(_) => Err(MusicError::new("MUSIC_CLOSED", "The player is closed.")),
            }
        })
    }

    /// Mark the Lavalink session stale; controls fail until `recover()`.
    pub fn invalidate_session(&self) {
        if self.closing.load(Ordering::SeqCst) {
            return;
        }
        let mut shared = self.shared.write().unwrap();
        shared.sequence += 1;
        shared.recovery_required = true;
        if let Some(mut state) = shared.state {
            state.position = position_of(&shared);
            shared.state = Some(state);
            shared.state_received_at = Instant::now();
        }
    }

    /// Route a backend event into the player's serialized pipeline.
    pub(crate) fn receive(&self, node: &Arc<dyn MusicBackend>, event: MusicEvent) {
        if self.closing.load(Ordering::SeqCst)
            || !Arc::ptr_eq(node, &self.node)
            || event.guild_id() != self.guild_id
        {
            return;
        }
        {
            let shared = self.shared();
            if shared.recovery_required {
                return;
            }
            if !matches!(event, MusicEvent::PlayerUpdate { .. } | MusicEvent::WebSocketClosed { .. })
            {
                // Ignore events for playbacks we never started.
                let marker_value = event
                    .track()
                    .and_then(|track| track.user_data.get(PLAYBACK_MARKER))
                    .and_then(|value| value.as_str())
                    .map(str::to_owned);
                let expected = format!("{}:{}", shared.playback_prefix, shared.sequence);
                let accepted = marker_value
                    .as_deref()
                    .map(|value| {
                        Some(value) == shared.playback_id.as_deref() || value == expected
                    })
                    .unwrap_or(false);
                if !accepted {
                    return;
                }
            }
        }
        let _ = self.tx.unbounded_send(Op::Event(event));
    }

    /// Wait until every queued control and event has been handled.
    pub async fn drain(&self) {
        let (reply, rx) = oneshot::channel();
        if self.tx.unbounded_send(Op::Drain(reply)).is_err() {
            return;
        }
        let _ = rx.await;
    }

    /// Destroy the remote player and release its slot on the manager.
    /// Returns the cleanup failures, if any.
    pub async fn destroy(&self) -> Vec<MusicError> {
        if self.closing.swap(true, Ordering::SeqCst) {
            return Vec::new();
        }
        let (reply, rx) = oneshot::channel();
        if self.tx.unbounded_send(Op::Destroy(reply)).is_err() {
            return Vec::new();
        }
        match rx.await {
            Ok(Ok(failures)) => failures,
            _ => Vec::new(),
        }
    }
}

fn position_of(shared: &SharedState) -> i64 {
    let (Some(playing), Some(state)) = (&shared.current, &shared.state) else {
        return 0;
    };
    let speed = shared
        .filters
        .timescale
        .as_ref()
        .map(|t| t.speed.unwrap_or(1.0) * t.rate.unwrap_or(1.0))
        .unwrap_or(1.0);
    let elapsed = if shared.paused || !state.connected || shared.recovery_required {
        0.0
    } else {
        shared.state_received_at.elapsed().as_secs_f64() * 1000.0 * speed
    };
    let position = (state.position as f64 + elapsed).max(0.0);
    if playing.info.is_stream {
        position as i64
    } else {
        position.min(playing.info.length as f64) as i64
    }
}

fn new_prefix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}{:x}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn validate_track(track: &MusicTrack) -> MusicResult<()> {
    if track.encoded.is_empty() {
        return Err(MusicError::new(
            "MUSIC_TRACK",
            "Invalid track: encoded and audio metadata are required.",
        ));
    }
    Ok(())
}

struct AdvanceTarget {
    track: MusicTrack,
    dequeue: bool,
    requeue: Option<MusicTrack>,
}

struct PlayerActor {
    player: Arc<MusicPlayer>,
    rx: mpsc::UnboundedReceiver<Op>,
    advance_target: Option<AdvanceTarget>,
    playback_id: Option<String>,
    playback_prefix: String,
    sequence: u64,
    voice_state: Option<MusicVoiceState>,
    session_version: u64,
    pending_events: Vec<String>,
    voice_joined: bool,
    node: Arc<dyn MusicBackend>,
    voice: Option<Arc<dyn MusicVoiceAdapter>>,
    audio: Option<Arc<dyn MusicAudioOutput>>,
    on_error: Arc<dyn Fn(MusicError) + Send + Sync>,
    release: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl PlayerActor {
    async fn run(mut self) {
        while let Some(op) = self.rx.next().await {
            match op {
                Op::Play(reply) => {
                    let _ = reply.send(self.handle_play().await);
                    self.release_pending();
                }
                Op::Skip(reply) => {
                    let _ = reply.send(self.handle_skip().await);
                    self.release_pending();
                }
                Op::Stop {
                    clear_queue,
                    reply,
                } => {
                    let _ = reply.send(self.handle_stop(clear_queue).await);
                    self.release_pending();
                }
                Op::Pause { paused, reply } => {
                    let _ = reply.send(self.handle_pause(paused).await);
                    self.release_pending();
                }
                Op::SetVolume { volume, reply } => {
                    let _ = reply.send(self.handle_volume(volume).await);
                    self.release_pending();
                }
                Op::Seek { position, reply } => {
                    let _ = reply.send(self.handle_seek(position).await);
                    self.release_pending();
                }
                Op::SetFilters { filters, reply } => {
                    let _ = reply.send(self.handle_filters(filters).await);
                    self.release_pending();
                }
                Op::Connect { channel_id, reply } => {
                    let _ = reply.send(self.handle_connect(channel_id).await);
                    self.release_pending();
                }
                Op::Recover(reply) => {
                    let _ = reply.send(self.handle_recover().await);
                }
                Op::Event(event) => self.handle_event(event).await,
                Op::AudioState { connected, error } => self.handle_audio_state(connected, error),
                Op::Drain(reply) => {
                    let _ = reply.send(());
                }
                Op::Destroy(reply) => {
                    let failures = self.handle_destroy().await;
                    if let Some(release) = self.release.lock().unwrap().take() {
                        release();
                    }
                    let _ = reply.send(Ok(failures));
                    return;
                }
            }
        }
    }

    fn release_pending(&self) {
        self.player.pending.fetch_sub(1, Ordering::SeqCst);
    }

    fn shared(&self) -> std::sync::RwLockReadGuard<'_, SharedState> {
        self.player.shared.read().unwrap()
    }

    fn shared_write(&self) -> std::sync::RwLockWriteGuard<'_, SharedState> {
        self.player.shared.write().unwrap()
    }

    fn accept_state(&self, state: MusicPlayerState) {
        let mut shared = self.shared_write();
        let mut state = state;
        if self.audio.is_some() {
            state.connected = shared.audio_connected;
        }
        shared.state = Some(state);
        shared.state_received_at = Instant::now();
    }

    fn guard(&self) -> MusicResult<()> {
        let shared = self.player.shared();
        if shared.recovery_required {
            return Err(MusicError::new(
                "MUSIC_RECOVERY_REQUIRED",
                "Wait for player recovery or call recover() after fixing the failure.",
            ));
        }
        Ok(())
    }

    async fn update(&self, update: MusicPlayerUpdate) -> MusicResult<MusicPlayerState> {
        let version = self.session_version;
        let remote = self
            .node
            .update_player(&self.player.guild_id, update, false)
            .await?;
        if version != self.session_version {
            return Err(MusicError::new(
                "MUSIC_SESSION_CHANGED",
                "The session changed during control; local state was preserved.",
            ));
        }
        if remote.guild_id != self.player.guild_id {
            return Err(MusicError::new(
                "MUSIC_STATE",
                "The backend returned an invalid player state.",
            ));
        }
        Ok(remote.state)
    }

    fn commit_queue(&mut self, track: &MusicTrack, dequeue: bool, requeue: Option<MusicTrack>) {
        let mut tracks = self.player.tracks.lock().unwrap();
        if dequeue {
            if tracks.peek() == Some(track) {
                tracks.dequeue();
            } else if let Some(index) = tracks.index_of(track) {
                let _ = tracks.remove(index, 1);
            }
        }
        if let Some(requeue) = requeue {
            if tracks.len() < tracks.capacity() {
                let _ = tracks.enqueue(requeue);
            }
        }
    }

    async fn start(
        &mut self,
        track: MusicTrack,
        dequeue: bool,
        requeue: Option<MusicTrack>,
    ) -> MusicResult<MusicTrack> {
        self.sequence += 1;
        let playback_id = format!("{}:{}", self.playback_prefix, self.sequence);
        let mut user_data = track.user_data.clone();
        user_data.insert(
            PLAYBACK_MARKER.to_string(),
            serde_json::Value::String(playback_id.clone()),
        );
        // Publish the in-flight marker before the PATCH lands: Lavalink may
        // send events for the new playback while the request is in flight.
        self.shared_write().sequence = self.sequence;
        let state = match self
            .update(MusicPlayerUpdate {
                track: Some(UpdateTrack::Encoded {
                    encoded: Some(track.encoded.clone()),
                    user_data: Some(user_data),
                }),
                paused: Some(false),
                end_time: Some(None),
                ..Default::default()
            })
            .await
        {
            Ok(state) => state,
            Err(error) => {
                self.sequence -= 1;
                self.shared_write().sequence = self.sequence;
                return Err(error);
            }
        };
        self.commit_queue(&track, dequeue, requeue);
        let mut shared = self.shared_write();
        shared.current = Some(track.clone());
        shared.playback_id = Some(playback_id.clone());
        shared.sequence = self.sequence;
        shared.paused = false;
        drop(shared);
        self.advance_target = None;
        self.playback_id = Some(playback_id);
        self.accept_state(state);
        Ok(track)
    }

    async fn handle_play(&mut self) -> MusicResult<Option<MusicTrack>> {
        self.guard()?;
        if let Some(playing) = self.shared().current.clone() {
            return Ok(Some(playing));
        }
        if let Some(target) = self.advance_target.take() {
            return self
                .start(target.track, target.dequeue, target.requeue)
                .await
                .map(Some);
        }
        let next = self.player.tracks.lock().unwrap().peek().cloned();
        match next {
            Some(track) => self.start(track, true, None).await.map(Some),
            None => Ok(None),
        }
    }

    async fn handle_skip(&mut self) -> MusicResult<Option<MusicTrack>> {
        self.guard()?;
        let next = self.player.tracks.lock().unwrap().peek().cloned();
        if let Some(track) = next {
            return self.start(track, true, None).await.map(Some);
        }
        self.update(MusicPlayerUpdate {
            track: Some(UpdateTrack::Encoded {
                encoded: None,
                user_data: None,
            }),
            ..Default::default()
        })
        .await?;
        self.advance_target = None;
        self.playback_id = None;
        let mut shared = self.shared_write();
        shared.current = None;
        shared.playback_id = None;
        shared.paused = false;
        Ok(None)
    }

    async fn handle_stop(&mut self, clear_queue: bool) -> MusicResult<()> {
        self.guard()?;
        self.update(MusicPlayerUpdate {
            track: Some(UpdateTrack::Encoded {
                encoded: None,
                user_data: None,
            }),
            ..Default::default()
        })
        .await?;
        self.advance_target = None;
        self.playback_id = None;
        let mut shared = self.shared_write();
        shared.current = None;
        shared.playback_id = None;
        shared.paused = false;
        drop(shared);
        if clear_queue {
            self.player.tracks.lock().unwrap().clear();
        }
        Ok(())
    }

    async fn handle_pause(&mut self, paused: bool) -> MusicResult<()> {
        let state = self
            .update(MusicPlayerUpdate {
                paused: Some(paused),
                ..Default::default()
            })
            .await?;
        self.shared_write().paused = paused;
        self.accept_state(state);
        Ok(())
    }

    async fn handle_volume(&mut self, volume: i64) -> MusicResult<()> {
        self.guard()?;
        if !(0..=1000).contains(&volume) {
            return Err(MusicError::new(
                "MUSIC_VOLUME",
                "Volume must be an integer between 0 and 1000.",
            ));
        }
        let state = self
            .update(MusicPlayerUpdate {
                volume: Some(volume),
                ..Default::default()
            })
            .await?;
        self.shared_write().volume = volume;
        self.accept_state(state);
        Ok(())
    }

    async fn handle_seek(&mut self, position: i64) -> MusicResult<()> {
        self.guard()?;
        {
            let shared = self.player.shared();
            let valid = match &shared.current {
                Some(playing) => {
                    playing.info.is_seekable
                        && position >= 0
                        && position <= playing.info.length as i64
                }
                None => false,
            };
            if !valid {
                return Err(MusicError::new(
                    "MUSIC_SEEK",
                    "Invalid position or track does not support seeking.",
                ));
            }
        }
        let state = self
            .update(MusicPlayerUpdate {
                position: Some(position),
                ..Default::default()
            })
            .await?;
        self.accept_state(state);
        Ok(())
    }

    async fn handle_filters(&mut self, filters: MusicFilters) -> MusicResult<()> {
        self.guard()?;
        if let Some(timescale) = &filters.timescale {
            for value in [timescale.speed, timescale.rate, timescale.pitch]
                .into_iter()
                .flatten()
            {
                if !(value.is_finite() && value > 0.0) {
                    return Err(MusicError::new(
                        "MUSIC_FILTERS",
                        "timescale fields must be positive and finite.",
                    ));
                }
            }
            let product = timescale.speed.unwrap_or(1.0) * timescale.rate.unwrap_or(1.0);
            if !product.is_finite() {
                return Err(MusicError::new(
                    "MUSIC_FILTERS",
                    "The product of speed and rate must be finite.",
                ));
            }
        }
        let state = self
            .update(MusicPlayerUpdate {
                filters: Some(filters.clone()),
                ..Default::default()
            })
            .await?;
        self.shared_write().filters = filters;
        self.accept_state(state);
        Ok(())
    }

    async fn handle_connect(&mut self, channel_id: String) -> MusicResult<()> {
        self.guard()?;
        if self.voice.is_none() && self.audio.is_none() {
            return Err(MusicError::new(
                "MUSIC_VOICE_ADAPTER",
                "Configure an audio output or a compatible voice adapter.",
            ));
        }
        if channel_id.trim().is_empty() || channel_id.len() > 128 {
            return Err(MusicError::new(
                "MUSIC_CHANNEL",
                "Provide a valid voice channel.",
            ));
        }
        if let Some(audio) = self.audio.clone() {
            let channel = self.shared().channel.clone();
            if let Some(channel) = &channel {
                if channel != &channel_id {
                    return Err(MusicError::new(
                        "MUSIC_CHANNEL",
                        "Destroy the player before changing its voice channel.",
                    ));
                }
            }
            let on_state = self.audio_state_callback();
            audio
                .connect(
                    self.node.clone(),
                    &self.player.guild_id,
                    &channel_id,
                    on_state,
                )
                .await?;
            self.voice_joined = true;
            match self.update(MusicPlayerUpdate::default()).await {
                Ok(state) => {
                    self.shared_write().channel = Some(channel_id);
                    self.accept_state(state);
                    Ok(())
                }
                Err(error) => {
                    let cleanup = audio.disconnect(&self.player.guild_id).await;
                    self.voice_joined = false;
                    match cleanup {
                        Ok(()) => Err(error),
                        Err(cleanup_error) => Err(MusicError::new(
                            "MUSIC_CONNECT",
                            format!(
                                "The connection failed ({}) and the audio output could not be closed ({}).",
                                error, cleanup_error
                            ),
                        )),
                    }
                }
            }
        } else {
            let voice = self.voice.as_ref().unwrap();
            let joined = voice.join(&self.player.guild_id, &channel_id).await?;
            self.voice_joined = true;
            let snapshot = validate_voice(&joined, &channel_id)?;
            match self
                .update(MusicPlayerUpdate {
                    voice: Some(snapshot.clone()),
                    ..Default::default()
                })
                .await
            {
                Ok(state) => {
                    self.voice_state = Some(snapshot);
                    self.shared_write().channel = Some(channel_id);
                    self.accept_state(state);
                    Ok(())
                }
                Err(error) => {
                    self.shared_write().channel = None;
                    let cleanup = voice.leave(&self.player.guild_id).await;
                    self.voice_joined = false;
                    match cleanup {
                        Ok(()) => Err(error),
                        Err(cleanup_error) => Err(MusicError::new(
                            "MUSIC_CONNECT",
                            format!(
                                "The connection failed ({}) and voice could not be left ({}).",
                                error, cleanup_error
                            ),
                        )),
                    }
                }
            }
        }
    }

    async fn handle_recover(&mut self) -> MusicResult<()> {
        {
            let shared = self.player.shared();
            if !shared.recovery_required {
                return Ok(());
            }
        }
        if self.node.available() == Some(false) {
            return Err(MusicError::new(
                "MUSIC_NODE_UNAVAILABLE",
                "The music node is unavailable.",
            ));
        }
        let version = self.session_version;
        let channel = self.shared().channel.clone();
        if let (Some(channel), Some(audio)) = (&channel, &self.audio) {
            let on_state = self.audio_state_callback();
            audio
                .connect(self.node.clone(), &self.player.guild_id, channel, on_state)
                .await?;
        }
        let voice = match (&channel, &self.voice) {
            (Some(channel), Some(voice)) => {
                let joined = voice.join(&self.player.guild_id, channel).await?;
                Some(validate_voice(&joined, channel)?)
            }
            _ => self.voice_state.clone(),
        };
        if version != self.session_version {
            return Err(MusicError::new(
                "MUSIC_SESSION_CHANGED",
                "The session changed during recovery.",
            ));
        }
        let playing = self.shared().current.clone();
        let advance = if playing.is_none() {
            self.advance_target.take()
        } else {
            None
        };
        let track = playing.or_else(|| advance.as_ref().map(|a| a.track.clone()));
        self.sequence += 1;
        let playback_id = track
            .as_ref()
            .map(|_| format!("{}:{}", self.playback_prefix, self.sequence));
        let mut update = MusicPlayerUpdate {
            volume: Some(self.shared().volume),
            paused: Some(self.shared().paused),
            filters: Some(self.shared().filters.clone()),
            end_time: Some(None),
            voice,
            ..Default::default()
        };
        match &track {
            Some(track) => {
                let mut user_data = track.user_data.clone();
                if let Some(playback_id) = &playback_id {
                    user_data.insert(
                        PLAYBACK_MARKER.to_string(),
                        serde_json::Value::String(playback_id.clone()),
                    );
                }
                update.track = Some(UpdateTrack::Encoded {
                    encoded: Some(track.encoded.clone()),
                    user_data: Some(user_data),
                });
                if track.info.is_seekable && !track.info.is_stream {
                    update.position = Some(if advance.is_some() {
                        0
                    } else {
                        self.player.position()
                    });
                }
            }
            None => {
                update.track = Some(UpdateTrack::Encoded {
                    encoded: None,
                    user_data: None,
                });
            }
        }
        let state = self.update(update).await?;
        if let Some(advance) = advance {
            self.commit_queue(&advance.track, advance.dequeue, advance.requeue);
        }
        {
            let mut shared = self.shared_write();
            shared.current = track.clone();
            shared.playback_id = playback_id.clone();
            shared.sequence = self.sequence;
            shared.recovery_required = false;
        }
        self.playback_id = playback_id;
        self.accept_state(state);
        self.player.emit(PlayerEvent::Recovered);
        Ok(())
    }

    async fn handle_event(&mut self, event: MusicEvent) {
        match event {
            MusicEvent::PlayerUpdate { state, .. } => {
                self.accept_state(state);
            }
            MusicEvent::TrackStart { track, .. } => {
                if !self.matches_playback(&track) {
                    return;
                }
                if !self.dedup("TrackStartEvent", &track) {
                    return;
                }
                self.player.emit(PlayerEvent::TrackStart(track));
            }
            MusicEvent::TrackException {
                track, exception, ..
            } => {
                if !self.matches_playback(&track) || !self.dedup("TrackExceptionEvent", &track) {
                    return;
                }
                let error = Arc::new(MusicError::new(
                    "MUSIC_TRACK_EXCEPTION",
                    exception
                        .message
                        .clone()
                        .unwrap_or_else(|| exception.cause.clone()),
                ));
                self.player.emit(PlayerEvent::PlayerError(error.clone()));
                (self.on_error)((*error).clone());
            }
            MusicEvent::TrackStuck { track, .. } => {
                if !self.matches_playback(&track) || !self.dedup("TrackStuckEvent", &track) {
                    return;
                }
                let error = Arc::new(MusicError::new(
                    "MUSIC_TRACK_STUCK",
                    "The track stopped providing audio.",
                ));
                self.player.emit(PlayerEvent::PlayerError(error.clone()));
                (self.on_error)((*error).clone());
            }
            MusicEvent::WebSocketClosed { code, .. } => {
                if !self.dedup_key("WebSocketClosedEvent:voice") {
                    return;
                }
                self.player.emit(PlayerEvent::VoiceClose(code));
            }
            MusicEvent::TrackEnd {
                track, reason, ..
            } => {
                if !self.matches_playback(&track) || !self.dedup("TrackEndEvent", &track) {
                    return;
                }
                let previous = self.shared().current.clone();
                let Some(previous) = previous else { return };
                self.player.emit(PlayerEvent::TrackEnd(previous.clone(), reason));
                if reason != TrackEndReason::Finished && reason != TrackEndReason::LoadFailed {
                    if reason != TrackEndReason::Replaced {
                        self.shared_write().current = None;
                        self.shared_write().playback_id = None;
                        self.playback_id = None;
                    }
                    return;
                }
                self.shared_write().current = None;
                self.shared_write().playback_id = None;
                self.playback_id = None;
                let repeat = self.shared().repeat;
                if repeat == LoopMode::Track && reason == TrackEndReason::Finished {
                    if let Err(error) = self.start(previous.clone(), false, None).await {
                        self.fail(error);
                    }
                    return;
                }
                let next = self.player.tracks.lock().unwrap().peek().cloned();
                let repeat_queue = repeat == LoopMode::Queue && reason == TrackEndReason::Finished;
                if let Some(next) = next {
                    let requeue = repeat_queue.then(|| previous.clone());
                    let result = self.start(next, true, requeue).await;
                    if let Err(error) = result {
                        self.fail(error);
                    }
                } else if repeat_queue {
                    let result = self.start(previous, false, None).await;
                    if let Err(error) = result {
                        self.fail(error);
                    }
                } else {
                    self.player.emit(PlayerEvent::QueueEnd);
                }
            }
        }
    }

    fn matches_playback(&self, track: &MusicTrack) -> bool {
        let shared = self.player.shared();
        let marker = track
            .user_data
            .get(PLAYBACK_MARKER)
            .and_then(|value| value.as_str());
        marker.is_some() && marker == shared.playback_id.as_deref()
    }

    fn dedup_key(&mut self, key: &str) -> bool {
        if self.pending_events.iter().any(|k| k == key) {
            return false;
        }
        self.pending_events.push(key.to_string());
        // Bound the dedup memory like yuzuha's pendingEvents set lifecycle.
        if self.pending_events.len() > 256 {
            self.pending_events.remove(0);
        }
        true
    }

    fn dedup(&mut self, kind: &str, track: &MusicTrack) -> bool {
        let marker = track
            .user_data
            .get(PLAYBACK_MARKER)
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string();
        self.dedup_key(&format!("{kind}:{marker}"))
    }

    fn handle_audio_state(&mut self, connected: bool, error: Option<MusicError>) {
        let position = self.player.position();
        {
            let mut shared = self.shared_write();
            shared.audio_connected = connected;
            if let Some(mut state) = shared.state {
                state.position = position;
                state.connected = connected;
                shared.state = Some(state);
                shared.state_received_at = Instant::now();
            }
        }
        if let Some(error) = error {
            let error = Arc::new(error);
            self.player.emit(PlayerEvent::PlayerError(error.clone()));
            (self.on_error)((*error).clone());
        }
    }

    fn audio_state_callback(&self) -> Arc<dyn Fn(bool, Option<MusicError>) + Send + Sync> {
        let player = self.player.clone();
        Arc::new(move |connected, error| {
            let _ = player.tx.unbounded_send(Op::AudioState { connected, error });
        })
    }

    fn fail(&self, error: MusicError) {
        let error = Arc::new(error);
        self.player.emit(PlayerEvent::PlayerError(error.clone()));
        (self.on_error)((*error).clone());
    }

    async fn handle_destroy(&mut self) -> Vec<MusicError> {
        let mut failures = Vec::new();
        if let Err(error) = self
            .node
            .destroy_player(&self.player.guild_id)
            .await
        {
            failures.push(error);
        }
        if self.voice_joined {
            if let Some(voice) = &self.voice {
                if let Err(error) = voice.leave(&self.player.guild_id).await {
                    failures.push(error);
                }
            }
            if let Some(audio) = &self.audio {
                if let Err(error) = audio.disconnect(&self.player.guild_id).await {
                    failures.push(error);
                }
            }
        }
        self.player.tracks.lock().unwrap().clear();
        {
            let mut shared = self.shared_write();
            shared.current = None;
            shared.playback_id = None;
            shared.channel = None;
        }
        self.player.listeners.lock().unwrap().clear();
        failures
    }
}

fn validate_voice(voice: &MusicVoiceState, channel_id: &str) -> MusicResult<MusicVoiceState> {
    if voice.token.is_empty()
        || voice.endpoint.is_empty()
        || voice.session_id.is_empty()
        || (voice.channel_id.is_some() && voice.channel_id.as_deref() != Some(channel_id))
    {
        return Err(MusicError::new(
            "MUSIC_VOICE_STATE",
            "The adapter must return a valid token, endpoint and sessionId for the requested channel.",
        ));
    }
    let mut snapshot = voice.clone();
    snapshot.channel_id = Some(channel_id.to_string());
    Ok(snapshot)
}
