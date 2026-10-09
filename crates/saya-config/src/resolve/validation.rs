use super::*;

/// Rejects an `[ai] context_byte_budget` below the floor with a typed error
///, matching the `[memory]` range-check style. The upper end is
/// unbounded: a user may raise the budget to fit a larger context window, which
/// is the reason the setting exists, so no ceiling is enforced here.
pub(super) fn require_context_byte_budget(value: usize) -> Result<(), ConfigError> {
    if value >= MIN_CONTEXT_BYTE_BUDGET {
        Ok(())
    } else {
        Err(ConfigError::SettingBelowMinimum {
            field: "context_byte_budget",
            value,
            min: MIN_CONTEXT_BYTE_BUDGET,
        })
    }
}

/// Rejects a declared `[ai] context_window_tokens` of zero. A window of zero
/// tokens is a typo, not a deployment: no model turns every prompt into a
/// refusal. There is no upper bound — a user declaring their gateway's window
/// is stating a fact saya has no other way to learn, and the largest published
/// window today is a few million tokens, so any plausible ceiling would
/// outlive its reason.
pub(super) fn require_context_window_tokens(value: Option<u64>) -> Result<(), ConfigError> {
    if value.is_none_or(|tokens| tokens > 0) {
        Ok(())
    } else {
        Err(ConfigError::SettingBelowMinimum {
            field: "context_window_tokens",
            value: value.unwrap_or_default() as usize,
            min: 1,
        })
    }
}

/// Rejects an `[ai] retry_delays_ms` schedule longer than `MAX_RETRY_DELAYS`.
/// An empty list is allowed — it means "do not retry" (one attempt, no
/// sleeps), which is a valid choice. Sibling to `require_context_byte_budget`
/// in style: a typed error at resolve time rather than a silent clamp at the
/// point of use.
pub(super) fn require_retry_delays(value: &[u64]) -> Result<(), ConfigError> {
    if value.len() <= MAX_RETRY_DELAYS {
        Ok(())
    } else {
        Err(ConfigError::SettingAboveMaximum {
            field: "retry_delays_ms",
            value: value.len(),
            max: MAX_RETRY_DELAYS,
        })
    }
}

/// Rejects a `[run] candidates` outside `MIN_CANDIDATES..=MAX_CANDIDATES`.
/// Zero is meaningless (zero attempts answer nothing) and an unbounded value
/// would let a typo start hundreds of full agent runs, multiplying a user's
/// model spend. The accepted range is reported, not just one bound, matching
/// the `[memory]` range-check style. Sibling to `require_retry_delays`.
pub(super) fn require_candidates(value: usize) -> Result<(), ConfigError> {
    if (MIN_CANDIDATES..=MAX_CANDIDATES).contains(&value) {
        Ok(())
    } else {
        Err(ConfigError::SettingOutOfRange {
            field: "candidates",
            value,
            min: MIN_CANDIDATES,
            max: MAX_CANDIDATES,
        })
    }
}
