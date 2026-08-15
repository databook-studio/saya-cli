use crate::{ConfigError, MemoryLearning, MemoryRecall, model::MemoryFile};

/// Effective memory settings, resolved from `[memory]` plus the safe defaults.
/// Nothing reads these yet — Phase 4b wires behaviour. A setting that parses but
/// changes nothing lets the defaults be proven safe before anything depends on them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMemory {
    pub recall: MemoryRecall,
    pub learning: MemoryLearning,
    pub max_contracts: u32,
    pub max_claims_per_contract: u32,
    pub max_context_bytes: u32,
    pub retention_days: u32,
}

/// Safe defaults so an upgrade changes nothing until the user opts in (ADR 0002, plan §10).
const DEFAULT_MAX_CONTRACTS: u32 = 5;
const DEFAULT_MAX_CLAIMS_PER_CONTRACT: u32 = 12;
const DEFAULT_MAX_CONTEXT_BYTES: u32 = 16384;
const DEFAULT_RETENTION_DAYS: u32 = 180;

/// The floor reserved out of the agent message budget for the parts a request
/// needs besides memory context: the fixed system prompt, the context-block
/// wrapper, and the user's own question. `max_context_bytes` may not consume
/// this — the question is the point and the context is the assist.
const CONTEXT_RESERVATION_BYTES: u32 = 4096;

/// The largest accepted `[memory] max_context_bytes`: the agent message budget
/// ([`saya_types::MAX_MESSAGE_BYTES`]) less [`CONTEXT_RESERVATION_BYTES`].
/// Derived from the agent constant rather than copied so the two cannot drift: a
/// setting above this guarantees an ordinary request exceeds the message budget
/// and fails with `ContextLimit` — a memory setting breaking the thing memory is
/// supposed to help — so it is refused here with a typed error naming both values.
const MAX_CONTEXT_BYTES_CEILING: u32 =
    (saya_types::MAX_MESSAGE_BYTES as u32).saturating_sub(CONTEXT_RESERVATION_BYTES);

/// Resolve `[memory]` into concrete values, validating the numeric bounds. Unknown
/// mode strings are already rejected by serde with the accepted values named, so only
/// the range checks live here.
pub(crate) fn resolve(file: &MemoryFile) -> Result<ResolvedMemory, ConfigError> {
    let recall = file.recall.unwrap_or(MemoryRecall::Confirmed);
    let learning = file.learning.unwrap_or(MemoryLearning::Off);
    let max_contracts = file.max_contracts.unwrap_or(DEFAULT_MAX_CONTRACTS);
    let max_claims_per_contract = file
        .max_claims_per_contract
        .unwrap_or(DEFAULT_MAX_CLAIMS_PER_CONTRACT);
    let max_context_bytes = file.max_context_bytes.unwrap_or(DEFAULT_MAX_CONTEXT_BYTES);
    let retention_days = file.retention_days.unwrap_or(DEFAULT_RETENTION_DAYS);
    require_range("max_contracts", max_contracts, 1, 50)?;
    require_range("max_claims_per_contract", max_claims_per_contract, 1, 100)?;
    require_range(
        "max_context_bytes",
        max_context_bytes,
        1024,
        MAX_CONTEXT_BYTES_CEILING,
    )?;
    require_range("retention_days", retention_days, 1, 3650)?;
    Ok(ResolvedMemory {
        recall,
        learning,
        max_contracts,
        max_claims_per_contract,
        max_context_bytes,
        retention_days,
    })
}

fn require_range(field: &'static str, value: u32, min: u32, max: u32) -> Result<(), ConfigError> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(ConfigError::MemoryRange {
            field,
            value,
            min,
            max,
        })
    }
}
