//! flush-music: music and Lavalink support for Flush bots.
//!
//! - `LavalinkNode`: Lavalink v4 REST + WebSocket client (a `MusicBackend`).
//! - `MusicManager`: player registry with node selection and recovery.
//! - `MusicPlayer`: per-guild queue, playback control, filters and events.
//! - `MusicBackend`/`MusicVoiceAdapter`/`MusicAudioOutput`: adapter ports.
//! - `testing`: `mock_track` and `MockMusicBackend` for offline tests.

pub mod backend;
pub mod domain;
pub mod error;
pub mod manager;
pub mod node;
pub mod player;
pub mod queue;
pub mod testing;

pub use backend::{
    BoxMusicFuture, EventListener, LifecycleListener, MusicAudioOutput, MusicAudioStream,
    MusicBackend, MusicResult, MusicVoiceAdapter, Subscription,
};
pub use domain::*;
pub use error::MusicError;
pub use manager::{MusicManager, MusicManagerOptions, MusicPlayerOptions};
pub use node::{
    CacheOptions, LavalinkDiagnostics, LavalinkNode, LavalinkOptions, NodeState, ReconnectOptions,
};
pub use player::{LoopMode, MusicPlayer, PlayerEvent, PLAYBACK_MARKER};
pub use queue::TrackQueue;
pub use testing::{mock_track, MockMusicBackend, RecordedUpdate};
