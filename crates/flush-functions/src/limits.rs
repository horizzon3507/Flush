//! Execution budgets (port of `runtime/limits.ts`).

use crate::errors::FnError;

/// Per-execution budgets: call count, nesting depth, output characters
/// and wall-clock timeout in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub calls: u64,
    pub depth: u64,
    pub output: usize,
    pub timeout_ms: u64,
}

/// Hard cap on script source characters (`maxSourceLength`).
pub const SOURCE_LIMIT: usize = 100_000;

pub const DEFAULT_LIMITS: Limits = Limits {
    calls: 10_000,
    depth: 64,
    output: 100_000,
    timeout_ms: 10_000,
};

impl Limits {
    /// Merge overrides over the defaults and validate (`resolveLimits`).
    pub fn resolved(overrides: Limits) -> Result<Limits, FnError> {
        for (name, value) in [
            ("calls", overrides.calls as usize),
            ("depth", overrides.depth as usize),
            ("output", overrides.output),
            ("timeoutMs", overrides.timeout_ms as usize),
        ] {
            if value < 1 {
                return Err(FnError::new("LIMITS", format!("Invalid limit: {name}.")));
            }
        }
        Ok(overrides)
    }

    /// Defaults with any provided fields replaced.
    pub fn with(
        calls: Option<u64>,
        depth: Option<u64>,
        output: Option<usize>,
        timeout_ms: Option<u64>,
    ) -> Result<Limits, FnError> {
        Self::resolved(Limits {
            calls: calls.unwrap_or(DEFAULT_LIMITS.calls),
            depth: depth.unwrap_or(DEFAULT_LIMITS.depth),
            output: output.unwrap_or(DEFAULT_LIMITS.output),
            timeout_ms: timeout_ms.unwrap_or(DEFAULT_LIMITS.timeout_ms),
        })
    }
}
