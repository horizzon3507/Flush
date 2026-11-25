//! Voice runtime (mirrors Yuzuha `services/voice-states.ts`, `ports/voice.ts`
//! and `infrastructure/fluxer/voice.ts`):
//!
//! - [`VoiceStateTracker`]: in-memory voice states per guild/user, fed by
//!   `VOICE_STATE_UPDATE`, `VOICE_STATE_ACK` (canonical state), `GUILD_CREATE`
//!   voice snapshots, `GUILD_DELETE` and `CHANNEL_DELETE`.
//! - [`Voice`]: a driver-backed connection manager. The audio itself comes
//!   from a [`VoiceDriver`] implementation (Flush does not ship one); the
//!   manager validates the channel, dedups joins, normalizes events into
//!   sanitized [`VoiceEvent`]s, and caps speaker bursts at 20 events/second.
//!   Raw driver payloads never reach listeners: unknown error codes become
//!   `VOICE_STATE` and the message is always the fixed voice error text, so
//!   tokens and endpoints in driver payloads are not leaked.

use std::collections::HashMap;
use std::future::Future;
use std::io::Read;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use flush_core::{Channel, Snowflake, VoiceState};
use futures::future::{BoxFuture, Shared};
use futures::FutureExt;

/// Normalized voice state of a member connected to a voice channel
/// (`VoiceStateInfo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceStateInfo {
    pub guild_id: Snowflake,
    pub channel_id: Snowflake,
    pub user_id: Snowflake,
    pub session_id: String,
    pub deaf: bool,
    pub mute: bool,
    pub self_deaf: bool,
    pub self_mute: bool,
    pub self_stream: bool,
    pub self_video: bool,
    pub suppress: bool,
    /// Request-to-speak timestamp in Unix ms, when the gateway sent a
    /// numeric (or numeric-string) value.
    pub request_to_speak_timestamp: Option<i64>,
}

fn speak_timestamp_ms(value: Option<&serde_json::Value>) -> Option<i64> {
    match value? {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

#[derive(Default)]
struct TrackerInner {
    guilds: HashMap<Snowflake, HashMap<Snowflake, VoiceStateInfo>>,
    total: usize,
}

/// In-memory tracker of voice states by guild and user. Cheap to clone:
/// clones share the same underlying map (`bot.voice_states()` and
/// `CommandCtx::voice_states` see the same data).
#[derive(Clone, Default)]
pub struct VoiceStateTracker {
    inner: Arc<Mutex<TrackerInner>>,
}

impl VoiceStateTracker {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, TrackerInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Total number of tracked voice states across every guild.
    pub fn size(&self) -> usize {
        self.lock().total
    }

    /// A copy of one user's voice state in the guild.
    pub fn get(&self, guild_id: Snowflake, user_id: Snowflake) -> Option<VoiceStateInfo> {
        self.lock()
            .guilds
            .get(&guild_id)
            .and_then(|m| m.get(&user_id))
            .cloned()
    }

    /// Every tracked voice state of the guild.
    pub fn guild(&self, guild_id: Snowflake) -> Vec<VoiceStateInfo> {
        self.lock()
            .guilds
            .get(&guild_id)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Tracked voice states currently in one channel of the guild.
    pub fn channel(&self, guild_id: Snowflake, channel_id: Snowflake) -> Vec<VoiceStateInfo> {
        self.lock()
            .guilds
            .get(&guild_id)
            .map(|m| {
                m.values()
                    .filter(|s| s.channel_id == channel_id)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The user's current voice channel in the guild (`None` when out of voice).
    pub fn user_channel(&self, guild_id: Snowflake, user_id: Snowflake) -> Option<Snowflake> {
        self.lock()
            .guilds
            .get(&guild_id)
            .and_then(|m| m.get(&user_id))
            .map(|s| s.channel_id)
    }

    /// Clears the states of one guild, or the whole tracker with `None`.
    pub fn clear(&self, guild_id: Option<Snowflake>) {
        let mut inner = self.lock();
        match guild_id {
            Some(id) => {
                if let Some(members) = inner.guilds.remove(&id) {
                    inner.total -= members.len();
                }
            }
            None => {
                inner.guilds.clear();
                inner.total = 0;
            }
        }
    }

    /// Drops every state bound to a deleted channel; `guild_id` narrows the
    /// search when the channel's guild is known.
    pub fn clear_channel(&self, channel_id: Snowflake, guild_id: Option<Snowflake>) {
        let mut inner = self.lock();
        match guild_id {
            Some(id) => evict_channel(&mut inner, id, channel_id),
            None => {
                let ids: Vec<Snowflake> = inner.guilds.keys().copied().collect();
                for id in ids {
                    evict_channel(&mut inner, id, channel_id);
                }
            }
        }
    }

    /// Replaces all states of a guild from a snapshot (`GUILD_CREATE`
    /// `voice_states`, the SDK's `voiceStatesSync` source).
    pub fn sync_guild(&self, guild_id: Snowflake, states: &[VoiceState]) {
        self.clear(Some(guild_id));
        for raw in states {
            self.apply_with_guild(raw, Some(guild_id));
        }
    }

    /// Applies a single `VOICE_STATE_UPDATE` (or a `VOICE_STATE_ACK`
    /// canonical state).
    pub fn apply(&self, raw: &VoiceState) {
        self.apply_with_guild(raw, None);
    }

    /// Like [`Self::apply`] with a guild fallback for payloads whose
    /// `guild_id` is absent.
    pub fn apply_with_guild(&self, raw: &VoiceState, fallback_guild: Option<Snowflake>) {
        let Some(guild_id) = raw.guild_id.or(fallback_guild) else {
            return;
        };
        let user_id = raw.user_id;
        let mut inner = self.lock();
        match raw.channel_id {
            None => {
                let (removed, emptied) = match inner.guilds.get_mut(&guild_id) {
                    Some(members) => (members.remove(&user_id).is_some(), members.is_empty()),
                    None => (false, false),
                };
                if removed {
                    inner.total -= 1;
                }
                if emptied {
                    inner.guilds.remove(&guild_id);
                }
            }
            Some(channel_id) => {
                let is_new = {
                    let members = inner.guilds.entry(guild_id).or_default();
                    let is_new = !members.contains_key(&user_id);
                    members.insert(
                    user_id,
                    VoiceStateInfo {
                        guild_id,
                        channel_id,
                        user_id,
                        session_id: raw.session_id.clone().unwrap_or_default(),
                        deaf: raw.deaf,
                        mute: raw.mute,
                        self_deaf: raw.self_deaf,
                        self_mute: raw.self_mute,
                        self_stream: raw.self_stream,
                        self_video: raw.self_video,
                        suppress: raw.suppress,
                        request_to_speak_timestamp: speak_timestamp_ms(
                            raw.request_to_speak_timestamp.as_ref(),
                        ),
                    },
                );
                is_new
                };
                if is_new {
                    inner.total += 1;
                }
            }
        }
    }
}

fn evict_channel(inner: &mut TrackerInner, guild_id: Snowflake, channel_id: Snowflake) {
    let mut remove_guild = false;
    if let Some(members) = inner.guilds.get_mut(&guild_id) {
        let before = members.len();
        members.retain(|_, s| s.channel_id != channel_id);
        inner.total -= before - members.len();
        remove_guild = members.is_empty();
    }
    if remove_guild {
        inner.guilds.remove(&guild_id);
    }
}

/// Names of the normalized voice events, as exposed by Yuzuha
/// (`voiceReady`, `voiceDisconnect`, `voiceError`, `voiceServerLeave`,
/// `voiceSpeakerStart`, `voiceSpeakerStop`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VoiceEventName {
    Ready,
    Disconnect,
    Error,
    ServerLeave,
    SpeakerStart,
    SpeakerStop,
}

impl VoiceEventName {
    /// The Yuzuha event name (`"voiceReady"` and so on).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "voiceReady",
            Self::Disconnect => "voiceDisconnect",
            Self::Error => "voiceError",
            Self::ServerLeave => "voiceServerLeave",
            Self::SpeakerStart => "voiceSpeakerStart",
            Self::SpeakerStop => "voiceSpeakerStop",
        }
    }
}

impl std::fmt::Display for VoiceEventName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A sanitized voice event. Driver payloads are reduced to ids and a
/// whitelist error code: connection tokens, endpoints and session ids never
/// reach listeners.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceEvent {
    pub name: VoiceEventName,
    pub guild_id: Snowflake,
    pub channel_id: Snowflake,
    pub user_id: Option<Snowflake>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

/// The only error codes a driver may leak (`VOICE_STATE` otherwise).
const KNOWN_NET_CODES: &[&str] = &["ECONNRESET", "ETIMEDOUT", "ECONNREFUSED", "EPIPE"];

/// Fixed message for `voiceError`; driver messages may carry credentials.
pub const VOICE_ERROR_MESSAGE: &str = "Voice connection failed.";

/// Events a [`VoiceConnection`] reports to its subscribers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceConnEvent {
    Ready,
    Disconnect,
    /// `code` is the driver's own error code (an `errno`-style string).
    Error { code: Option<String> },
    /// The server dropped the connection (LiveKit `serverLeave`).
    ServerLeave,
    SpeakerStart { participant_id: String },
    SpeakerStop { participant_id: String },
}

/// Callback a driver invokes when its connection emits an event.
pub type VoiceConnCallback = Arc<dyn Fn(VoiceConnEvent) + Send + Sync>;

/// An audio source for [`VoiceConnection::play`]: a URI the driver
/// resolves, or a byte stream.
pub enum VoiceSource {
    Uri(String),
    Reader(Box<dyn Read + Send>),
}

impl std::fmt::Debug for VoiceSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Uri(u) => f.debug_tuple("Uri").field(u).finish(),
            Self::Reader(_) => f.debug_tuple("Reader").field(&"..").finish(),
        }
    }
}

impl From<String> for VoiceSource {
    fn from(s: String) -> Self {
        Self::Uri(s)
    }
}
impl From<&str> for VoiceSource {
    fn from(s: &str) -> Self {
        Self::Uri(s.to_owned())
    }
}
impl From<Box<dyn Read + Send>> for VoiceSource {
    fn from(r: Box<dyn Read + Send>) -> Self {
        Self::Reader(r)
    }
}

/// Errors of the voice runtime. [`VoiceError::code`] returns the Yuzuha
/// error code (`VOICE_CLOSED`, `VOICE_CHANNEL`, ...).
#[derive(Debug, Clone, thiserror::Error)]
pub enum VoiceError {
    #[error("voice is closed")]
    Closed,
    #[error("the given channel is not a voice channel")]
    ChannelNotVoice,
    #[error("the voice channel belongs to another guild")]
    GuildMismatch,
    #[error("leave the current channel before joining another in the same guild")]
    AlreadyConnected,
    #[error("a voice connection is already pending in this guild")]
    PendingJoin,
    #[error("join a voice channel before playing audio")]
    NotConnected,
    #[error("{0}")]
    Unsupported(String),
    #[error("native volume must be an integer between 0 and 200")]
    Volume,
    #[error("{0}")]
    Dependency(String),
    #[error("REST call failed: {0}")]
    Rest(String),
    #[error("{0}")]
    Driver(String),
}

impl From<flush_rest::RestError> for VoiceError {
    fn from(e: flush_rest::RestError) -> Self {
        Self::Rest(e.to_string())
    }
}

impl VoiceError {
    /// The Yuzuha `YuzuhaError` code for this failure.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Closed => "VOICE_CLOSED",
            Self::ChannelNotVoice | Self::AlreadyConnected | Self::PendingJoin => "VOICE_CHANNEL",
            Self::GuildMismatch => "VOICE_GUILD",
            Self::NotConnected => "VOICE_NOT_CONNECTED",
            Self::Unsupported(_) => "VOICE_UNSUPPORTED",
            Self::Volume => "VOICE_VOLUME",
            Self::Dependency(_) => "VOICE_DEPENDENCY",
            Self::Rest(_) | Self::Driver(_) => "VOICE_STATE",
        }
    }
}

/// Self mute/deaf flags sent to the driver (`updateVoiceState`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SelfVoiceState {
    pub self_mute: bool,
    pub self_deaf: bool,
}

/// A live voice connection produced by a [`VoiceDriver`]
/// (`FluxerVoiceConnection`).
pub trait VoiceConnection: Send + Sync {
    /// Whether audio is currently playing.
    fn playing(&self) -> bool {
        false
    }
    /// Whether video is currently playing.
    fn playing_video(&self) -> bool {
        false
    }
    /// Whether the connection is still usable.
    fn is_connected(&self) -> bool {
        true
    }
    /// The current native volume (0–200), when the driver reports it.
    fn get_volume(&self) -> Option<u32> {
        None
    }
    /// Plays an audio source until it ends or the driver errors.
    fn play<'a>(
        &'a self,
        source: VoiceSource,
    ) -> Pin<Box<dyn Future<Output = Result<(), VoiceError>> + Send + 'a>>;
    /// Stops audio playback.
    fn stop(&self);
    /// Stops video playback.
    fn stop_video(&self) {}
    /// Sets the native volume (0–200). The default errors with
    /// [`VoiceError::Unsupported`].
    fn set_volume(&self, _volume: u32) -> Result<(), VoiceError> {
        Err(VoiceError::Unsupported(
            "this voice connection has no volume control".into(),
        ))
    }
    /// Registers a connection-event callback; the returned id unsubscribes it.
    fn subscribe(&self, cb: VoiceConnCallback) -> u64;
    /// Removes a callback previously registered with [`Self::subscribe`].
    fn unsubscribe(&self, id: u64);
}

/// Produces [`VoiceConnection`]s (`FluxerVoiceDriver`). The audio backend is
/// supplied by the caller — Flush ships no driver of its own.
pub trait VoiceDriver: Send + Sync {
    /// Joins the bot to the given voice channel.
    fn join(&self, channel: Channel) -> JoinFuture;
    /// Leaves the voice channel in the guild.
    fn leave(&self, guild_id: Snowflake);
    /// Channel-scoped leave. The default returns `false`, and [`Voice`]
    /// falls back to [`Self::leave`].
    fn leave_channel(&self, _channel_id: Snowflake) -> bool {
        false
    }
    /// Updates the bot's self mute/deaf flags. The default errors with
    /// [`VoiceError::Unsupported`].
    fn update_voice_state(
        &self,
        _channel_id: Snowflake,
        _state: SelfVoiceState,
    ) -> Result<(), VoiceError> {
        Err(VoiceError::Unsupported(
            "the driver cannot update the voice state".into(),
        ))
    }
}

type DriverFactory = Box<dyn Fn() -> Result<Arc<dyn VoiceDriver>, VoiceError> + Send + Sync>;

type JoinFuture =
    Pin<Box<dyn Future<Output = Result<Arc<dyn VoiceConnection>, VoiceError>> + Send>>;

type JoinResult = Result<Arc<dyn VoiceConnection>, Arc<VoiceError>>;
type PendingJoin = Arc<Shared<BoxFuture<'static, JoinResult>>>;

struct ConnEntry {
    channel_id: Snowflake,
    connection: Arc<dyn VoiceConnection>,
    /// The conn-event subscription id registered by [`Voice`].
    sub_id: u64,
    state: SelfVoiceState,
}

type VoiceListener = Arc<dyn Fn(&VoiceEvent) + Send + Sync>;

fn lock<'a, T>(m: &'a Mutex<T>) -> std::sync::MutexGuard<'a, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct VoiceInner {
    resolved: Mutex<Option<Arc<dyn VoiceDriver>>>,
    factory: Mutex<Option<DriverFactory>>,
    connections: Mutex<HashMap<Snowflake, ConnEntry>>,
    pending: Mutex<HashMap<Snowflake, (Snowflake, PendingJoin)>>,
    listeners: Mutex<HashMap<u64, VoiceListener>>,
    next_listener: AtomicU64,
    closed: AtomicBool,
    closing: async_std::sync::Mutex<()>,
}

/// A driver-backed voice manager (`FluxerVoice`/`NativeVoice`). Clone to
/// share; every clone manages the same connections.
#[derive(Clone)]
pub struct Voice {
    inner: Arc<VoiceInner>,
}

/// Unsubscribes a [`Voice`] listener on drop (the disposer Yuzuha returns
/// from `subscribe`).
pub struct VoiceEventGuard {
    id: u64,
    voice: Voice,
    active: bool,
}

impl Drop for VoiceEventGuard {
    fn drop(&mut self) {
        if self.active {
            self.active = false;
            self.voice.unsubscribe(self.id);
        }
    }
}

impl Voice {
    /// A manager whose driver is already loaded.
    pub fn new(driver: Arc<dyn VoiceDriver>) -> Self {
        Self::build(Some(driver), None)
    }

    /// A manager that resolves its driver on the first join — the runtime
    /// only asks for the voice backend when a channel is actually joined
    /// (the lazy `@fluxerjs/voice` import of Yuzuha).
    pub fn lazy(
        factory: impl Fn() -> Result<Arc<dyn VoiceDriver>, VoiceError> + Send + Sync + 'static,
    ) -> Self {
        Self::build(None, Some(Box::new(factory)))
    }

    fn build(driver: Option<Arc<dyn VoiceDriver>>, factory: Option<DriverFactory>) -> Self {
        Self {
            inner: Arc::new(VoiceInner {
                resolved: Mutex::new(driver),
                factory: Mutex::new(factory),
                connections: Mutex::new(HashMap::new()),
                pending: Mutex::new(HashMap::new()),
                listeners: Mutex::new(HashMap::new()),
                next_listener: AtomicU64::new(1),
                closed: AtomicBool::new(false),
                closing: async_std::sync::Mutex::new(()),
            }),
        }
    }

    fn driver(&self) -> Result<Arc<dyn VoiceDriver>, VoiceError> {
        let mut resolved = lock(&self.inner.resolved);
        if let Some(d) = resolved.as_ref() {
            return Ok(d.clone());
        }
        let factory = lock(&self.inner.factory).take();
        let Some(factory) = factory else {
            return Err(VoiceError::Dependency(
                "no voice driver is configured; supply one with Voice::new or Voice::lazy".into(),
            ));
        };
        let driver = factory()?;
        *resolved = Some(driver.clone());
        Ok(driver)
    }

    /// Number of active connections.
    pub fn size(&self) -> usize {
        lock(&self.inner.connections).len()
    }

    /// Subscribes to normalized voice events. No-op when closed; the
    /// returned guard unsubscribes on drop.
    pub fn subscribe(
        &self,
        listener: impl Fn(&VoiceEvent) + Send + Sync + 'static,
    ) -> VoiceEventGuard {
        let id = self.inner.next_listener.fetch_add(1, Ordering::Relaxed);
        if !self.inner.closed.load(Ordering::SeqCst) {
            lock(&self.inner.listeners).insert(id, Arc::new(listener));
        }
        VoiceEventGuard {
            id,
            voice: self.clone(),
            active: true,
        }
    }

    /// Removes a listener registered with [`Self::subscribe`].
    pub fn unsubscribe(&self, id: u64) {
        lock(&self.inner.listeners).remove(&id);
    }

    /// The channel the bot is connected to in the guild.
    pub fn channel_id(&self, guild_id: Snowflake) -> Option<Snowflake> {
        lock(&self.inner.connections)
            .get(&guild_id)
            .map(|e| e.channel_id)
    }

    /// The live connection in the guild.
    pub fn get(&self, guild_id: Snowflake) -> Option<Arc<dyn VoiceConnection>> {
        lock(&self.inner.connections)
            .get(&guild_id)
            .map(|e| e.connection.clone())
    }

    fn publish(&self, event: VoiceEvent) {
        if self.inner.closed.load(Ordering::SeqCst) {
            return;
        }
        let listeners: Vec<VoiceListener> =
            lock(&self.inner.listeners).values().cloned().collect();
        for listener in listeners {
            listener(&event);
        }
    }

    /// The guild's live connection, or `Closed`/`NotConnected`.
    fn connection(&self, guild_id: Snowflake) -> Result<(Snowflake, Arc<dyn VoiceConnection>), VoiceError> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(VoiceError::Closed);
        }
        lock(&self.inner.connections)
            .get(&guild_id)
            .map(|e| (e.channel_id, e.connection.clone()))
            .ok_or(VoiceError::NotConnected)
    }

    /// Fetches the channel over REST and joins it (`join(channelID, {guildID})`).
    pub async fn join_id(
        &self,
        rest: &flush_rest::RestClient,
        channel_id: Snowflake,
        guild_id: Option<Snowflake>,
    ) -> Result<Arc<dyn VoiceConnection>, VoiceError> {
        let channel = rest.channel(channel_id).await?;
        self.join(channel, guild_id).await
    }

    /// Joins a resolved voice channel. One connection per guild: rejoining
    /// the same channel returns the live connection, and a different
    /// channel (connected or still pending) errors.
    pub async fn join(
        &self,
        channel: Channel,
        guild_id: Option<Snowflake>,
    ) -> Result<Arc<dyn VoiceConnection>, VoiceError> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(VoiceError::Closed);
        }
        if !channel.is_voice() {
            return Err(VoiceError::ChannelNotVoice);
        }
        let Some(gid) = channel.guild_id else {
            return Err(VoiceError::ChannelNotVoice);
        };
        if let Some(expected) = guild_id {
            if expected != gid {
                return Err(VoiceError::GuildMismatch);
            }
        }
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(VoiceError::Closed);
        }
        {
            let connections = lock(&self.inner.connections);
            if let Some(existing) = connections.get(&gid) {
                if existing.channel_id == channel.id {
                    return Ok(existing.connection.clone());
                }
                return Err(VoiceError::AlreadyConnected);
            }
        }
        let channel_id = channel.id;
        let voice = self.clone();
        let task: PendingJoin = Arc::new(
            async move { voice.do_join(channel).await }
                .boxed()
                .shared(),
        );
        let existing = {
            let mut pending = lock(&self.inner.pending);
            match pending.get(&gid) {
                Some((pending_channel, _)) if *pending_channel != channel_id => {
                    return Err(VoiceError::PendingJoin);
                }
                Some((_, stored)) => Some(stored.clone()),
                None => {
                    pending.insert(gid, (channel_id, task.clone()));
                    None
                }
            }
        };
        if let Some(task) = existing {
            return task.as_ref().clone().await.map_err(|e| (*e).clone());
        }
        // Keep the driver join alive even if every caller drops its future,
        // and clear the pending marker once it settles: a cancelled join
        // must not leave the guild's voice blocked.
        async_std::task::spawn({
            let shared = (*task).clone();
            let task = task.clone();
            let inner = Arc::downgrade(&self.inner);
            async move {
                let _ = shared.await;
                if let Some(inner) = inner.upgrade() {
                    let mut pending = lock(&inner.pending);
                    if matches!(pending.get(&gid), Some((_, t)) if Arc::ptr_eq(t, &task)) {
                        pending.remove(&gid);
                    }
                }
            }
        });
        let result = task.as_ref().clone().await;
        // Drop our pending marker only when it is still the same join.
        let mut pending = lock(&self.inner.pending);
        if let Some((_, stored)) = pending.get(&gid) {
            if Arc::ptr_eq(stored, &task) {
                pending.remove(&gid);
            }
        }
        result.map_err(|e| (*e).clone())
    }

    async fn do_join(&self, channel: Channel) -> JoinResult {
        let driver = self.driver().map_err(Arc::new)?;
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(Arc::new(VoiceError::Closed));
        }
        let gid = channel.guild_id.unwrap_or_default();
        let channel_id = channel.id;
        let connection = driver.join(channel).await.map_err(|error| {
            self.publish(VoiceEvent {
                name: VoiceEventName::Error,
                guild_id: gid,
                channel_id,
                user_id: None,
                error_code: Some(normalize_error_code(&error)),
                error_message: Some(VOICE_ERROR_MESSAGE.to_string()),
            });
            Arc::new(error)
        })?;
        if self.inner.closed.load(Ordering::SeqCst) {
            driver_leave(&driver, gid, channel_id);
            return Err(Arc::new(VoiceError::Closed));
        }

        // One callback carries every connection event, so the driver's own
        // emit order is preserved: a serverLeave emitted before disconnect
        // is still delivered, and nothing arrives after cleanup. The
        // callback holds a weak manager reference: a live Voice owns the
        // connection, and a strong one would keep both alive forever.
        let weak = Arc::downgrade(&self.inner);
        let speaker = Arc::new(Mutex::new(SpeakerWindow::default()));
        let sub_id = connection.subscribe(Arc::new(move |ev| {
            if let Some(inner) = weak.upgrade() {
                Voice { inner }.on_conn_event(gid, channel_id, &speaker, ev);
            }
        }));
        {
            let mut connections = lock(&self.inner.connections);
            if let Some(existing) = connections.get(&gid) {
                // A racing join registered while this one was in flight:
                // hand back per the join contract and leave the extra
                // connection with the driver.
                let result = if existing.channel_id == channel_id {
                    Ok(existing.connection.clone())
                } else {
                    Err(Arc::new(VoiceError::AlreadyConnected))
                };
                drop(connections);
                driver_leave(&driver, gid, channel_id);
                return result;
            }
            connections.insert(
                gid,
                ConnEntry {
                    channel_id,
                    connection: connection.clone(),
                    sub_id,
                    state: SelfVoiceState::default(),
                },
            );
        }
        self.publish(VoiceEvent {
            name: VoiceEventName::Ready,
            guild_id: gid,
            channel_id,
            user_id: None,
            error_code: None,
            error_message: None,
        });
        Ok(connection)
    }

    fn on_conn_event(
        &self,
        guild_id: Snowflake,
        channel_id: Snowflake,
        speaker: &Mutex<SpeakerWindow>,
        ev: VoiceConnEvent,
    ) {
        if let VoiceConnEvent::Disconnect = ev {
            let still_current = lock(&self.inner.connections)
                .get(&guild_id)
                .map(|e| e.channel_id == channel_id)
                .unwrap_or(false);
            if !still_current {
                return;
            }
            self.publish(VoiceEvent {
                name: VoiceEventName::Disconnect,
                guild_id,
                channel_id,
                user_id: None,
                error_code: None,
                error_message: None,
            });
            let removed = lock(&self.inner.connections).remove(&guild_id);
            if let Some(entry) = removed {
                entry.connection.unsubscribe(entry.sub_id);
            }
            return;
        }
        let (name, user_id, error_code, error_message) = match ev {
            // `do_join` publishes `voiceReady` when the connection registers;
            // a driver that also emits ready must not announce it twice.
            VoiceConnEvent::Ready => return,
            VoiceConnEvent::ServerLeave => (VoiceEventName::ServerLeave, None, None, None),
            VoiceConnEvent::Error { code } => (
                VoiceEventName::Error,
                None,
                Some(match code {
                    Some(c) if KNOWN_NET_CODES.contains(&c.as_str()) => c,
                    _ => "VOICE_STATE".to_string(),
                }),
                Some(VOICE_ERROR_MESSAGE.to_string()),
            ),
            VoiceConnEvent::SpeakerStart { participant_id } => {
                match speaker_event(speaker, participant_id) {
                    Some(user) => (VoiceEventName::SpeakerStart, Some(user), None, None),
                    None => return,
                }
            }
            VoiceConnEvent::SpeakerStop { participant_id } => {
                match speaker_event(speaker, participant_id) {
                    Some(user) => (VoiceEventName::SpeakerStop, Some(user), None, None),
                    None => return,
                }
            }
            VoiceConnEvent::Disconnect => unreachable!(),
        };
        self.publish(VoiceEvent {
            name,
            guild_id,
            channel_id,
            user_id,
            error_code,
            error_message,
        });
    }

    /// Updates the bot's self mute/deaf flags, merging with the stored ones
    /// (`updateVoiceState`).
    pub async fn update_voice_state(
        &self,
        guild_id: Snowflake,
        self_mute: Option<bool>,
        self_deaf: Option<bool>,
    ) -> Result<(), VoiceError> {
        let (channel_id, _) = self.connection(guild_id)?;
        let driver = self.driver()?;
        let merged = {
            let connections = lock(&self.inner.connections);
            let current = connections.get(&guild_id).ok_or(VoiceError::NotConnected)?;
            SelfVoiceState {
                self_mute: self_mute.unwrap_or(current.state.self_mute),
                self_deaf: self_deaf.unwrap_or(current.state.self_deaf),
            }
        };
        driver.update_voice_state(channel_id, merged)?;
        if let Some(e) = lock(&self.inner.connections).get_mut(&guild_id) {
            e.state = merged;
        }
        Ok(())
    }

    /// Plays an audio source on the guild's connection. A driver error
    /// event that arrives while `play` is awaited fails the call too.
    pub async fn play(
        &self,
        guild_id: Snowflake,
        source: VoiceSource,
    ) -> Result<(), VoiceError> {
        let (_, connection) = self.connection(guild_id)?;
        let captured = Arc::new(Mutex::new(None::<String>));
        let capture = {
            let captured = captured.clone();
            Arc::new(move |ev: VoiceConnEvent| {
                if let VoiceConnEvent::Error { code } = ev {
                    *captured.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(code.unwrap_or_else(|| "VOICE_STATE".into()));
                }
            }) as VoiceConnCallback
        };
        // RAII: a cancelled `play` still unsubscribes its error listener.
        struct SubGuard<'a> {
            connection: &'a dyn VoiceConnection,
            id: u64,
        }
        impl Drop for SubGuard<'_> {
            fn drop(&mut self) {
                self.connection.unsubscribe(self.id);
            }
        }
        let sub = SubGuard {
            connection: &*connection,
            id: connection.subscribe(capture),
        };
        let result = connection.play(source).await;
        drop(sub);
        result?;
        if let Some(code) = captured.lock().unwrap_or_else(|e| e.into_inner()).take() {
            return Err(VoiceError::Driver(code));
        }
        Ok(())
    }

    /// Stops audio and video on the guild's connection.
    pub fn stop(&self, guild_id: Snowflake) -> Result<(), VoiceError> {
        let (_, connection) = self.connection(guild_id)?;
        connection.stop();
        connection.stop_video();
        Ok(())
    }

    /// Sets the native volume (integer 0–200).
    pub fn set_volume(&self, guild_id: Snowflake, volume: u32) -> Result<(), VoiceError> {
        let (_, connection) = self.connection(guild_id)?;
        if volume > 200 {
            return Err(VoiceError::Volume);
        }
        connection.set_volume(volume)
    }

    /// Leaves the guild's channel: waits out a pending join, stops the
    /// connection and tells the driver. Returns `false` when there was
    /// nothing to leave.
    pub async fn leave(&self, guild_id: Snowflake) -> Result<bool, VoiceError> {
        let pending_task = lock(&self.inner.pending)
            .get(&guild_id)
            .map(|(_, t)| t.clone());
        if let Some(task) = pending_task {
            let _ = task.as_ref().clone().await;
        }
        let Some((channel_id, connection)) = lock(&self.inner.connections)
            .get(&guild_id)
            .map(|e| (e.channel_id, e.connection.clone()))
        else {
            return Ok(false);
        };
        let driver = self.driver()?;
        connection.stop();
        driver_leave(&driver, guild_id, channel_id);
        // Run the disconnect path: publish `voiceDisconnect`, drop the
        // entry and unsubscribe the conn callback.
        self.on_conn_event(
            guild_id,
            channel_id,
            &Mutex::new(SpeakerWindow::default()),
            VoiceConnEvent::Disconnect,
        );
        Ok(true)
    }

    /// Closes the runtime: pending joins are awaited and every connection
    /// is left. Later calls return immediately. Errors from individual
    /// leaves are joined into one message.
    pub async fn close(&self) -> Result<(), VoiceError> {
        if self.inner.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let _guard = self.inner.closing.lock().await;
        lock(&self.inner.listeners).clear();
        let pending: Vec<PendingJoin> = lock(&self.inner.pending)
            .values()
            .map(|(_, t)| t.clone())
            .collect();
        for task in pending {
            let _ = task.as_ref().clone().await;
        }
        let guilds: Vec<Snowflake> = lock(&self.inner.connections).keys().copied().collect();
        let mut failures = Vec::new();
        for guild_id in guilds {
            if let Err(e) = self.leave(guild_id).await {
                failures.push(e.to_string());
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(VoiceError::Driver(format!(
                "failed to close voice: {}",
                failures.join("; ")
            )))
        }
    }
}

fn driver_leave(driver: &Arc<dyn VoiceDriver>, guild_id: Snowflake, channel_id: Snowflake) {
    if !driver.leave_channel(channel_id) {
        driver.leave(guild_id);
    }
}

fn normalize_error_code(error: &VoiceError) -> String {
    match error {
        VoiceError::Driver(code) if KNOWN_NET_CODES.contains(&code.as_str()) => code.clone(),
        _ => "VOICE_STATE".to_string(),
    }
}

/// Per-connection speaker burst window: at most 20 speaker events per
/// rolling second (`speakerCount >= 20` is dropped in Yuzuha).
#[derive(Default)]
struct SpeakerWindow {
    window_start: Option<Instant>,
    count: u32,
}

/// Validates a `participantId` (1–128 decimal digits) and enforces the
/// burst cap; the snowflake is returned for publishing.
fn speaker_event(window: &Mutex<SpeakerWindow>, participant_id: String) -> Option<Snowflake> {
    let valid = !participant_id.is_empty()
        && participant_id.len() <= 128
        && participant_id.bytes().all(|b| b.is_ascii_digit());
    if !valid {
        return None;
    }
    let user_id = participant_id.parse::<u64>().ok().map(Snowflake::new)?;
    let mut w = window.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    if w.window_start.map(|t| now.duration_since(t).as_secs() >= 1) != Some(false) {
        w.window_start = Some(now);
        w.count = 0;
    }
    if w.count >= 20 {
        return None;
    }
    w.count += 1;
    Some(user_id)
}
