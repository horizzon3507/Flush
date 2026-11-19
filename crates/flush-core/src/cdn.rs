//! Fluxer CDN URLs (mirrors `Cdn.ts` in `@fluxerjs/core`).
//!
//! Custom avatars live on `fluxerusercontent.com`; the fallback uses
//! `fluxerstatic.com/avatars/{id % 6}.png`. `a_*` hashes serve `gif`, the rest
//! `png`; `size` snaps to the official ladder (icon: 128–1024).

use crate::snowflake::Snowflake;

pub const CDN_URL: &str = "https://fluxerusercontent.com";
pub const STATIC_CDN_URL: &str = "https://fluxerstatic.com";

const SIZE_LADDER: &[u32] = &[
    16, 20, 22, 24, 28, 32, 40, 44, 48, 56, 60, 64, 80, 96, 100, 128, 160, 240, 256, 300, 320, 480,
    512, 600, 640, 1024, 1280, 1536, 2048, 3072, 4096, 8192, 16384,
];
const ICON_MIN: u32 = 128;
const ICON_MAX: u32 = 1024;

/// Size classes of `CDN_ASSET_CLASS_RANGE` in `@fluxerjs/core`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetClass {
    Icon,
    Banner,
    Emoji,
    Sticker,
}

impl AssetClass {
    fn range(self) -> (u32, u32) {
        match self {
            Self::Icon => (ICON_MIN, ICON_MAX),
            Self::Banner => (480, 2400),
            Self::Emoji => (32, 512),
            Self::Sticker => (128, 512),
        }
    }
}

/// Guild asset folders served by the media CDN.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuildAsset {
    Icon,
    Banner,
    Splash,
}

/// Snaps `requested` to the official ladder inside the class range.
pub fn snap_size(requested: u32, class: AssetClass) -> u32 {
    let (min, max) = class.range();
    let clamped = nearest(requested, SIZE_LADDER).clamp(min, max);
    let allowed: Vec<u32> = SIZE_LADDER
        .iter()
        .copied()
        .filter(|s| (min..=max).contains(s))
        .collect();
    nearest(clamped, &allowed)
}

fn size_query(size: Option<u32>, class: AssetClass) -> String {
    match size {
        Some(s) if s > 0 => format!("?size={}", snap_size(s, class)),
        _ => String::new(),
    }
}

fn nearest(requested: u32, candidates: &[u32]) -> u32 {
    let mut best = candidates[0];
    let mut best_dist = requested.abs_diff(best);
    for &step in &candidates[1..] {
        let dist = requested.abs_diff(step);
        if dist < best_dist || (dist == best_dist && step > best) {
            best = step;
            best_dist = dist;
        }
    }
    best
}

fn snap_icon_size(requested: u32) -> u32 {
    let snapped = nearest(requested, SIZE_LADDER);
    let clamped = snapped.clamp(ICON_MIN, ICON_MAX);
    let allowed: Vec<u32> = SIZE_LADDER
        .iter()
        .copied()
        .filter(|s| (ICON_MIN..=ICON_MAX).contains(s))
        .collect();
    nearest(clamped, &allowed)
}

fn extension(hash: &str) -> &'static str {
    if hash.starts_with("a_") {
        "gif"
    } else {
        "png"
    }
}

/// `cdnAvatarURL`: `None` for an empty hash, as in the SDK.
pub fn avatar_url(user_id: Snowflake, hash: &str, size: Option<u32>) -> Option<String> {
    if hash.is_empty() {
        return None;
    }
    let mut url = format!(
        "{}/avatars/{}/{}.{}",
        CDN_URL,
        user_id,
        hash,
        extension(hash)
    );
    if let Some(size) = size {
        url.push_str(&format!("?size={}", snap_icon_size(size)));
    }
    Some(url)
}

/// `cdnDefaultAvatarURL`: index `id % 6` on the static CDN.
pub fn default_avatar_url(user_id: Snowflake) -> String {
    format!("{}/avatars/{}.png", STATIC_CDN_URL, user_id.get() % 6)
}

/// `cdnDisplayAvatarURL`: custom or fallback, never empty.
pub fn display_avatar_url(user_id: Snowflake, hash: Option<&str>, size: Option<u32>) -> String {
    hash.and_then(|h| avatar_url(user_id, h, size))
        .unwrap_or_else(|| default_avatar_url(user_id))
}

/// `cdnBannerURL`: user or guild banner; `None` for an empty hash.
pub fn banner_url(resource_id: Snowflake, hash: &str, size: Option<u32>) -> Option<String> {
    if hash.is_empty() {
        return None;
    }
    Some(format!(
        "{CDN_URL}/banners/{resource_id}/{hash}.{}{}",
        extension(hash),
        size_query(size, AssetClass::Banner)
    ))
}

/// `cdnMemberAvatarURL`: per-guild avatar.
pub fn member_avatar_url(
    guild_id: Snowflake,
    user_id: Snowflake,
    hash: &str,
    size: Option<u32>,
) -> Option<String> {
    if hash.is_empty() {
        return None;
    }
    Some(format!(
        "{CDN_URL}/guilds/{guild_id}/users/{user_id}/avatars/{hash}.{}{}",
        extension(hash),
        size_query(size, AssetClass::Icon)
    ))
}

/// `cdnMemberBannerURL`: per-guild banner.
pub fn member_banner_url(
    guild_id: Snowflake,
    user_id: Snowflake,
    hash: &str,
    size: Option<u32>,
) -> Option<String> {
    if hash.is_empty() {
        return None;
    }
    Some(format!(
        "{CDN_URL}/guilds/{guild_id}/users/{user_id}/banners/{hash}.{}{}",
        extension(hash),
        size_query(size, AssetClass::Banner)
    ))
}

/// `cdnGuildAssetURL`: always `png`, as in the SDK.
pub fn guild_asset_url(
    kind: GuildAsset,
    guild_id: Snowflake,
    hash: &str,
    size: Option<u32>,
) -> Option<String> {
    if hash.is_empty() {
        return None;
    }
    let (folder, class) = match kind {
        GuildAsset::Icon => ("icons", AssetClass::Icon),
        GuildAsset::Banner => ("banners", AssetClass::Banner),
        GuildAsset::Splash => ("splashes", AssetClass::Banner),
    };
    Some(format!(
        "{CDN_URL}/{folder}/{guild_id}/{hash}.png{}",
        size_query(size, class)
    ))
}

/// `cdnEmojiURL`.
pub fn emoji_url(emoji_id: Snowflake, animated: bool, size: Option<u32>) -> String {
    let ext = if animated { "gif" } else { "png" };
    format!(
        "{CDN_URL}/emojis/{emoji_id}.{ext}{}",
        size_query(size, AssetClass::Emoji)
    )
}

/// `cdnStickerURL`.
pub fn sticker_url(sticker_id: Snowflake, animated: bool, size: Option<u32>) -> String {
    let ext = if animated { "gif" } else { "png" };
    format!(
        "{CDN_URL}/stickers/{sticker_id}.{ext}{}",
        size_query(size, AssetClass::Sticker)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: &str) -> Snowflake {
        raw.parse().unwrap()
    }

    #[test]
    fn custom_avatar_matches_sdk() {
        assert_eq!(
            display_avatar_url(id("123"), Some("abc"), Some(512)),
            "https://fluxerusercontent.com/avatars/123/abc.png?size=512"
        );
    }

    #[test]
    fn animated_hash_serves_gif() {
        let url = avatar_url(id("123"), "a_xyz", None).unwrap();
        assert!(url.ends_with("/a_xyz.gif"), "{url}");
    }

    #[test]
    fn default_avatar_matches_sdk() {
        assert_eq!(
            display_avatar_url(id("123"), None, None),
            "https://fluxerstatic.com/avatars/3.png"
        );
    }

    #[test]
    fn size_snaps_to_ladder() {
        let url = avatar_url(id("123"), "abc", Some(500)).unwrap();
        assert!(url.ends_with("?size=512"), "{url}");
        let url = avatar_url(id("123"), "abc", Some(100)).unwrap();
        assert!(url.ends_with("?size=128"), "{url}");
    }

    #[test]
    fn empty_hash_falls_back() {
        assert_eq!(
            display_avatar_url(id("123"), Some(""), None),
            "https://fluxerstatic.com/avatars/3.png"
        );
    }
}
