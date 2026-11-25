//! Port of `src/functions/builtins/voice.ts` — the voice provider is
//! deferred, so every builtin reports NATIVE_UNAVAILABLE.

use crate::errors::{codes, FnError};
use crate::types::Definitions;

macro_rules! unavailable {
    ($d:ident, $(($name:expr, $min:expr, $max:expr, $usage:expr, $desc:expr)),+ $(,)?) => {$(
        $d.add($name, "voice", $min, $max, $usage, $desc, |c| {
            Box::pin(async move {
                let _ = c;
                Err(FnError::new(codes::NATIVE_UNAVAILABLE, "Voice is not available.").into())
            })
        });
    )+};
}

pub fn register(d: &mut Definitions) {
    unavailable!(
        d,
        ("voiceStates", 0, 0, "", "Lists up to 100 guild voice states, without sessions."),
        ("voiceMembers", 0, 1, "[canal?]", "Lists up to 100 members; default: the author's voice channel."),
        ("botVoiceChannel", 0, 0, "", "The bot's connected channel or empty text."),
        ("voiceJoin", 0, 1, "[canal?]", "Connects to the author's or the given channel; requires Connect for the author and Connect/Speak for the bot."),
        ("voiceStatus", 0, 0, "", "Connection, channel, guild, playback and volume; never exposes endpoint/token."),
        ("voiceLeave", 0, 1, "[canal?]", "Leaves voice; requires the bot's same channel or MoveMembers, even without a connection."),
        ("voiceStop", 0, 0, "", "Stops audio and video; requires the bot's same channel or MoveMembers."),
        ("voiceVolume", 0, 1, "[volume?]", "Reads or sets integer volume 0-200; requires the bot's same channel or MoveMembers."),
        ("botVoiceState", 0, 2, "[selfMute?;selfDeaf?]", "Updates the bot's mute/deaf; requires the bot's same channel or MoveMembers."),
    );
}
