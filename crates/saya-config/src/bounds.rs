//! The two shape checks every bounded numeric setting shares, so a section
//! that adds a limit adds a bound and not a third way of reporting one.

use crate::ConfigError;

/// A count with a floor of one and a stated ceiling.
pub(crate) fn require(value: usize, field: &'static str, max: usize) -> Result<(), ConfigError> {
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

/// The same for a `u64` setting, reported in the `usize` the error carries.
pub(crate) fn require_u64(value: u64, field: &'static str, max: u64) -> Result<(), ConfigError> {
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
