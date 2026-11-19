/// Uniform-enough value in `0..bound` for games (not for cryptography).
pub fn random_below(bound: u64) -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(unix_nanos());
    hasher.finish() % bound.max(1)
}

fn unix_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod random_tests {
    use super::random_below;

    #[test]
    fn stays_below_the_bound() {
        assert!((0..100).all(|_| random_below(6) < 6));
        assert_eq!(random_below(1), 0);
    }
}
