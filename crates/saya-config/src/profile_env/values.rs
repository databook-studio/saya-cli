use std::collections::BTreeMap;

use crate::ConfigError;

pub(super) fn required(
    env: &BTreeMap<String, String>,
    name: &'static str,
    fallback: Option<String>,
) -> Result<String, ConfigError> {
    env.get(name)
        .cloned()
        .or(fallback)
        .ok_or(ConfigError::MissingDatabaseField { name })
}

pub(super) fn invalid_port() -> ConfigError {
    ConfigError::InvalidEnvironment {
        name: "SAYA_DB_PORT".into(),
        reason: "expected an unsigned 16-bit integer".into(),
    }
}

pub(super) fn parse_bool(name: &'static str, value: &str) -> Result<bool, ConfigError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ConfigError::InvalidEnvironment {
            name: name.into(),
            reason: "expected true or false".into(),
        }),
    }
}
