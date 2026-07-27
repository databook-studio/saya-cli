use std::collections::BTreeMap;

use saya_types::{DatabaseProfile, SecretRef};

use crate::ConfigError;

pub(crate) fn overlay_database_environment(
    profile: Option<DatabaseProfile>,
    env_file: &BTreeMap<String, String>,
    process: &BTreeMap<String, String>,
) -> Result<Option<DatabaseProfile>, ConfigError> {
    let mut env = env_file.clone();
    env.extend(process.clone());
    if !env.keys().any(|key| key.starts_with("SAYA_DB_")) {
        return Ok(profile);
    }
    let database_type = env
        .get("SAYA_DB_TYPE")
        .map(String::as_str)
        .or_else(|| profile.as_ref().map(profile_type))
        .ok_or(ConfigError::MissingDatabaseField {
            name: "SAYA_DB_TYPE",
        })?;
    match database_type {
        "postgresql" | "postgres" => network_profile(&env, profile, true).map(Some),
        "mysql" => network_profile(&env, profile, false).map(Some),
        "duckdb" => duckdb_profile(&env, profile).map(Some),
        other => Err(ConfigError::UnsupportedDatabaseType(other.into())),
    }
}

fn network_profile(
    env: &BTreeMap<String, String>,
    profile: Option<DatabaseProfile>,
    postgres: bool,
) -> Result<DatabaseProfile, ConfigError> {
    let (host, port, database, user, password) = match profile {
        Some(DatabaseProfile::Postgres {
            host,
            port,
            database,
            user,
            password,
        })
        | Some(DatabaseProfile::Mysql {
            host,
            port,
            database,
            user,
            password,
        }) => (Some(host), port, Some(database), Some(user), password),
        _ => (None, None, None, None, None),
    };
    let host = required(env, "SAYA_DB_HOST", host)?;
    let database = required(env, "SAYA_DB_NAME", database)?;
    let user = required(env, "SAYA_DB_USER", user)?;
    let port = env
        .get("SAYA_DB_PORT")
        .map(|value| value.parse().map_err(|_| invalid_port()))
        .transpose()?
        .or(port);
    let password = env
        .contains_key("SAYA_DB_PASSWORD")
        .then(|| SecretRef::Env {
            env: "SAYA_DB_PASSWORD".into(),
        })
        .or(password);
    if postgres {
        Ok(DatabaseProfile::Postgres {
            host,
            port,
            database,
            user,
            password,
        })
    } else {
        Ok(DatabaseProfile::Mysql {
            host,
            port,
            database,
            user,
            password,
        })
    }
}

fn duckdb_profile(
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

fn required(
    env: &BTreeMap<String, String>,
    name: &'static str,
    fallback: Option<String>,
) -> Result<String, ConfigError> {
    env.get(name)
        .cloned()
        .or(fallback)
        .ok_or(ConfigError::MissingDatabaseField { name })
}

fn profile_type(profile: &DatabaseProfile) -> &'static str {
    match profile {
        DatabaseProfile::Postgres { .. } => "postgresql",
        DatabaseProfile::Mysql { .. } => "mysql",
        DatabaseProfile::DuckDb { .. } => "duckdb",
        DatabaseProfile::Snowflake { .. } => "snowflake",
    }
}

fn invalid_port() -> ConfigError {
    ConfigError::InvalidEnvironment {
        name: "SAYA_DB_PORT".into(),
        reason: "expected an unsigned 16-bit integer".into(),
    }
}
