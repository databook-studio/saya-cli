use std::collections::BTreeMap;

use saya_types::DatabaseProfile;

use super::values::required;
use crate::ConfigError;

pub(super) fn profile(
    env: &BTreeMap<String, String>,
    profile: Option<DatabaseProfile>,
) -> Result<DatabaseProfile, ConfigError> {
    let (path, read_only) = match profile {
        Some(DatabaseProfile::DuckDb { path, read_only }) => (Some(path), read_only),
        _ => (None, None),
    };
    let read_only = env
        .get("SAYA_DB_READ_ONLY")
        .map(|value| parse_read_only(value))
        .transpose()?
        .or(read_only);
    Ok(DatabaseProfile::DuckDb {
        path: required(env, "SAYA_DB_PATH", path)?,
        read_only,
    })
}

fn parse_read_only(value: &str) -> Result<bool, ConfigError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ConfigError::InvalidEnvironment {
            name: "SAYA_DB_READ_ONLY".into(),
            reason: "expected true or false".into(),
        }),
    }
}
