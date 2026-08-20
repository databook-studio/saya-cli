use std::collections::BTreeMap;

use saya_types::DatabaseProfile;

use super::values;
use crate::ConfigError;

pub(super) fn profile(
    env: &BTreeMap<String, String>,
    profile: Option<DatabaseProfile>,
) -> Result<DatabaseProfile, ConfigError> {
    let (path, read_only) = match profile {
        Some(DatabaseProfile::Sqlite { path, read_only }) => (Some(path), Some(read_only)),
        _ => (None, None),
    };
    let read_only = env
        .get("SAYA_DB_READ_ONLY")
        .map(|v| values::parse_bool("SAYA_DB_READ_ONLY", v))
        .transpose()?
        .or(read_only)
        .unwrap_or(true);
    Ok(DatabaseProfile::Sqlite {
        path: values::required(env, "SAYA_DB_PATH", path)?,
        read_only,
    })
}
