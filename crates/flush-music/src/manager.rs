//! `MusicManager`: node selection, player registry and recovery.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_std::task;
use futures::future::{join_all, BoxFuture, FutureExt, Shared};
use futures::stream::{self, StreamExt};

use crate::backend::{
    MusicAudioOutput, MusicBackend, MusicResult, MusicVoiceAdapter, Subscription,
};
use crate::domain::{MusicNodeEvent, TrackLoadResult};
use crate::error::MusicError;
use crate::player::{MusicPlayer, PlayerEvent};

fn positive(value: usize, max: usize, name: &str) -> MusicResult<usize> {
    if !(1..=max).contains(&value) {
        return Err(MusicError::new(
            "MUSIC_OPTIONS",
            format!("{name} must be an integer between 1 and {max}."),
        ));
    }
    Ok(value)
}

/// Called with every `PlayerError` a player emits.
pub type MusicErrorHandler = Arc<dyn Fn(MusicError, Arc<MusicPlayer>) + Send + Sync>;

/// Options accepted by `MusicManager::new`.
pub struct MusicManagerOptions {
    pub nodes: Vec<Arc<dyn MusicBackend>>,
    pub voice: Option<Arc<dyn MusicVoiceAdapter>>,
    pub audio: Option<Arc<dyn MusicAudioOutput>>,
    /// Maximum simultaneous players (default 1000).
    pub max_players: usize,
    /// Default queue capacity per player (default 1000).
    pub queue_limit: usize,
    /// Default pending-control limit per player (default 128).
    pub max_pending_controls: usize,
    /// Players recovered in parallel after a node reconnects (default 4).
    pub recovery_concurrency: usize,
    /// Called with every `PlayerError` a player emits.
    pub on_error: Option<MusicErrorHandler>,
}

impl Default for MusicManagerOptions {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            voice: None,
            audio: None,
            max_players: 1000,
            queue_limit: 1000,
            max_pending_controls: 128,
            recovery_concurrency: 4,
            on_error: None,
        }
    }
}

/// Per-player overrides for `MusicManager::create_player`.
#[derive(Default)]
pub struct MusicPlayerOptions {
    pub queue_limit: Option<usize>,
    pub node: Option<Arc<dyn MusicBackend>>,
    pub max_pending_controls: Option<usize>,
}

struct Inner {
    players: Arc<Mutex<HashMap<String, Arc<MusicPlayer>>>>,
    counts: Mutex<HashMap<usize, usize>>,
    options: MusicManagerOptions,
    subscriptions: Mutex<Vec<Subscription>>,
    recoveries: Mutex<HashMap<usize, Shared<BoxFuture<'static, MusicResult<()>>>>>,
    recovery_versions: Mutex<HashMap<usize, u64>>,
    closing: AtomicBool,
}

impl Inner {
    fn node_index(&self, node: &Arc<dyn MusicBackend>) -> Option<usize> {
        self.options
            .nodes
            .iter()
            .position(|candidate| Arc::ptr_eq(candidate, node))
    }

    fn select_node(&self) -> Option<Arc<dyn MusicBackend>> {
        let counts = self.counts.lock().unwrap();
        let mut best: Option<(usize, f64)> = None;
        for (index, node) in self.options.nodes.iter().enumerate() {
            if node.available() == Some(false) {
                continue;
            }
            let penalty = node.penalty();
            let penalty = if penalty.is_finite() && penalty >= 0.0 {
                penalty
            } else {
                0.0
            };
            let load = *counts.get(&index).unwrap_or(&0) as f64 + penalty;
            if best.map(|(_, score)| load < score).unwrap_or(true) {
                best = Some((index, load));
            }
        }
        best.map(|(index, _)| self.options.nodes[index].clone())
    }

    fn release(&self, guild_id: &str, player: &Arc<MusicPlayer>, node_index: usize) {
        let mut players = self.players.lock().unwrap();
        if players
            .get(guild_id)
            .map(|current| Arc::ptr_eq(current, player))
            .unwrap_or(false)
        {
            players.remove(guild_id);
            drop(players);
            let mut counts = self.counts.lock().unwrap();
            let count = counts.entry(node_index).or_insert(0);
            *count = count.saturating_sub(1);
        }
    }
}

/// Registry of guild players across one or more Lavalink nodes.
pub struct MusicManager {
    inner: Arc<Inner>,
}

impl MusicManager {
    pub fn new(options: MusicManagerOptions) -> MusicResult<Self> {
        if options.voice.is_some() && options.audio.is_some() {
            return Err(MusicError::new(
                "MUSIC_OPTIONS",
                "Configure only one audio output or voice adapter.",
            ));
        }
        if options.nodes.is_empty() {
            return Err(MusicError::new(
                "MUSIC_OPTIONS",
                "Provide distinct music nodes.",
            ));
        }
        for (index, node) in options.nodes.iter().enumerate() {
            if options
                .nodes
                .iter()
                .skip(index + 1)
                .any(|other| Arc::ptr_eq(other, node))
            {
                return Err(MusicError::new(
                    "MUSIC_OPTIONS",
                    "Provide distinct music nodes.",
                ));
            }
        }
        positive(options.max_players, 100_000, "max_players")?;
        positive(options.queue_limit, 100_000, "queue_limit")?;
        positive(options.max_pending_controls, 10_000, "max_pending_controls")?;
        positive(options.recovery_concurrency, 128, "recovery_concurrency")?;

        let inner = Arc::new(Inner {
            players: Arc::new(Mutex::new(HashMap::new())),
            counts: Mutex::new(HashMap::new()),
            options,
            subscriptions: Mutex::new(Vec::new()),
            recoveries: Mutex::new(HashMap::new()),
            recovery_versions: Mutex::new(HashMap::new()),
            closing: AtomicBool::new(false),
        });

        for node in &inner.options.nodes {
            let players = inner.players.clone();
            let target = node.clone();
            let subscription = node.subscribe(Arc::new(move |event| {
                let player = players.lock().unwrap().get(event.guild_id()).cloned();
                if let Some(player) = player {
                    player.receive(&target, event);
                }
            }));
            inner.subscriptions.lock().unwrap().push(subscription);
            let weak = Arc::downgrade(&inner);
            let target = node.clone();
            if let Some(lifecycle) = node.subscribe_lifecycle(Arc::new(move |event| {
                let Some(inner) = weak.upgrade() else { return };
                if inner.closing.load(Ordering::SeqCst) {
                    return;
                }
                match event {
                    MusicNodeEvent::Ready { resumed: false, .. } => {
                        let inner = inner.clone();
                        let target = target.clone();
                        task::spawn(async move {
                            let _ = start_recovery(&inner, target).await;
                        });
                    }
                    MusicNodeEvent::Disconnect { code } => {
                        let players: Vec<Arc<MusicPlayer>> = inner
                            .players
                            .lock()
                            .unwrap()
                            .values()
                            .filter(|player| Arc::ptr_eq(player.node(), &target))
                            .cloned()
                            .collect();
                        for player in players {
                            player.emit(PlayerEvent::NodeDisconnect(code));
                        }
                    }
                    MusicNodeEvent::Ready { .. } | MusicNodeEvent::Closed => {}
                }
            })) {
                inner.subscriptions.lock().unwrap().push(lifecycle);
            }
        }
        Ok(Self { inner })
    }

    pub fn len(&self) -> usize {
        self.inner.players.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, guild_id: &str) -> Option<Arc<MusicPlayer>> {
        self.inner.players.lock().unwrap().get(guild_id).cloned()
    }

    /// Get or create the player for `guild_id`.
    pub fn create_player(
        &self,
        guild_id: &str,
        options: MusicPlayerOptions,
    ) -> MusicResult<Arc<MusicPlayer>> {
        if self.inner.closing.load(Ordering::SeqCst) {
            return Err(MusicError::new(
                "MUSIC_CLOSED",
                "The music manager is closed.",
            ));
        }
        if guild_id.trim().is_empty() || guild_id.len() > 128 {
            return Err(MusicError::new("MUSIC_GUILD", "Provide a valid guild."));
        }
        if let Some(existing) = self.get(guild_id) {
            return Ok(existing);
        }
        {
            let players = self.inner.players.lock().unwrap();
            if players.len() >= self.inner.options.max_players {
                return Err(MusicError::new(
                    "MUSIC_CAPACITY",
                    "Player limit exceeded.",
                ));
            }
        }
        let node = match options.node {
            Some(node) => {
                if self.inner.node_index(&node).is_none() {
                    return Err(MusicError::new(
                        "MUSIC_NODE",
                        "The node does not belong to this manager.",
                    ));
                }
                node
            }
            None => self.inner.select_node().ok_or_else(|| {
                MusicError::new("MUSIC_NODE_UNAVAILABLE", "No music node available.")
            })?,
        };
        if node.available() == Some(false) {
            return Err(MusicError::new(
                "MUSIC_NODE_UNAVAILABLE",
                "No music node available.",
            ));
        }
        let node_index = self.inner.node_index(&node).unwrap();
        let queue_limit = positive(
            options.queue_limit.unwrap_or(self.inner.options.queue_limit),
            100_000,
            "queue_limit",
        )?;
        let max_pending = positive(
            options
                .max_pending_controls
                .unwrap_or(self.inner.options.max_pending_controls),
            10_000,
            "max_pending_controls",
        )?;
        let weak = Arc::downgrade(&self.inner);
        let guild = guild_id.to_string();
        let on_error = self.inner.options.on_error.clone();
        let player = MusicPlayer::new(
            guild_id.to_string(),
            node.clone(),
            queue_limit,
            self.inner.options.voice.clone(),
            self.inner.options.audio.clone(),
            Arc::new(move |error| {
                if let (Some(inner), Some(handler)) = (weak.upgrade(), on_error.as_ref()) {
                    let player = inner.players.lock().unwrap().get(&guild).cloned();
                    if let Some(player) = player {
                        handler(error, player);
                    }
                }
            }),
            Box::new({
                let inner = self.inner.clone();
                let guild = guild_id.to_string();
                move || {
                    let player = inner.players.lock().unwrap().get(&guild).cloned();
                    if let Some(player) = player {
                        inner.release(&guild, &player, node_index);
                    }
                }
            }),
            max_pending,
        )?;
        self.inner
            .players
            .lock()
            .unwrap()
            .insert(guild_id.to_string(), player.clone());
        *self.inner.counts.lock().unwrap().entry(node_index).or_insert(0) += 1;
        Ok(player)
    }

    /// Load tracks on the least-loaded available node.
    pub async fn search(&self, identifier: &str) -> MusicResult<TrackLoadResult> {
        if self.inner.closing.load(Ordering::SeqCst) {
            return Err(MusicError::new(
                "MUSIC_CLOSED",
                "The music manager is closed.",
            ));
        }
        let node = self.inner.select_node().ok_or_else(|| {
            MusicError::new("MUSIC_NODE_UNAVAILABLE", "No music node available.")
        })?;
        node.load_tracks(identifier).await
    }

    /// Invalidate and recover every player on `node` (new Lavalink session).
    pub async fn recover_node(&self, node: Arc<dyn MusicBackend>) -> MusicResult<()> {
        if self.inner.closing.load(Ordering::SeqCst) {
            return Err(MusicError::new(
                "MUSIC_CLOSED",
                "The music manager is closed.",
            ));
        }
        if self.inner.node_index(&node).is_none() {
            return Err(MusicError::new(
                "MUSIC_NODE",
                "The node does not belong to this manager.",
            ));
        }
        start_recovery(&self.inner, node).await
    }

    /// Destroy the player for `guild_id`. Returns `false` when none exists.
    pub async fn destroy_player(&self, guild_id: &str) -> MusicResult<bool> {
        let Some(player) = self.get(guild_id) else {
            return Ok(false);
        };
        let failures = player.destroy().await;
        if failures.is_empty() {
            Ok(true)
        } else {
            Err(MusicError::new(
                "MUSIC_CLOSE",
                failures
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; "),
            ))
        }
    }

    /// Wait for every player's queued work to finish.
    pub async fn drain(&self) {
        let players: Vec<Arc<MusicPlayer>> =
            self.inner.players.lock().unwrap().values().cloned().collect();
        join_all(players.iter().map(|player| player.drain())).await;
    }

    /// Destroy every player and release node subscriptions.
    pub async fn close(&self) -> MusicResult<()> {
        if self.inner.closing.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.inner.subscriptions.lock().unwrap().clear();
        let players: Vec<Arc<MusicPlayer>> =
            self.inner.players.lock().unwrap().values().cloned().collect();
        let mut failures = Vec::new();
        for player in players {
            failures.extend(player.destroy().await);
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(MusicError::new(
                "MUSIC_CLOSE",
                failures
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; "),
            ))
        }
    }
}

/// Chain a recovery batch after any in-flight one for the same node.
fn start_recovery(
    inner: &Arc<Inner>,
    node: Arc<dyn MusicBackend>,
) -> Shared<BoxFuture<'static, MusicResult<()>>> {
    let Some(node_index) = inner.node_index(&node) else {
        return (async move {
            Err(MusicError::new(
                "MUSIC_NODE",
                "The node does not belong to this manager.",
            ))
        })
        .boxed()
        .shared();
    };
    let previous = inner.recoveries.lock().unwrap().remove(&node_index);
    let worker = inner.clone();
    let task: Shared<BoxFuture<'static, MusicResult<()>>> = (async move {
        if let Some(previous) = previous {
            let _ = previous.await;
        }
        recover_node(&worker, node, node_index).await
    })
    .boxed()
    .shared();
    inner
        .recoveries
        .lock()
        .unwrap()
        .insert(node_index, task.clone());
    task
}

async fn recover_node(
    inner: &Arc<Inner>,
    node: Arc<dyn MusicBackend>,
    node_index: usize,
) -> MusicResult<()> {
    let players: Vec<Arc<MusicPlayer>> = inner
        .players
        .lock()
        .unwrap()
        .values()
        .filter(|player| Arc::ptr_eq(player.node(), &node))
        .cloned()
        .collect();
    for player in &players {
        player.invalidate_session();
    }
    let version = {
        let mut versions = inner.recovery_versions.lock().unwrap();
        let version = versions.get(&node_index).copied().unwrap_or(0) + 1;
        versions.insert(node_index, version);
        version
    };
    if inner.closing.load(Ordering::SeqCst)
        || inner.recovery_versions.lock().unwrap().get(&node_index).copied() != Some(version)
    {
        return Ok(());
    }
    let concurrency = inner.options.recovery_concurrency;
    let failures: Vec<Option<MusicError>> = stream::iter(players)
        .map(|player| {
            let inner = inner.clone();
            async move {
                let keep = {
                    let players = inner.players.lock().unwrap();
                    players
                        .get(player.guild_id())
                        .map(|current| Arc::ptr_eq(current, &player))
                        .unwrap_or(false)
                };
                if !keep {
                    return None;
                }
                match player.recover().await {
                    Ok(()) => None,
                    Err(error) if !inner.closing.load(Ordering::SeqCst) => {
                        let arc = Arc::new(error.clone());
                        player.emit(PlayerEvent::PlayerError(arc));
                        if let Some(handler) = &inner.options.on_error {
                            handler(error.clone(), player.clone());
                        }
                        Some(error)
                    }
                    Err(_) => None,
                }
            }
        })
        .buffer_unordered(concurrency)
        .collect::<Vec<Option<MusicError>>>()
        .await;
    let failures: Vec<MusicError> = failures.into_iter().flatten().collect();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(MusicError::new(
            "MUSIC_RECOVERY",
            format!("Failed to recover {} music player(s).", failures.len()),
        ))
    }
}
