//! Music ports: backend, voice adapter and audio output contracts.

use std::pin::Pin;
use std::sync::Arc;

use futures::channel::mpsc;
use futures::Future;

use crate::domain::{
    MusicEvent, MusicNodeEvent, MusicPlayerUpdate, MusicVoiceState, RemoteMusicPlayer,
    TrackLoadResult,
};
use crate::error::MusicError;

pub type MusicResult<T> = Result<T, MusicError>;
pub type BoxMusicFuture<'a, T> = Pin<Box<dyn Future<Output = MusicResult<T>> + Send + 'a>>;

/// A listener for player events. The returned `Subscription` unregisters on drop.
pub type EventListener = Arc<dyn Fn(MusicEvent) + Send + Sync>;
pub type LifecycleListener = Arc<dyn Fn(MusicNodeEvent) + Send + Sync>;

/// Unregisters the listener it was created from when dropped.
pub struct Subscription {
    unsubscribe: Option<Box<dyn FnOnce() + Send>>,
}

impl Subscription {
    pub fn new(unsubscribe: impl FnOnce() + Send + 'static) -> Self {
        Self {
            unsubscribe: Some(Box::new(unsubscribe)),
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(unsubscribe) = self.unsubscribe.take() {
            unsubscribe();
        }
    }
}

/// A continuous encoded audio output from the node (`webm/opus` chunks).
/// Dropping the receiver closes the stream.
pub struct MusicAudioStream {
    pub format: &'static str,
    pub data: mpsc::Receiver<MusicResult<Vec<u8>>>,
    pub(crate) close: Option<Box<dyn FnOnce() + Send>>,
}

impl MusicAudioStream {
    pub fn close(&mut self) {
        if let Some(close) = self.close.take() {
            close();
        }
    }
}

impl Drop for MusicAudioStream {
    fn drop(&mut self) {
        self.close();
    }
}

/// The backend a `MusicPlayer` talks to: a Lavalink node, or a mock in tests.
/// Every method must be object-safe; `subscribe` registers listeners that the
/// backend invokes from its own tasks.
pub trait MusicBackend: Send + Sync {
    /// `Some(false)` means the node is known to be down; `None` means unknown.
    fn available(&self) -> Option<bool> {
        None
    }

    /// Extra load used by `MusicManager` when picking a node.
    fn penalty(&self) -> f64 {
        0.0
    }

    fn load_tracks<'a>(&'a self, identifier: &'a str) -> BoxMusicFuture<'a, TrackLoadResult>;

    fn update_player<'a>(
        &'a self,
        guild_id: &'a str,
        update: MusicPlayerUpdate,
        no_replace: bool,
    ) -> BoxMusicFuture<'a, RemoteMusicPlayer>;

    fn destroy_player<'a>(&'a self, guild_id: &'a str) -> BoxMusicFuture<'a, ()>;

    /// The `yuzuha-audio` plugin's continuous output. `None` when unsupported.
    fn open_audio_stream<'a>(
        &'a self,
        _guild_id: &'a str,
    ) -> Option<BoxMusicFuture<'a, MusicAudioStream>> {
        None
    }

    fn subscribe(&self, listener: EventListener) -> Subscription;

    fn subscribe_lifecycle(&self, listener: LifecycleListener) -> Option<Subscription> {
        let _ = listener;
        None
    }
}

/// Voice join/leave used by `MusicPlayer::connect` when the bot owns the voice
/// connection itself (the voice runtime is tracked separately in PARITY.md).
pub trait MusicVoiceAdapter: Send + Sync {
    fn join<'a>(
        &'a self,
        guild_id: &'a str,
        channel_id: &'a str,
    ) -> BoxMusicFuture<'a, MusicVoiceState>;

    fn leave<'a>(&'a self, guild_id: &'a str) -> BoxMusicFuture<'a, ()>;
}

/// An audio output that owns the whole pipeline from the node to the voice
/// connection (yuzuha's `MusicAudioOutput`, e.g. `FluxerLavalinkBridge`).
pub trait MusicAudioOutput: Send + Sync {
    fn connect<'a>(
        &'a self,
        node: Arc<dyn MusicBackend>,
        guild_id: &'a str,
        channel_id: &'a str,
        on_state: Arc<dyn Fn(bool, Option<MusicError>) + Send + Sync>,
    ) -> BoxMusicFuture<'a, ()>;

    fn disconnect<'a>(&'a self, guild_id: &'a str) -> BoxMusicFuture<'a, ()>;
}
