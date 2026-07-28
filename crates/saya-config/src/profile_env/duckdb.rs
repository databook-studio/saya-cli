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
    Ok(DatabaseProfile::DuckDb {
        path: required(env, "SAYA_DB_PATH", path)?,
        read_only,
    })
}
