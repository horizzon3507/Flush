//! `LavalinkNode`: Lavalink v4 REST + WebSocket client implementing
//! `MusicBackend`, with reconnect, load-track caching and request coalescing.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_std::future::timeout;
use async_std::task;
#[allow(deprecated)]
use async_tungstenite::async_std::connect_async;
use async_tungstenite::tungstenite::client::IntoClientRequest;
use async_tungstenite::tungstenite::http::Request as WsRequest;
use async_tungstenite::tungstenite::Message as WsMessage;
use futures::channel::{mpsc, oneshot};
use futures::future::{AbortHandle, Abortable, BoxFuture, FutureExt, Shared};
use futures::io::AsyncReadExt;
use futures::{SinkExt, StreamExt};
use serde_json::Value;

use crate::backend::{
    BoxMusicFuture, EventListener, LifecycleListener, MusicAudioStream, MusicBackend, MusicResult,
    Subscription,
};
use crate::domain::{
    LavalinkInfo, LavalinkStats, MusicEvent, MusicNodeEvent, MusicPlayerUpdate,
    RemoteMusicPlayer, TrackEndReason, TrackExceptionSeverity, TrackLoadResult,
};
use crate::error::MusicError;

const MAX_ENCODED_LEN: usize = 65_536;
const MAX_AUDIO_CHUNK: usize = 1_048_576;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeState {
    Idle,
    Connecting,
    Ready,
    Disconnected,
    Closed,
}

/// Reconnect policy: exponential backoff from `delay` up to `max_delay`,
/// at most `attempts` retries.
#[derive(Debug, Clone, Copy)]
pub struct ReconnectOptions {
    pub attempts: u32,
    pub delay: Duration,
    pub max_delay: Duration,
}

/// Load-track response cache (LRU with TTL).
#[derive(Debug, Clone, Copy)]
pub struct CacheOptions {
    pub ttl: Duration,
    pub capacity: usize,
    pub max_bytes: usize,
}

/// Options accepted by `LavalinkNode::new`.
#[derive(Clone)]
pub struct LavalinkOptions {
    /// HTTP(S) base URL of the node, e.g. `http://localhost:2333`.
    pub url: String,
    /// Lavalink password (`Authorization` header).
    pub password: String,
    /// The bot's user id (`User-Id` header).
    pub user_id: String,
    /// `Client-Name` header; defaults to `flush/<version>`.
    pub client_name: Option<String>,
    /// Per-request timeout (default 10 s).
    pub timeout: Duration,
    /// How long the Lavalink session stays resumable (default 60 s, 0 disables).
    pub resume_timeout: Duration,
    /// Resume this session id on connect.
    pub session_id: Option<String>,
    /// `None` disables reconnects (default 5 attempts, 1 s → 30 s backoff).
    pub reconnect: Option<ReconnectOptions>,
    /// Response/event byte cap (default 2 MiB).
    pub max_response_bytes: usize,
    /// Simultaneous REST requests (default 128).
    pub max_pending_requests: usize,
    /// Waiters across all in-flight `load_tracks` calls (default 1024).
    pub max_pending_loads: usize,
    /// Simultaneous audio streams (default 128).
    pub max_audio_streams: usize,
    /// Audio stream idle timeout (default 15 s).
    pub audio_idle_timeout: Duration,
    /// `None` disables the load-track cache.
    pub cache: Option<CacheOptions>,
}

impl Default for LavalinkOptions {
    fn default() -> Self {
        Self {
            url: String::new(),
            password: String::new(),
            user_id: String::new(),
            client_name: None,
            timeout: Duration::from_secs(10),
            resume_timeout: Duration::from_secs(60),
            session_id: None,
            reconnect: Some(ReconnectOptions {
                attempts: 5,
                delay: Duration::from_secs(1),
                max_delay: Duration::from_secs(30),
            }),
            max_response_bytes: 2_000_000,
            max_pending_requests: 128,
            max_pending_loads: 1024,
            max_audio_streams: 128,
            audio_idle_timeout: Duration::from_secs(15),
            cache: Some(CacheOptions {
                ttl: Duration::from_secs(15),
                capacity: 64,
                max_bytes: 4_000_000,
            }),
        }
    }
}

/// Counters exposed by `LavalinkNode::diagnostics`.
#[derive(Debug, Clone, Copy, Default)]
pub struct LavalinkDiagnostics {
    pub requests: u64,
    pub pending_requests: usize,
    pub pending_loads: usize,
    pub coalesced_loads: u64,
    pub cache_hits: u64,
    pub cache_entries: usize,
    pub cache_bytes: usize,
    pub active_audio_streams: usize,
}

struct Cached {
    value: TrackLoadResult,
    expires: Instant,
    bytes: usize,
}

#[derive(Default)]
struct Cache {
    map: HashMap<String, Cached>,
    lru: VecDeque<String>,
    bytes: usize,
    hits: u64,
}

type LoadShared = Shared<BoxFuture<'static, MusicResult<TrackLoadResult>>>;

struct Inner {
    base: String,
    ws_base: String,
    password: String,
    user_id: String,
    client_name: String,
    http: surf::Client,
    timeout: Duration,
    resume_timeout: Duration,
    reconnect: Option<ReconnectOptions>,
    max_bytes: usize,
    max_pending: usize,
    max_pending_loads: usize,
    max_audio_streams: usize,
    audio_idle_timeout: Duration,
    cache_opts: Option<CacheOptions>,
    state: Mutex<NodeState>,
    session: Mutex<Option<String>>,
    socket_abort: Mutex<Option<AbortHandle>>,
    connect_lock: Mutex<Option<Shared<BoxFuture<'static, MusicResult<()>>>>>,
    retries: AtomicUsize,
    requests: AtomicU64,
    coalesced_loads: AtomicU64,
    load_waiters: AtomicUsize,
    pending: AtomicUsize,
    stats: Mutex<Option<(LavalinkStats, Instant)>>,
    cache: Mutex<Cache>,
    loading: Mutex<HashMap<String, LoadShared>>,
    audio_streams: Mutex<HashMap<String, AbortHandle>>,
    listeners: Mutex<Vec<(u64, EventListener)>>,
    lifecycle: Mutex<Vec<(u64, LifecycleListener)>>,
    next_listener: AtomicU64,
    closed: AtomicBool,
}

impl Inner {
    fn state(&self) -> NodeState {
        *self.state.lock().unwrap()
    }

    fn set_state(&self, state: NodeState) {
        *self.state.lock().unwrap() = state;
    }

    fn session(&self) -> Option<String> {
        self.session.lock().unwrap().clone()
    }

    fn emit(&self, event: MusicEvent) {
        for (_, listener) in self.listeners.lock().unwrap().iter() {
            listener(event.clone());
        }
    }

    fn emit_lifecycle(&self, event: MusicNodeEvent) {
        for (_, listener) in self.lifecycle.lock().unwrap().iter() {
            listener(event.clone());
        }
    }
}

/// A Lavalink node. Cheap to clone; clones share one connection.
#[derive(Clone)]
pub struct LavalinkNode {
    inner: Arc<Inner>,
}

fn integer(value: usize, min: usize, max: usize, name: &str) -> MusicResult<usize> {
    if !(min..=max).contains(&value) {
        return Err(MusicError::new(
            "LAVALINK_OPTIONS",
            format!("{name} must be an integer between {min} and {max}."),
        ));
    }
    Ok(value)
}

fn duration_ms(value: Duration, min_ms: u64, max_ms: u64, name: &str) -> MusicResult<Duration> {
    let ms = value.as_millis() as u64;
    if !(min_ms..=max_ms).contains(&ms) {
        return Err(MusicError::new(
            "LAVALINK_OPTIONS",
            format!("{name} must be between {min_ms} and {max_ms} milliseconds."),
        ));
    }
    Ok(value)
}

fn check_id(value: &str) -> MusicResult<()> {
    if value.is_empty() || value.len() > 128 {
        return Err(MusicError::new(
            "LAVALINK_ID",
            "Id must contain between 1 and 128 characters.",
        ));
    }
    Ok(())
}

fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

impl LavalinkNode {
    pub fn new(options: LavalinkOptions) -> MusicResult<Self> {
        let parsed = url::Url::parse(&options.url)
            .map_err(|_| MusicError::new("LAVALINK_OPTIONS", "Provide a valid HTTP(S) Lavalink URL."))?;
        if !matches!(parsed.scheme(), "http" | "https")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(MusicError::new(
                "LAVALINK_OPTIONS",
                "The URL must use HTTP(S) without credentials, query or fragment.",
            ));
        }
        let mut base = options.url.trim_end_matches('/').to_string();
        base.push('/');
        let ws_base = format!(
            "{}://{}",
            if parsed.scheme() == "https" { "wss" } else { "ws" },
            &base[base.find("://").unwrap() + 3..]
        );
        if options.password.is_empty() || options.password.contains(['\r', '\n']) {
            return Err(MusicError::new(
                "LAVALINK_OPTIONS",
                "Invalid Lavalink password.",
            ));
        }
        check_id(&options.user_id)?;
        if options.user_id.contains(['\r', '\n']) {
            return Err(MusicError::new(
                "LAVALINK_OPTIONS",
                "User-Id must be valid.",
            ));
        }
        let client_name = options
            .client_name
            .unwrap_or_else(|| format!("flush/{}", env!("CARGO_PKG_VERSION")));
        let valid_client = client_name
            .split_once('/')
            .map(|(name, version)| {
                !name.is_empty()
                    && !version.is_empty()
                    && !name.chars().any(char::is_whitespace)
                    && !version.chars().any(char::is_whitespace)
            })
            .unwrap_or(false);
        if !valid_client {
            return Err(MusicError::new(
                "LAVALINK_OPTIONS",
                "Client-Name must use the name/version shape.",
            ));
        }
        duration_ms(options.timeout, 1, 300_000, "timeout_ms")?;
        duration_ms(options.resume_timeout, 0, 3_600_000, "resume_timeout_seconds")?;
        integer(options.max_response_bytes, 1, 100_000_000, "max_response_bytes")?;
        integer(options.max_pending_requests, 1, 10_000, "max_pending_requests")?;
        integer(options.max_pending_loads, 1, 100_000, "max_pending_loads")?;
        integer(options.max_audio_streams, 1, 10_000, "max_audio_streams")?;
        duration_ms(options.audio_idle_timeout, 1, 300_000, "audio_idle_timeout_ms")?;
        if let Some(reconnect) = options.reconnect {
            integer(reconnect.attempts as usize, 0, 100, "reconnect.attempts")?;
            duration_ms(reconnect.delay, 1, 300_000, "reconnect.delay_ms")?;
            duration_ms(reconnect.max_delay, 1, 300_000, "reconnect.max_delay_ms")?;
        }
        if let Some(cache) = options.cache {
            duration_ms(cache.ttl, 1, 3_600_000, "cache.ttl_ms")?;
            integer(cache.capacity, 1, 10_000, "cache.capacity")?;
            integer(cache.max_bytes, 1, 100_000_000, "cache.max_bytes")?;
        }
        if let Some(session) = &options.session_id {
            check_id(session)?;
        }
        Ok(Self {
            inner: Arc::new(Inner {
                base,
                ws_base,
                password: options.password,
                user_id: options.user_id,
                client_name,
                http: surf::client(),
                timeout: options.timeout,
                resume_timeout: options.resume_timeout,
                reconnect: options.reconnect,
                max_bytes: options.max_response_bytes,
                max_pending: options.max_pending_requests,
                max_pending_loads: options.max_pending_loads,
                max_audio_streams: options.max_audio_streams,
                audio_idle_timeout: options.audio_idle_timeout,
                cache_opts: options.cache,
                state: Mutex::new(NodeState::Idle),
                session: Mutex::new(options.session_id),
                socket_abort: Mutex::new(None),
                connect_lock: Mutex::new(None),
                retries: AtomicUsize::new(0),
                requests: AtomicU64::new(0),
                coalesced_loads: AtomicU64::new(0),
                load_waiters: AtomicUsize::new(0),
                pending: AtomicUsize::new(0),
                stats: Mutex::new(None),
                cache: Mutex::new(Cache::default()),
                loading: Mutex::new(HashMap::new()),
                audio_streams: Mutex::new(HashMap::new()),
                listeners: Mutex::new(Vec::new()),
                lifecycle: Mutex::new(Vec::new()),
                next_listener: AtomicU64::new(0),
                closed: AtomicBool::new(false),
            }),
        })
    }

    pub fn state(&self) -> NodeState {
        self.inner.state()
    }

    pub fn available(&self) -> bool {
        self.inner.state() == NodeState::Ready
    }

    pub fn session_id(&self) -> Option<String> {
        self.inner.session()
    }

    pub fn stats(&self) -> Option<LavalinkStats> {
        self.inner
            .stats
            .lock()
            .unwrap()
            .as_ref()
            .map(|(stats, _)| stats.clone())
    }

    /// Load score used by `MusicManager` node selection (yuzuha's `penalty`).
    pub fn penalty(&self) -> f64 {
        let stats = self.inner.stats.lock().unwrap();
        let Some((stats, received)) = stats.as_ref() else {
            return 0.0;
        };
        if received.elapsed() > Duration::from_secs(90) {
            return 0.0;
        }
        let streams = self.inner.audio_streams.lock().unwrap().len();
        let frames = if streams == 0 { stats.frame_stats.as_ref() } else { None };
        stats.playing_players
            + stats.cpu.lavalink_load * 100.0
            + frames
                .map(|f| (f.deficit.max(0.0) + f.nulled) / 3000.0 * 100.0)
                .unwrap_or(0.0)
    }

    pub fn diagnostics(&self) -> LavalinkDiagnostics {
        let cache = self.inner.cache.lock().unwrap();
        LavalinkDiagnostics {
            requests: self.inner.requests.load(Ordering::SeqCst),
            pending_requests: self.inner.pending.load(Ordering::SeqCst),
            pending_loads: self.inner.load_waiters.load(Ordering::SeqCst),
            coalesced_loads: self.inner.coalesced_loads.load(Ordering::SeqCst),
            cache_hits: cache.hits,
            cache_entries: cache.map.len(),
            cache_bytes: cache.bytes,
            active_audio_streams: self.inner.audio_streams.lock().unwrap().len(),
        }
    }

    /// Connect (or resume) the WebSocket session. Safe to call concurrently;
    /// callers share one handshake.
    pub async fn connect(&self) -> MusicResult<()> {
        let shared = {
            let mut lock = self.inner.connect_lock.lock().unwrap();
            match self.inner.state() {
                NodeState::Closed => {
                    return Err(MusicError::new(
                        "LAVALINK_CLOSED",
                        "The Lavalink node is closed.",
                    ))
                }
                NodeState::Ready => return Ok(()),
                _ => {}
            }
            if let Some(task) = lock.as_ref() {
                task.clone()
            } else {
                let inner = self.inner.clone();
                let task: Shared<BoxFuture<'static, MusicResult<()>>> =
                    connect_once(inner).boxed().shared();
                *lock = Some(task.clone());
                task
            }
        };
        let result = shared.await;
        {
            let mut lock = self.inner.connect_lock.lock().unwrap();
            *lock = None;
        }
        result
    }

    /// Terminate the connection and refuse further work.
    pub async fn close(&self) {
        if self.inner.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.inner.set_state(NodeState::Closed);
        if let Some(abort) = self.inner.socket_abort.lock().unwrap().take() {
            abort.abort();
        }
        let streams: Vec<AbortHandle> = self
            .inner
            .audio_streams
            .lock()
            .unwrap()
            .drain()
            .map(|(_, handle)| handle)
            .collect();
        for handle in streams {
            handle.abort();
        }
        self.inner.cache.lock().unwrap().map.clear();
        self.inner.emit_lifecycle(MusicNodeEvent::Closed);
    }

    fn player_path(&self, guild_id: Option<&str>) -> MusicResult<String> {
        if self.inner.state() != NodeState::Ready {
            return Err(MusicError::new(
                "LAVALINK_NOT_READY",
                "Connect the Lavalink node before controlling players.",
            ));
        }
        let session = self.inner.session().ok_or_else(|| {
            MusicError::new("LAVALINK_NOT_READY", "Connect the Lavalink node first.")
        })?;
        let base = format!("v4/sessions/{}/players", encode(&session));
        Ok(match guild_id {
            Some(guild) => format!("{base}/{}", encode(guild)),
            None => base,
        })
    }

    /// `GET /v4/loadtracks`, with cache and request coalescing.
    /// Clones share the same in-flight request via `Shared`.
    pub async fn load_tracks(&self, identifier: &str) -> MusicResult<TrackLoadResult> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(MusicError::new("LAVALINK_CLOSED", "The Lavalink node is closed."));
        }
        if identifier.trim().is_empty() || identifier.len() > 4096 {
            return Err(MusicError::new(
                "LAVALINK_IDENTIFIER",
                "The identifier must contain between 1 and 4096 characters.",
            ));
        }
        if let Some(cached) = self.cache_get(identifier) {
            return Ok(cached);
        }
        if self.inner.load_waiters.load(Ordering::SeqCst) >= self.inner.max_pending_loads {
            return Err(MusicError::new(
                "LAVALINK_CAPACITY",
                "Pending Lavalink load limit exceeded.",
            ));
        }
        let shared = {
            let mut loading = self.inner.loading.lock().unwrap();
            match loading.get(identifier) {
                Some(flight) => {
                    self.inner.coalesced_loads.fetch_add(1, Ordering::SeqCst);
                    flight.clone()
                }
                None => {
                    let inner = self.inner.clone();
                    let key = identifier.to_string();
                    let identifier = identifier.to_string();
                    let task: Shared<BoxFuture<'static, MusicResult<TrackLoadResult>>> = (async move {
                        let path = format!("v4/loadtracks?identifier={}", encode(&identifier));
                        let result = request(&inner, "GET", &path, None).await;
                        match result {
                            Ok(value) => {
                                let parsed = parse_load_result(&value);
                                if let (Ok(parsed), Some(cache)) = (&parsed, inner.cache_opts) {
                                    if !matches!(parsed, TrackLoadResult::Error(_)) {
                                        cache_insert(&inner, &identifier, parsed.clone(), cache);
                                    }
                                }
                                parsed
                            }
                            Err(error) => Err(error),
                        }
                    })
                                    .boxed()
                        .shared();
                    loading.insert(key, task.clone());
                    task
                }
            }
        };
        self.inner.load_waiters.fetch_add(1, Ordering::SeqCst);
        let result = shared.await;
        self.inner.load_waiters.fetch_sub(1, Ordering::SeqCst);
        {
            let mut loading = self.inner.loading.lock().unwrap();
            loading.remove(identifier);
        }
        result
    }

    fn cache_get(&self, identifier: &str) -> Option<TrackLoadResult> {
        let mut cache = self.inner.cache.lock().unwrap();
        let entry = cache.map.get(identifier)?;
        if entry.expires > Instant::now() {
            let value = entry.value.clone();
            cache.hits += 1;
            // Refresh LRU position.
            cache.lru.retain(|key| key != identifier);
            cache.lru.push_back(identifier.to_string());
            return Some(value);
        }
        let entry = cache.map.remove(identifier).unwrap();
        cache.bytes = cache.bytes.saturating_sub(entry.bytes);
        cache.lru.retain(|key| key != identifier);
        None
    }

    /// `GET /v4/decodetrack`.
    pub async fn decode_track(&self, encoded: &str) -> MusicResult<crate::domain::MusicTrack> {
        check_encoded(encoded)?;
        let path = format!("v4/decodetrack?encodedTrack={}", encode(encoded));
        let value = request(&self.inner, "GET", &path, None).await?;
        serde_json::from_value(value)
            .map_err(|_| MusicError::new("LAVALINK_PROTOCOL", "Invalid track from Lavalink."))
    }

    /// `POST /v4/decodetracks` (1–1000 encoded tracks).
    pub async fn decode_tracks(
        &self,
        encoded: &[String],
    ) -> MusicResult<Vec<crate::domain::MusicTrack>> {
        if encoded.is_empty() || encoded.len() > 1000 {
            return Err(MusicError::new(
                "LAVALINK_TRACKS",
                "Provide between 1 and 1000 encoded tracks.",
            ));
        }
        for value in encoded {
            check_encoded(value)?;
        }
        let body = Value::Array(
            encoded
                .iter()
                .map(|value| Value::String(value.clone()))
                .collect(),
        );
        if body.to_string().len() > self.inner.max_bytes {
            return Err(MusicError::new(
                "LAVALINK_TRACKS",
                "The track batch exceeds the byte limit.",
            ));
        }
        let value = request(&self.inner, "POST", "v4/decodetracks", Some(body)).await?;
        let tracks: Vec<crate::domain::MusicTrack> = serde_json::from_value(value)
            .map_err(|_| MusicError::new("LAVALINK_PROTOCOL", "Invalid track list from Lavalink."))?;
        if tracks.len() != encoded.len() {
            return Err(MusicError::new(
                "LAVALINK_PROTOCOL",
                "Lavalink returned a different number of tracks.",
            ));
        }
        Ok(tracks)
    }

    /// `GET /v4/info`.
    pub async fn info(&self) -> MusicResult<LavalinkInfo> {
        let value = request(&self.inner, "GET", "v4/info", None).await?;
        serde_json::from_value(value)
            .map_err(|_| MusicError::new("LAVALINK_PROTOCOL", "Invalid info from Lavalink."))
    }

    /// `GET /v4/stats`.
    pub async fn get_stats(&self) -> MusicResult<LavalinkStats> {
        let value = request(&self.inner, "GET", "v4/stats", None).await?;
        serde_json::from_value(value)
            .map_err(|_| MusicError::new("LAVALINK_PROTOCOL", "Invalid stats from Lavalink."))
    }

    /// `GET /v4/sessions/{id}/players`.
    pub async fn get_players(&self) -> MusicResult<Vec<RemoteMusicPlayer>> {
        let path = self.player_path(None)?;
        let value = request(&self.inner, "GET", &path, None).await?;
        serde_json::from_value(value)
            .map_err(|_| MusicError::new("LAVALINK_PROTOCOL", "Invalid players from Lavalink."))
    }

    /// `GET /v4/sessions/{id}/players/{guild}`.
    pub async fn get_player(&self, guild_id: &str) -> MusicResult<RemoteMusicPlayer> {
        let path = self.player_path(Some(guild_id))?;
        let value = request(&self.inner, "GET", &path, None).await?;
        let player: RemoteMusicPlayer = serde_json::from_value(value)
            .map_err(|_| MusicError::new("LAVALINK_PROTOCOL", "Invalid player from Lavalink."))?;
        if player.guild_id != guild_id {
            return Err(MusicError::new(
                "LAVALINK_PROTOCOL",
                "Invalid player from Lavalink.",
            ));
        }
        Ok(player)
    }

    /// `PATCH /v4/sessions/{id}/players/{guild}`.
    pub async fn update_player(
        &self,
        guild_id: &str,
        update: MusicPlayerUpdate,
        no_replace: bool,
    ) -> MusicResult<RemoteMusicPlayer> {
        if let Some(volume) = update.volume {
            if !(0..=1000).contains(&volume) {
                return Err(MusicError::new(
                    "LAVALINK_UPDATE",
                    "volume must be an integer between 0 and 1000.",
                ));
            }
        }
        if let Some(position) = update.position {
            if position < 0 {
                return Err(MusicError::new(
                    "LAVALINK_UPDATE",
                    "position must be a non-negative integer.",
                ));
            }
        }
        let path = format!("{}?noReplace={no_replace}", self.player_path(Some(guild_id))?);
        let body = serde_json::to_value(update)
            .map_err(|_| MusicError::new("LAVALINK_UPDATE", "Invalid player update."))?;
        let value = request(&self.inner, "PATCH", &path, Some(body)).await?;
        let player: RemoteMusicPlayer = serde_json::from_value(value)
            .map_err(|_| MusicError::new("LAVALINK_PROTOCOL", "Invalid player from Lavalink."))?;
        if player.guild_id != guild_id {
            return Err(MusicError::new(
                "LAVALINK_PROTOCOL",
                "Invalid player from Lavalink.",
            ));
        }
        Ok(player)
    }

    /// `DELETE /v4/sessions/{id}/players/{guild}`.
    pub async fn destroy_player(&self, guild_id: &str) -> MusicResult<()> {
        let path = self.player_path(Some(guild_id))?;
        request(&self.inner, "DELETE", &path, None).await?;
        Ok(())
    }

    /// `GET /v4/yuzuha/sessions/{id}/players/{guild}/audio` — the continuous
    /// `webm/opus` output of the yuzuha-audio plugin.
    pub async fn open_audio_stream(&self, guild_id: &str) -> MusicResult<MusicAudioStream> {
        let path = format!("{}/audio", self.player_path(Some(guild_id))?)
            .replace("v4/sessions/", "v4/yuzuha/sessions/");
        {
            let streams = self.inner.audio_streams.lock().unwrap();
            if streams.contains_key(guild_id) {
                return Err(MusicError::new(
                    "LAVALINK_AUDIO_BUSY",
                    "There is already an audio output on this guild.",
                ));
            }
            if streams.len() >= self.inner.max_audio_streams {
                return Err(MusicError::new(
                    "LAVALINK_AUDIO_LIMIT",
                    "Audio output limit exceeded.",
                ));
            }
        }
        let url = format!("{}{}", self.inner.base, path);
        let fetching = timeout(
            self.inner.timeout,
            self.inner
                .http
                .get(&url)
                .header("Authorization", self.inner.password.clone()),
        );
        let mut response = fetching
            .await
            .map_err(|_| {
                MusicError::new(
                    "LAVALINK_AUDIO_TIMEOUT",
                    "Lavalink did not open the audio output in time.",
                )
            })?
            .map_err(|error| {
                MusicError::new(
                    "LAVALINK_CONNECTION",
                    format!("Could not open the Lavalink audio output: {error}."),
                )
            })?;
        let status = response.status();
        let version_ok = response
            .header("x-yuzuha-audio-version")
            .map(|value| value.last().as_str() == "1")
            .unwrap_or(false);
        let content_type = response
            .header("content-type")
            .map(|value| {
                value
                    .last()
                    .as_str()
                    .to_ascii_lowercase()
                    .starts_with("audio/webm")
            })
            .unwrap_or(false);
        if !status.is_success() || !version_ok || !content_type {
            let code = if status == surf::StatusCode::NotFound {
                "LAVALINK_AUDIO_PLUGIN"
            } else if status == surf::StatusCode::Conflict {
                "LAVALINK_AUDIO_BUSY"
            } else if status == surf::StatusCode::TooManyRequests {
                "LAVALINK_AUDIO_LIMIT"
            } else if status == surf::StatusCode::Unauthorized
                || status == surf::StatusCode::Forbidden
            {
                "LAVALINK_AUDIO_AUTH"
            } else {
                "LAVALINK_AUDIO_PROTOCOL"
            };
            let message = if code == "LAVALINK_AUDIO_PLUGIN" {
                "Install the yuzuha-audio plugin on Lavalink 4.2.2 to stream audio to Fluxer."
            } else {
                "Incompatible audio output."
            };
            return Err(MusicError::new(code, message));
        }
        let (abort, registration) = AbortHandle::new_pair();
        let _ = &registration;
        self.inner
            .audio_streams
            .lock()
            .unwrap()
            .insert(guild_id.to_string(), abort.clone());
        let (tx, rx) = mpsc::channel(8);
        let inner = self.inner.clone();
        let guild = guild_id.to_string();
        let body = response.take_body();
        task::spawn(async move {
            let mut tx = tx;
            let mut reader = body.into_reader().take((inner.max_bytes + 1) as u64);
            let mut buf = vec![0u8; 16 * 1024];
            let result: MusicResult<()> = loop {
                let read = timeout(inner.audio_idle_timeout, reader.read(&mut buf)).await;
                match read {
                    Err(_) => {
                        break Err(MusicError::new(
                            "LAVALINK_AUDIO_TIMEOUT",
                            "The audio output stopped providing data.",
                        ))
                    }
                    Ok(Err(_)) => {
                        break Err(MusicError::new(
                            "LAVALINK_AUDIO_CONNECTION",
                            "The Lavalink audio connection was interrupted.",
                        ))
                    }
                    Ok(Ok(0)) => break Ok(()),
                    Ok(Ok(n)) => {
                        if n > MAX_AUDIO_CHUNK {
                            break Err(MusicError::new(
                                "LAVALINK_AUDIO_LIMIT",
                                "Audio chunk exceeds 1 MiB.",
                            ));
                        }
                        if tx.send(Ok(buf[..n].to_vec())).await.is_err() {
                            break Ok(());
                        }
                    }
                }
            };
            if let Err(error) = result {
                let _ = tx.send(Err(error)).await;
            }
            inner.audio_streams.lock().unwrap().remove(&guild);
        });
        let abort_for_close = abort.clone();
        Ok(MusicAudioStream {
            format: "webm-opus",
            data: rx,
            close: Some(Box::new(move || {
                abort_for_close.abort();
            })),
        })
    }
}

impl MusicBackend for LavalinkNode {
    fn available(&self) -> Option<bool> {
        Some(self.available())
    }

    fn penalty(&self) -> f64 {
        self.penalty()
    }

    fn load_tracks<'a>(&'a self, identifier: &'a str) -> BoxMusicFuture<'a, TrackLoadResult> {
        Box::pin(LavalinkNode::load_tracks(self, identifier))
    }

    fn update_player<'a>(
        &'a self,
        guild_id: &'a str,
        update: MusicPlayerUpdate,
        no_replace: bool,
    ) -> BoxMusicFuture<'a, RemoteMusicPlayer> {
        Box::pin(LavalinkNode::update_player(self, guild_id, update, no_replace))
    }

    fn destroy_player<'a>(&'a self, guild_id: &'a str) -> BoxMusicFuture<'a, ()> {
        Box::pin(LavalinkNode::destroy_player(self, guild_id))
    }

    fn open_audio_stream<'a>(
        &'a self,
        guild_id: &'a str,
    ) -> Option<BoxMusicFuture<'a, MusicAudioStream>> {
        Some(Box::pin(LavalinkNode::open_audio_stream(
            self, guild_id,
        )))
    }

    fn subscribe(&self, listener: EventListener) -> Subscription {
        let id = self.inner.next_listener.fetch_add(1, Ordering::SeqCst);
        self.inner.listeners.lock().unwrap().push((id, listener));
        let inner = Arc::downgrade(&self.inner);
        Subscription::new(move || {
            if let Some(inner) = inner.upgrade() {
                inner
                    .listeners
                    .lock()
                    .unwrap()
                    .retain(|(listener_id, _)| *listener_id != id);
            }
        })
    }

    fn subscribe_lifecycle(&self, listener: LifecycleListener) -> Option<Subscription> {
        let id = self.inner.next_listener.fetch_add(1, Ordering::SeqCst);
        self.inner.lifecycle.lock().unwrap().push((id, listener));
        let inner = Arc::downgrade(&self.inner);
        Some(Subscription::new(move || {
            if let Some(inner) = inner.upgrade() {
                inner
                    .lifecycle
                    .lock()
                    .unwrap()
                    .retain(|(listener_id, _)| *listener_id != id);
            }
        }))
    }
}

fn check_encoded(value: &str) -> MusicResult<()> {
    if value.is_empty() || value.len() > MAX_ENCODED_LEN {
        return Err(MusicError::new(
            "LAVALINK_TRACKS",
            "Encoded tracks must contain between 1 and 65536 characters.",
        ));
    }
    Ok(())
}

fn cache_insert(inner: &Inner, identifier: &str, value: TrackLoadResult, opts: CacheOptions) {
    let bytes = serde_json::to_string(&value).map(|s| s.len()).unwrap_or(0);
    if bytes > opts.max_bytes {
        return;
    }
    let mut cache = inner.cache.lock().unwrap();
    if let Some(previous) = cache.map.remove(identifier) {
        cache.bytes = cache.bytes.saturating_sub(previous.bytes);
        cache.lru.retain(|key| key != identifier);
    }
    while !cache.lru.is_empty()
        && (cache.map.len() >= opts.capacity || cache.bytes + bytes > opts.max_bytes)
    {
        if let Some(key) = cache.lru.pop_front() {
            if let Some(entry) = cache.map.remove(&key) {
                cache.bytes = cache.bytes.saturating_sub(entry.bytes);
            }
        }
    }
    cache.map.insert(
        identifier.to_string(),
        Cached {
            value,
            expires: Instant::now() + opts.ttl,
            bytes,
        },
    );
    cache.lru.push_back(identifier.to_string());
    cache.bytes += bytes;
}

/// One REST request with pending caps, timeout and response limits.
async fn request(
    inner: &Arc<Inner>,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> MusicResult<Value> {
    if inner.closed.load(Ordering::SeqCst) {
        return Err(MusicError::new("LAVALINK_CLOSED", "The Lavalink node is closed."));
    }
    if inner.pending.load(Ordering::SeqCst) >= inner.max_pending {
        return Err(MusicError::new(
            "LAVALINK_CAPACITY",
            "Simultaneous Lavalink request limit exceeded.",
        ));
    }
    inner.requests.fetch_add(1, Ordering::SeqCst);
    inner.pending.fetch_add(1, Ordering::SeqCst);
    let result = request_inner(inner, method, path, body).await;
    inner.pending.fetch_sub(1, Ordering::SeqCst);
    result
}

async fn request_inner(
    inner: &Arc<Inner>,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> MusicResult<Value> {
    let url = format!("{}{}", inner.base, path);
    let verb = match method {
        "GET" => surf::http::Method::Get,
        "POST" => surf::http::Method::Post,
        "PATCH" => surf::http::Method::Patch,
        "PUT" => surf::http::Method::Put,
        "DELETE" => surf::http::Method::Delete,
        _ => surf::http::Method::Get,
    };
    let mut request = inner
        .http
        .request(verb, &url)
        .header("Authorization", inner.password.clone());
    if let Some(body) = body {
        request = request
            .header("Content-Type", "application/json")
            .body(body);
    }
    let mut response = match timeout(inner.timeout, request).await {
        Ok(result) => result.map_err(|error| {
            MusicError::new(
                "LAVALINK_CONNECTION",
                format!("Could not complete the Lavalink request: {error}."),
            )
        })?,
        Err(_) => {
            return Err(MusicError::new(
                "LAVALINK_TIMEOUT",
                "The Lavalink request timed out.",
            ))
        }
    };
    let status = response.status();
    if !status.is_success() {
        return Err(MusicError::new(
            "LAVALINK_HTTP",
            format!("Lavalink answered HTTP {}.", status as u16),
        ));
    }
    if status == surf::StatusCode::NoContent {
        return Ok(Value::Null);
    }
    if let Some(length) = response
        .header("content-length")
        .and_then(|value| value.as_str().parse::<usize>().ok())
    {
        if length > inner.max_bytes {
            return Err(MusicError::new(
                "LAVALINK_RESPONSE_LIMIT",
                "The Lavalink response exceeds the limit.",
            ));
        }
    }
    let mut bytes = Vec::new();
    let read = timeout(
        inner.timeout,
        response
            .take_body()
            .into_reader()
            .take((inner.max_bytes + 1) as u64)
            .read_to_end(&mut bytes),
    )
    .await;
    match read {
        Err(_) => {
            return Err(MusicError::new(
                "LAVALINK_TIMEOUT",
                "The Lavalink request timed out.",
            ))
        }
        Ok(Err(_)) => {
            return Err(MusicError::new(
                "LAVALINK_CONNECTION",
                "Could not complete the Lavalink request.",
            ))
        }
        Ok(Ok(_)) => {}
    }
    if bytes.len() > inner.max_bytes {
        return Err(MusicError::new(
            "LAVALINK_RESPONSE_LIMIT",
            "The Lavalink response exceeds the limit.",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        MusicError::new("LAVALINK_PROTOCOL", "Lavalink returned invalid JSON.")
    })
}

fn parse_load_result(value: &Value) -> MusicResult<TrackLoadResult> {
    let result: TrackLoadResult = serde_json::from_value(value.clone()).map_err(|_| {
        MusicError::new("LAVALINK_PROTOCOL", "Invalid load result from Lavalink.")
    })?;
    if let TrackLoadResult::Empty(data) = &result {
        if !data.is_none() && !data.as_ref().map(|v| v.is_null()).unwrap_or(true) {
            return Err(MusicError::new(
                "LAVALINK_PROTOCOL",
                "Invalid load result from Lavalink.",
            ));
        }
    }
    Ok(result)
}

/// Parse a Lavalink WS message into a `MusicEvent` (`event`/`playerUpdate`
/// ops). `Ok(None)` for unknown event types; an error for malformed events.
fn parse_music_event(value: &Value) -> MusicResult<Option<MusicEvent>> {
    let guild_id = value
        .get("guildId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            MusicError::new("LAVALINK_PROTOCOL", "Player event without a guild.")
        })?
        .to_string();
    if value.get("op").and_then(Value::as_str) == Some("playerUpdate") {
        let state: crate::domain::MusicPlayerState = serde_json::from_value(
            value.get("state").cloned().unwrap_or(Value::Null),
        )
        .map_err(|_| MusicError::new("LAVALINK_PROTOCOL", "Invalid player event from Lavalink."))?;
        return Ok(Some(MusicEvent::PlayerUpdate { guild_id, state }));
    }
    let kind = value.get("type").and_then(Value::as_str);
    let track = || -> MusicResult<crate::domain::MusicTrack> {
        serde_json::from_value(value.get("track").cloned().unwrap_or(Value::Null)).map_err(|_| {
            MusicError::new("LAVALINK_PROTOCOL", "Invalid player event from Lavalink.")
        })
    };
    let event = match kind {
        Some("TrackStartEvent") => MusicEvent::TrackStart {
            guild_id,
            track: track()?,
        },
        Some("TrackEndEvent") => {
            let reason = match value.get("reason").and_then(Value::as_str) {
                Some("finished") => TrackEndReason::Finished,
                Some("loadFailed") => TrackEndReason::LoadFailed,
                Some("stopped") => TrackEndReason::Stopped,
                Some("replaced") => TrackEndReason::Replaced,
                Some("cleanup") => TrackEndReason::Cleanup,
                _ => {
                    return Err(MusicError::new(
                        "LAVALINK_PROTOCOL",
                        "Invalid player event from Lavalink.",
                    ))
                }
            };
            MusicEvent::TrackEnd {
                guild_id,
                track: track()?,
                reason,
            }
        }
        Some("TrackExceptionEvent") => {
            let exception: crate::domain::TrackException = serde_json::from_value(
                value.get("exception").cloned().unwrap_or(Value::Null),
            )
            .map_err(|_| {
                MusicError::new("LAVALINK_PROTOCOL", "Invalid player event from Lavalink.")
            })?;
            if !matches!(
                exception.severity,
                TrackExceptionSeverity::Common
                    | TrackExceptionSeverity::Suspicious
                    | TrackExceptionSeverity::Fault
            ) {
                return Err(MusicError::new(
                    "LAVALINK_PROTOCOL",
                    "Invalid player event from Lavalink.",
                ));
            }
            MusicEvent::TrackException {
                guild_id,
                track: track()?,
                exception,
            }
        }
        Some("TrackStuckEvent") => {
            let threshold_ms = value
                .get("thresholdMs")
                .and_then(Value::as_f64)
                .ok_or_else(|| {
                    MusicError::new("LAVALINK_PROTOCOL", "Invalid player event from Lavalink.")
                })?;
            MusicEvent::TrackStuck {
                guild_id,
                track: track()?,
                threshold_ms,
            }
        }
        Some("WebSocketClosedEvent") => MusicEvent::WebSocketClosed {
            guild_id,
            code: value.get("code").and_then(Value::as_f64).ok_or_else(|| {
                MusicError::new("LAVALINK_PROTOCOL", "Invalid player event from Lavalink.")
            })?,
            reason: value
                .get("reason")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    MusicError::new("LAVALINK_PROTOCOL", "Invalid player event from Lavalink.")
                })?
                .to_string(),
            by_remote: value
                .get("byRemote")
                .and_then(Value::as_bool)
                .ok_or_else(|| {
                    MusicError::new("LAVALINK_PROTOCOL", "Invalid player event from Lavalink.")
                })?,
        },
        _ => return Ok(None),
    };
    Ok(Some(event))
}

/// Drives one socket until the `ready` op and session PATCH complete, then
/// resolves `ready_tx`; keeps dispatching ops until the socket closes.
#[allow(deprecated)]
async fn socket_loop(
    inner: Arc<Inner>,
    mut ws: async_tungstenite::WebSocketStream<async_tungstenite::async_std::ConnectStream>,
    ready_tx: Mutex<Option<oneshot::Sender<MusicResult<()>>>>,
) {
    let mut buffered: Vec<MusicEvent> = Vec::new();
    let mut buffered_bytes = 0usize;
    let mut confirmed = false;
    while let Some(message) = ws.next().await {
        if inner.closed.load(Ordering::SeqCst) || inner.state() == NodeState::Closed {
            break;
        }
        let message = match message {
            Ok(message) => message,
            Err(_) => break,
        };
        let (payload, size) = match &message {
            WsMessage::Text(text) => (text.as_str().as_bytes().to_vec(), text.len()),
            WsMessage::Binary(data) => (data.to_vec(), data.len()),
            WsMessage::Close(_) => break,
            _ => continue,
        };
        if size > inner.max_bytes || buffered_bytes + size > inner.max_bytes * 4 {
            fail_ready(
                &ready_tx,
                MusicError::new(
                    "LAVALINK_RESPONSE_LIMIT",
                    "A Lavalink event exceeds the limit.",
                ),
            );
            break;
        }
        let value: Value = match serde_json::from_slice(&payload) {
            Ok(value) => value,
            Err(_) => {
                let error = MusicError::new(
                    "LAVALINK_PROTOCOL",
                    "Could not read the Lavalink event.",
                );
                if !confirmed {
                    fail_ready(&ready_tx, error);
                    break;
                }
                continue;
            }
        };
        let op = value.get("op").and_then(Value::as_str).unwrap_or("");
        match op {
            "ready" => {
                if confirmed {
                    continue;
                }
                let (session_id, resumed) = match (
                    value.get("sessionId").and_then(Value::as_str),
                    value.get("resumed").and_then(Value::as_bool),
                ) {
                    (Some(session_id), Some(resumed)) if !session_id.is_empty() => {
                        (session_id.to_string(), resumed)
                    }
                    _ => {
                        fail_ready(
                            &ready_tx,
                            MusicError::new(
                                "LAVALINK_PROTOCOL",
                                "Invalid session from Lavalink.",
                            ),
                        );
                        break;
                    }
                };
                *inner.session.lock().unwrap() = Some(session_id.clone());
                let patch = serde_json::json!({
                    "resuming": inner.resume_timeout.as_millis() > 0,
                    "timeout": inner.resume_timeout.as_secs(),
                });
                let path = format!("v4/sessions/{}", encode(&session_id));
                match request(&inner, "PATCH", &path, Some(patch)).await {
                    Ok(_) => {
                        confirmed = true;
                        inner.retries.store(0, Ordering::SeqCst);
                        inner.set_state(NodeState::Ready);
                        if let Some(tx) = ready_tx.lock().unwrap().take() {
                            let _ = tx.send(Ok(()));
                        }
                        inner.emit_lifecycle(MusicNodeEvent::Ready {
                            session_id,
                            resumed,
                        });
                        for event in buffered.drain(..) {
                            inner.emit(event);
                        }
                        buffered_bytes = 0;
                    }
                    Err(error) => {
                        fail_ready(&ready_tx, error);
                        break;
                    }
                }
            }
            "stats" => {
                if let Ok(stats) = serde_json::from_value::<LavalinkStats>(value) {
                    *inner.stats.lock().unwrap() = Some((stats, Instant::now()));
                }
            }
            "event" | "playerUpdate" => match parse_music_event(&value) {
                Ok(Some(event)) => {
                    if inner.state() == NodeState::Ready {
                        inner.emit(event);
                    } else if buffered.len() < inner.max_pending_loads {
                        buffered_bytes += size;
                        buffered.push(event);
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    if !confirmed {
                        fail_ready(&ready_tx, error);
                        break;
                    }
                }
            },
            _ => {}
        }
    }
    let was_ready = inner.state() == NodeState::Ready;
    inner.socket_abort.lock().unwrap().take();
    if inner.state() != NodeState::Closed {
        inner.set_state(NodeState::Disconnected);
    }
    fail_ready(
        &ready_tx,
        MusicError::new(
            "LAVALINK_CONNECTION",
            "Lavalink closed the connection before confirming the session.",
        ),
    );
    if inner.state() != NodeState::Closed {
        inner.emit_lifecycle(MusicNodeEvent::Disconnect { code: None });
    }
    if was_ready && !inner.closed.load(Ordering::SeqCst) {
        schedule_reconnect(inner);
    }
}

fn fail_ready(
    ready_tx: &Mutex<Option<oneshot::Sender<MusicResult<()>>>>,
    error: MusicError,
) {
    if let Some(tx) = ready_tx.lock().unwrap().take() {
        let _ = tx.send(Err(error));
    }
}

/// `ws -> reader` plus the handshake; resolves once the session is ready.
#[allow(deprecated)]
async fn connect_once(inner: Arc<Inner>) -> MusicResult<()> {
    if inner.closed.load(Ordering::SeqCst) {
        return Err(MusicError::new(
            "LAVALINK_CLOSED",
            "The Lavalink node is closed.",
        ));
    }
    inner.set_state(NodeState::Connecting);
    let url = format!("{}v4/websocket", inner.ws_base);
    let mut request_ws: WsRequest<()> = url
        .into_client_request()
        .map_err(|_| MusicError::new("LAVALINK_CONNECTION", "Could not open the Lavalink WebSocket."))?;
    for (name, value) in [
        ("Authorization", inner.password.as_str()),
        ("User-Id", inner.user_id.as_str()),
        ("Client-Name", inner.client_name.as_str()),
    ] {
        request_ws.headers_mut().append(
            name.parse::<async_tungstenite::tungstenite::http::header::HeaderName>()
                .map_err(|_| {
                    MusicError::new("LAVALINK_CONNECTION", "Invalid Lavalink WebSocket header.")
                })?,
            value.parse().map_err(|_| {
                MusicError::new("LAVALINK_CONNECTION", "Invalid Lavalink WebSocket header.")
            })?,
        );
    }
    if let Some(session) = inner.session() {
        request_ws.headers_mut().append(
            async_tungstenite::tungstenite::http::header::HeaderName::from_static("session-id"),
            session.parse().map_err(|_| {
                MusicError::new("LAVALINK_CONNECTION", "Invalid Lavalink WebSocket header.")
            })?,
        );
    }
    let ws = match timeout(inner.timeout, connect_async(request_ws)).await {
        Ok(Ok((ws, _))) => ws,
        Ok(Err(error)) => {
            inner.set_state(NodeState::Disconnected);
            return Err(MusicError::new(
                "LAVALINK_CONNECTION",
                format!("WebSocket connection to Lavalink failed: {error}."),
            ));
        }
        Err(_) => {
            inner.set_state(NodeState::Disconnected);
            return Err(MusicError::new(
                "LAVALINK_TIMEOUT",
                "Lavalink did not confirm the session in time.",
            ));
        }
    };
    let (tx, rx) = oneshot::channel();
    let ready_tx = Mutex::new(Some(tx));
    let (abort, registration) = AbortHandle::new_pair();
    *inner.socket_abort.lock().unwrap() = Some(abort);
    let reader_inner = inner.clone();
    let reader = Abortable::new(
        task::spawn(socket_loop(reader_inner.clone(), ws, ready_tx)),
        registration,
    );
    task::spawn(async move {
        let _ = reader.await;
    });
    match timeout(inner.timeout, rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(MusicError::new(
            "LAVALINK_CONNECTION",
            "The Lavalink connection dropped during the handshake.",
        )),
        Err(_) => {
            if let Some(abort) = inner.socket_abort.lock().unwrap().take() {
                abort.abort();
            }
            inner.set_state(NodeState::Disconnected);
            Err(MusicError::new(
                "LAVALINK_TIMEOUT",
                "Lavalink did not confirm the session in time.",
            ))
        }
    }
}

fn schedule_reconnect(inner: Arc<Inner>) {
    let Some(reconnect) = inner.reconnect else { return };
    let retries = inner.retries.fetch_add(1, Ordering::SeqCst);
    if retries >= reconnect.attempts as usize {
        return;
    }
    let delay = reconnect
        .delay
        .checked_mul(2u32.saturating_pow(retries as u32))
        .unwrap_or(reconnect.max_delay)
        .min(reconnect.max_delay);
    task::spawn(async move {
        task::sleep(delay).await;
        if inner.closed.load(Ordering::SeqCst) || inner.state() == NodeState::Ready {
            return;
        }
        let shared: Shared<BoxFuture<'static, MusicResult<()>>> =
            connect_once(inner.clone()).boxed().shared();
        *inner.connect_lock.lock().unwrap() = Some(shared.clone());
        let _ = shared.await;
        *inner.connect_lock.lock().unwrap() = None;
        if inner.state() != NodeState::Ready && !inner.closed.load(Ordering::SeqCst) {
            schedule_reconnect(inner);
        }
    });
}
