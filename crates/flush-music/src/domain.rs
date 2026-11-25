//! Lavalink and music domain types (the wire shapes of Lavalink v4).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Extra JSON object fields kept verbatim (`pluginInfo`, `userData`).
pub type Extra = Map<String, Value>;

fn empty_extra() -> Extra {
    Map::new()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
    pub identifier: String,
    pub is_seekable: bool,
    pub author: String,
    /// Duration in milliseconds.
    pub length: u64,
    pub is_stream: bool,
    /// Start position in milliseconds.
    pub position: u64,
    pub title: String,
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artwork_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isrc: Option<String>,
    pub source_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicTrack {
    pub encoded: String,
    pub info: TrackInfo,
    #[serde(default = "empty_extra")]
    pub plugin_info: Extra,
    #[serde(default = "empty_extra")]
    pub user_data: Extra,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackException {
    pub message: Option<String>,
    pub severity: TrackExceptionSeverity,
    pub cause: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackExceptionSeverity {
    Common,
    Suspicious,
    Fault,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "loadType", content = "data", rename_all = "camelCase")]
pub enum TrackLoadResult {
    Track(MusicTrack),
    Playlist(PlaylistData),
    Search(Vec<MusicTrack>),
    Empty(Option<Value>),
    Error(TrackException),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistData {
    pub info: PlaylistInfo,
    #[serde(default = "empty_extra")]
    pub plugin_info: Extra,
    pub tracks: Vec<MusicTrack>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistInfo {
    pub name: String,
    pub selected_track: i64,
}

/// Voice connection data the Lavalink player needs (`voice` in player updates).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicVoiceState {
    pub token: String,
    pub endpoint: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MusicPlayerState {
    pub time: i64,
    pub position: i64,
    pub connected: bool,
    pub ping: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicFilters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equalizer: Option<Vec<EqualizerBand>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub karaoke: Option<KaraokeFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timescale: Option<TimescaleFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tremolo: Option<FrequencyDepthFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vibrato: Option<FrequencyDepthFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<RotationFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distortion: Option<DistortionFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_mix: Option<ChannelMixFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_pass: Option<LowPassFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_filters: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EqualizerBand {
    pub band: u32,
    pub gain: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KaraokeFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mono_level: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_band: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_width: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TimescaleFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FrequencyDepthFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RotationFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_hz: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistortionFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sin_offset: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sin_scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cos_offset: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cos_scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tan_offset: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tan_scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMixFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_to_left: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_to_right: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_to_left: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_to_right: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LowPassFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smoothing: Option<f64>,
}

/// One track in a player update: either an encoded track (or `null` to stop)
/// or an identifier for Lavalink to resolve.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum UpdateTrack {
    Encoded {
        encoded: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_data: Option<Extra>,
    },
    Identifier {
        identifier: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_data: Option<Extra>,
    },
}

/// `PATCH .../players/{guildId}` body. Only set fields are sent.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicPlayerUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<UpdateTrack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<Option<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<MusicFilters>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<MusicVoiceState>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteMusicPlayer {
    pub guild_id: String,
    pub track: Option<MusicTrack>,
    pub volume: i64,
    pub paused: bool,
    pub state: MusicPlayerState,
    pub voice: MusicVoiceState,
    #[serde(default)]
    pub filters: MusicFilters,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackEndReason {
    Finished,
    LoadFailed,
    Stopped,
    Replaced,
    Cleanup,
}

/// A player event received over the Lavalink WebSocket (`op: "event"` or
/// `op: "playerUpdate"`).
#[derive(Debug, Clone, PartialEq)]
pub enum MusicEvent {
    PlayerUpdate {
        guild_id: String,
        state: MusicPlayerState,
    },
    TrackStart {
        guild_id: String,
        track: MusicTrack,
    },
    TrackEnd {
        guild_id: String,
        track: MusicTrack,
        reason: TrackEndReason,
    },
    TrackException {
        guild_id: String,
        track: MusicTrack,
        exception: TrackException,
    },
    TrackStuck {
        guild_id: String,
        track: MusicTrack,
        threshold_ms: f64,
    },
    WebSocketClosed {
        guild_id: String,
        code: f64,
        reason: String,
        by_remote: bool,
    },
}

impl MusicEvent {
    pub fn guild_id(&self) -> &str {
        match self {
            Self::PlayerUpdate { guild_id, .. }
            | Self::TrackStart { guild_id, .. }
            | Self::TrackEnd { guild_id, .. }
            | Self::TrackException { guild_id, .. }
            | Self::TrackStuck { guild_id, .. }
            | Self::WebSocketClosed { guild_id, .. } => guild_id,
        }
    }

    pub fn track(&self) -> Option<&MusicTrack> {
        match self {
            Self::TrackStart { track, .. }
            | Self::TrackEnd { track, .. }
            | Self::TrackException { track, .. }
            | Self::TrackStuck { track, .. } => Some(track),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LavalinkStats {
    pub players: f64,
    pub playing_players: f64,
    pub uptime: f64,
    pub memory: LavalinkMemory,
    pub cpu: LavalinkCpu,
    #[serde(default)]
    pub frame_stats: Option<LavalinkFrameStats>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LavalinkMemory {
    pub free: f64,
    pub used: f64,
    pub allocated: f64,
    pub reservable: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LavalinkCpu {
    pub cores: f64,
    pub system_load: f64,
    pub lavalink_load: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LavalinkFrameStats {
    pub sent: f64,
    pub nulled: f64,
    pub deficit: f64,
}

/// Node lifecycle events emitted by `LavalinkNode::subscribe_lifecycle`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MusicNodeEvent {
    Ready { session_id: String, resumed: bool },
    Disconnect { code: Option<u16> },
    Closed,
}

/// `GET /v4/info` response.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LavalinkInfo {
    pub version: LavalinkVersion,
    pub build_time: i64,
    pub git: LavalinkGit,
    pub jvm: String,
    pub lavaplayer: String,
    pub source_managers: Vec<String>,
    pub filters: Vec<String>,
    #[serde(default)]
    pub plugins: Vec<LavalinkPlugin>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LavalinkVersion {
    pub semver: String,
    pub major: i64,
    pub minor: i64,
    pub patch: i64,
    pub pre_release: Option<String>,
    pub build: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LavalinkGit {
    pub branch: String,
    pub commit: String,
    pub commit_time: i64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LavalinkPlugin {
    pub name: String,
    pub version: String,
}
