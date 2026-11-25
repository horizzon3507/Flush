//! Port of `src/functions/builtins/music.ts` — the music provider is
//! deferred, so every builtin reports NATIVE_UNAVAILABLE.

use crate::errors::{codes, FnError};
use crate::types::Definitions;

macro_rules! unavailable {
    ($d:ident, $(($name:expr, $min:expr, $max:expr, $usage:expr, $desc:expr)),+ $(,)?) => {$(
        $d.add($name, "music", $min, $max, $usage, $desc, |c| {
            Box::pin(async move {
                let _ = c;
                Err(FnError::new(codes::NATIVE_UNAVAILABLE, "Music is not available.").into())
            })
        });
    )+};
}

pub fn register(d: &mut Definitions) {
    unavailable!(
        d,
        ("musicSearch", 1, 2, "[busca;limite?]", "Searches up to 25 tracks; queries up to 500 chars, no encoded blobs."),
        ("musicPlay", 1, 2, "[busca;índice?]", "Queues a track from the search/playlist (index from 1), connects to the author's voice and starts when idle."),
        ("musicNowPlaying", 0, 0, "", "Current track, position in ms, pause, volume and loop, without encoded data."),
        ("musicQueue", 0, 1, "[página?]", "Lists up to 25 queued tracks; pages start at 1."),
        ("musicPause", 0, 0, "", "Controls the player; requires the same voice channel or MoveMembers."),
        ("musicResume", 0, 0, "", "Controls the player; requires the same voice channel or MoveMembers."),
        ("musicSkip", 0, 0, "", "Controls the player; requires the same voice channel or MoveMembers."),
        ("musicShuffle", 0, 0, "", "Controls the player; requires the same voice channel or MoveMembers."),
        ("musicClear", 0, 0, "", "Controls the player; requires the same voice channel or MoveMembers."),
        ("musicStop", 0, 1, "[limparFila?]", "Stops playback; clears the queue by default. Requires the same channel or MoveMembers."),
        ("musicRemove", 1, 2, "[índice;quantidade?]", "Removes up to 25 queued tracks (index from 1); requires the same channel or MoveMembers."),
        ("musicMove", 2, 2, "[de;para]", "Moves a track in the queue; indexes from 1. Requires the same channel or MoveMembers."),
        ("musicLoop", 0, 1, "[off|track|queue?]", "Reads or sets loop; requires the same channel or MoveMembers."),
        ("musicVolume", 0, 1, "[volume?]", "Reads or sets integer volume 0-200; requires the same channel or MoveMembers."),
        ("musicSeek", 1, 1, "[ms]", "Seeks a position in milliseconds; requires a seekable track and the same channel or MoveMembers."),
        ("musicFilters", 0, 1, "[json?]", "Reads or replaces default filters via strict JSON (up to 5000 chars); requires the same channel or MoveMembers."),
    );
}
