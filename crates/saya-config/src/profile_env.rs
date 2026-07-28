use std::collections::BTreeMap;

use saya_types::{DatabaseProfile, SecretRef};

use crate::ConfigError;

mod duckdb;
mod mysql;
mod values;

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
        "duckdb" => duckdb::profile(&env, profile).map(Some),
        other => Err(ConfigError::UnsupportedDatabaseType(other.into())),
    }
}

fn network_profile(
    env: &BTreeMap<String, String>,
    profile: Option<DatabaseProfile>,
    postgres: bool,
) -> Result<DatabaseProfile, ConfigError> {
    let (host, port, database, user, postgres_ssl_mode, mysql_ssl_mode, ssl_ca, password) =
        match profile {
            Some(DatabaseProfile::Postgres {
                host,
                port,
                database,
                user,
                ssl_mode,
                password,
            }) => (
                Some(host),
                port,
                Some(database),
                Some(user),
                ssl_mode,
                None,
                None,
                password,
            ),
            Some(DatabaseProfile::Mysql {
                host,
                port,
                database,
                user,
                ssl_mode,
                ssl_ca,
                password,
            }) => (
                Some(host),
                port,
                Some(database),
                Some(user),
                None,
                ssl_mode,
                ssl_ca,
                password,
            ),
            _ => (None, None, None, None, None, None, None, None),
        };
    let host = values::required(env, "SAYA_DB_HOST", host)?;
    let database = values::required(env, "SAYA_DB_NAME", database)?;
    let user = values::required(env, "SAYA_DB_USER", user)?;
    let port = env
        .get("SAYA_DB_PORT")
        .map(|value| value.parse().map_err(|_| values::invalid_port()))
        .transpose()?
        .or(port);
    let password = env
        .contains_key("SAYA_DB_PASSWORD")
        .then(|| SecretRef::Env {
            env: "SAYA_DB_PASSWORD".into(),
        })
        .or(password);
    let ssl_ca = env
        .contains_key("SAYA_DB_SSL_CA")
        .then(|| SecretRef::Env {
            env: "SAYA_DB_SSL_CA".into(),
        })
        .or(ssl_ca);
    if postgres {
        Ok(DatabaseProfile::Postgres {
            host,
            port,
            database,
            user,
            ssl_mode: env
                .get("SAYA_DB_SSLMODE")
                .map(|value| crate::postgres_env::parse_ssl_mode(value))
                .transpose()?
                .or(postgres_ssl_mode),
            password,
        })
    } else {
        Ok(DatabaseProfile::Mysql {
            host,
            port,
            database,
            user,
            ssl_mode: env
                .get("SAYA_DB_SSLMODE")
                .map(|value| mysql::parse_ssl_mode(value))
                .transpose()?
                .or(mysql_ssl_mode),
            ssl_ca,
            password,
        })
    }
}

fn profile_type(profile: &DatabaseProfile) -> &'static str {
    match profile {
        DatabaseProfile::Postgres { .. } => "postgresql",
        DatabaseProfile::Mysql { .. } => "mysql",
        DatabaseProfile::DuckDb { .. } => "duckdb",
        DatabaseProfile::Snowflake { .. } => "snowflake",
    }
}
