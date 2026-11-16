use std::num::NonZeroU64;
use std::time::Duration;

use crate::ConfigError;

/// A validated ceiling for one limit dimension: a positive integer, or
/// `unlimited` for a surface that may spend without a bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetLimit {
    Finite(NonZeroU64),
    Unlimited,
}

impl BudgetLimit {
    /// Reads one raw value: absent takes the fallback, `"unlimited"` is the one
    /// unlimited spelling, and a positive integer is the only finite spelling.
    /// Everything else — zero, a negative, a float, a bool, a list, a near-miss
    /// string — is refused naming the exact key that carries it, so the fix is
    /// never in doubt.
    pub(crate) fn parse(
        value: Option<&toml::Value>,
        key: &'static str,
        fallback: BudgetLimit,
    ) -> Result<Self, ConfigError> {
        match value {
            None => Ok(fallback),
            Some(toml::Value::String(value)) if value == "unlimited" => Ok(Self::Unlimited),
            Some(toml::Value::Integer(value)) => u64::try_from(*value)
                .ok()
                .and_then(NonZeroU64::new)
                .map(Self::Finite)
                .ok_or(ConfigError::InvalidLimit { key }),
            Some(_) => Err(ConfigError::InvalidLimit { key }),
        }
    }

    /// A built-in default. Panics only on `0`, which no default spells.
    pub(crate) fn finite(value: u64) -> Self {
        Self::Finite(NonZeroU64::new(value).expect("built-in limits are nonzero"))
    }

    /// The finite value, or `None` for `unlimited`.
    pub fn value(self) -> Option<u64> {
        match self {
            Self::Finite(value) => Some(value.get()),
            Self::Unlimited => None,
        }
    }

    /// The finite value as a duration, or `None` for `unlimited`.
    pub fn duration(self) -> Option<Duration> {
        self.value().map(Duration::from_secs)
    }
}

/// A progress count: a non-negative integer within its ceiling. Zero is a
/// stated intent — it disables that guard — so this differs from the
/// positive-integer [`BudgetLimit`] dimensions.
pub(crate) fn parse_count(
    value: Option<&toml::Value>,
    key: &'static str,
    fallback: u32,
    max: u32,
) -> Result<u32, ConfigError> {
    let Some(value) = value else {
        return Ok(fallback);
    };
    let toml::Value::Integer(raw) = value else {
        return Err(ConfigError::InvalidLimit { key });
    };
    let raw = u64::try_from(*raw).map_err(|_| ConfigError::InvalidLimit { key })?;
    if raw > u64::from(max) {
        return Err(ConfigError::SettingOutOfRange {
            field: key,
            value: usize::try_from(raw).unwrap_or(usize::MAX),
            min: 0,
            max: max as usize,
        });
    }
    Ok(raw as u32)
}
