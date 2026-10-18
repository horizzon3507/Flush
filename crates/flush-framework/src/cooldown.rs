//! Cooldowns por (usuário, comando) — espelho de `Jully Bot.js`.
//!
//! Jully: `setCooldown(user, cmd, ms)` + `isOnCooldown`/`getCooldownRemaining`
//! com resposta "Aguarde Ns". Relógio em ms injetável para teste.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

fn system_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct Cooldowns {
    inner: HashMap<(String, String), u64>,
    now_ms: fn() -> u64,
}

impl Default for Cooldowns {
    fn default() -> Self {
        Self {
            inner: HashMap::new(),
            now_ms: system_ms,
        }
    }
}

impl Cooldowns {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    fn with_clock(now_ms: fn() -> u64) -> Self {
        Self {
            inner: HashMap::new(),
            now_ms,
        }
    }

    pub fn set(&mut self, user: &str, command: &str, cooldown_ms: u64) {
        let until = (self.now_ms)() + cooldown_ms;
        self.inner
            .insert((user.to_owned(), command.to_owned()), until);
    }

    /// Segundos restantes (arredondado p/ cima) ou `None` se livre.
    pub fn remaining(&self, user: &str, command: &str) -> Option<u64> {
        let until = *self.inner.get(&(user.to_owned(), command.to_owned()))?;
        let now = (self.now_ms)();
        if until <= now {
            return None;
        }
        Some((until - now).div_ceil(1000))
    }

    pub fn on_cooldown(&self, user: &str, command: &str) -> bool {
        self.remaining(user, command).is_some()
    }

    /// Limpa entradas expiradas (chamar periodicamente, ex.: a cada dispatch).
    pub fn sweep(&mut self) {
        let now = (self.now_ms)();
        self.inner.retain(|_, until| *until > now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NOW: AtomicU64 = AtomicU64::new(10_000);

    fn clock() -> u64 {
        NOW.load(Ordering::SeqCst)
    }

    #[test]
    fn cooldown_blocks_then_expires() {
        let mut c = Cooldowns::with_clock(clock);
        c.set("u1", "ping", 3_000);
        assert_eq!(c.remaining("u1", "ping"), Some(3));
        NOW.fetch_add(3_001, Ordering::SeqCst);
        assert_eq!(c.remaining("u1", "ping"), None);
        c.sweep();
        assert!(!c.on_cooldown("u1", "ping"));
        NOW.store(10_000, Ordering::SeqCst);
    }
}
