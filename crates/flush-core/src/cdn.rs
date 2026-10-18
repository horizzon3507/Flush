//! URLs de CDN da Fluxer (espelho de `Cdn.ts` do `@fluxerjs/core`).
//!
//! Avatar customizado vive em `fluxerusercontent.com`; o fallback usa
//! `fluxerstatic.com/avatars/{id % 6}.png`. Hash `a_*` serve `gif`, o resto
//! `png`; `size` sofre snap na escada oficial (ícone: 128–1024).

use crate::snowflake::Snowflake;

pub const CDN_URL: &str = "https://fluxerusercontent.com";
pub const STATIC_CDN_URL: &str = "https://fluxerstatic.com";

const SIZE_LADDER: &[u32] = &[
    16, 20, 22, 24, 28, 32, 40, 44, 48, 56, 60, 64, 80, 96, 100, 128, 160, 240, 256, 300, 320, 480,
    512, 600, 640, 1024, 1280, 1536, 2048, 3072, 4096, 8192, 16384,
];
const ICON_MIN: u32 = 128;
const ICON_MAX: u32 = 1024;

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

/// `cdnAvatarURL`: `None` para hash vazio, como no SDK.
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

/// `cdnDefaultAvatarURL`: índice `id % 6` sobre o CDN estático.
pub fn default_avatar_url(user_id: Snowflake) -> String {
    format!("{}/avatars/{}.png", STATIC_CDN_URL, user_id.get() % 6)
}

/// `cdnDisplayAvatarURL`: customizado ou fallback, nunca vazio.
pub fn display_avatar_url(user_id: Snowflake, hash: Option<&str>, size: Option<u32>) -> String {
    hash.and_then(|h| avatar_url(user_id, h, size))
        .unwrap_or_else(|| default_avatar_url(user_id))
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
