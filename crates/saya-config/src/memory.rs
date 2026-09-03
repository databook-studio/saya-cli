use crate::{ConfigError, MemoryMode, model::MemoryFile};

/// Effective memory settings, resolved from `[memory]` plus safe defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMemory {
    pub mode: MemoryMode,
    pub max_contracts: u32,
    pub max_claims_per_contract: u32,
    pub max_context_bytes: u32,
}

/// Safe defaults so an upgrade changes nothing until the user opts in (ADR 0002, plan §10).
const DEFAULT_MAX_CONTRACTS: u32 = 5;
const DEFAULT_MAX_CLAIMS_PER_CONTRACT: u32 = 12;
const DEFAULT_MAX_CONTEXT_BYTES: u32 = 16384;

/// The floor on `[memory] max_context_bytes`. The recall path bounds the
/// rendered block to fit the loop's `context_byte_budget` at request time, so
/// there is no upper ceiling here: a large setting is clamped by what the
/// conversation budget actually holds, never allowed to crowd out the prompt.
const MIN_MAX_CONTEXT_BYTES: u32 = 1024;

/// Resolve `[memory]` into concrete values, validating the numeric bounds. Unknown
/// mode strings are already rejected by serde with the accepted values named, so only
/// the range checks live here.
pub(crate) fn resolve(file: &MemoryFile) -> Result<ResolvedMemory, ConfigError> {
    let mode = file.mode.unwrap_or(MemoryMode::Off);
    let max_contracts = file.max_contracts.unwrap_or(DEFAULT_MAX_CONTRACTS);
    let max_claims_per_contract = file
        .max_claims_per_contract
        .unwrap_or(DEFAULT_MAX_CLAIMS_PER_CONTRACT);
    let max_context_bytes = file.max_context_bytes.unwrap_or(DEFAULT_MAX_CONTEXT_BYTES);
    require_range("max_contracts", max_contracts, 1, 50)?;
    require_range("max_claims_per_contract", max_claims_per_contract, 1, 100)?;
    require_min(
        "max_context_bytes",
        max_context_bytes,
        MIN_MAX_CONTEXT_BYTES,
    )?;
    Ok(ResolvedMemory {
        mode,
        max_contracts,
        max_claims_per_contract,
        max_context_bytes,
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

fn require_min(field: &'static str, value: u32, min: u32) -> Result<(), ConfigError> {
    if value >= min {
        Ok(())
    } else {
        Err(ConfigError::MemoryRange {
            field,
            value,
            min,
            max: u32::MAX,
        })
    }
}
