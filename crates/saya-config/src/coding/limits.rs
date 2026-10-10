use crate::ConfigError;

pub(super) const MAX_CHILDREN: usize = 3;
pub(super) const MAX_CONCURRENT_MODEL_REQUESTS: usize = 4;
pub(super) const MAX_CONCURRENT_COMMANDS: usize = 2;
pub(super) const MAX_LOGICAL_MODEL_REQUESTS: usize = 80;
pub(super) const MAX_PHYSICAL_TRANSPORT_ATTEMPTS: usize = 240;
pub(super) const MAX_TOOL_CALLS: usize = 300;
pub(super) const MAX_KNOWN_TOKENS: u64 = 400_000;
pub(super) const MAX_ELAPSED_SECONDS: u64 = 1_200;

pub(super) fn require(value: usize, field: &'static str, max: usize) -> Result<(), ConfigError> {
    if (1..=max).contains(&value) {
        Ok(())
    } else {
        Err(ConfigError::SettingOutOfRange {
            field,
            value,
            min: 1,
            max,
        })
    }
}

pub(super) fn require_u64(value: u64, field: &'static str, max: u64) -> Result<(), ConfigError> {
    if (1..=max).contains(&value) {
        Ok(())
    } else {
        Err(ConfigError::SettingOutOfRange {
            field,
            value: usize::try_from(value).unwrap_or(usize::MAX),
            min: 1,
            max: max as usize,
        })
    }
}
