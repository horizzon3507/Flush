use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static CTR: AtomicU64 = AtomicU64::new(0);

fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn seed() -> u64 {
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let c = CTR.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id() as u64;
    t ^ c.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ pid.wrapping_shl(32)
}

/// Inteiro em `[0, n)`. `n` deve ser positivo.
pub fn below(n: u64) -> u64 {
    splitmix64(seed()) % n
}

/// Inteiro em `[1, n]`. `n` deve ser positivo.
pub fn one_to(n: u64) -> u64 {
    below(n) + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_stays_in_range() {
        for _ in 0..1000 {
            assert!(below(6) < 6);
            assert!(below(100) < 100);
        }
    }

    #[test]
    fn one_to_stays_in_range() {
        for _ in 0..1000 {
            let r = one_to(6);
            assert!((1..=6).contains(&r));
        }
    }
}
