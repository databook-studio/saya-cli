use std::collections::BTreeMap;

use saya_types::{DatabaseProfile, SecretRef, SnowflakeAuth};

use super::values::required;
use crate::ConfigError;

pub(super) fn profile(
    env: &BTreeMap<String, String>,
    profile: Option<DatabaseProfile>,
) -> Result<DatabaseProfile, ConfigError> {
    let existing = match profile {
        Some(DatabaseProfile::Snowflake {
            account,
            user,
            auth_type,
            private_key,
            password,
            passphrase,
            warehouse,
            database,
            schema,
            role,
        }) => (
            Some(account),
            Some(user),
            Some(auth_type),
            private_key,
            password,
            passphrase,
            warehouse,
            database,
            schema,
            role,
        ),
        _ => (None, None, None, None, None, None, None, None, None, None),
    };
    let account = required(env, "SAYA_DB_ACCOUNT", existing.0)?;
    let user = required(env, "SAYA_DB_USER", existing.1)?;
    let auth_type = env
        .get("SAYA_DB_AUTH_TYPE")
        .map(|value| parse_auth(value))
        .transpose()?
        .or(existing.2)
        .ok_or(ConfigError::MissingDatabaseField {
            name: "SAYA_DB_AUTH_TYPE",
        })?;
    let private_key = secret(env, "SAYA_DB_PRIVATE_KEY").or(existing.3);
    let password = secret(env, "SAYA_DB_PASSWORD").or(existing.4);
    let passphrase = secret(env, "SAYA_DB_PRIVATE_KEY_PASSPHRASE").or(existing.5);
    let (private_key, password, passphrase) = match auth_type {
        SnowflakeAuth::Keypair => (private_key, None, passphrase),
        SnowflakeAuth::Userpass => (None, password, None),
        SnowflakeAuth::Externalbrowser => (None, None, None),
    };
    validate_secrets(&auth_type, private_key.as_ref(), password.as_ref())?;
    Ok(DatabaseProfile::Snowflake {
        account,
        user,
        auth_type,
        private_key,
        password,
        passphrase,
        warehouse: text(env, "SAYA_DB_WAREHOUSE").or(existing.6),
        database: text(env, "SAYA_DB_NAME").or(existing.7),
        schema: text(env, "SAYA_DB_SCHEMA").or(existing.8),
        role: text(env, "SAYA_DB_ROLE").or(existing.9),
    })
}

fn parse_auth(value: &str) -> Result<SnowflakeAuth, ConfigError> {
    match value {
        "keypair" => Ok(SnowflakeAuth::Keypair),
        "userpass" => Ok(SnowflakeAuth::Userpass),
        "externalbrowser" => Ok(SnowflakeAuth::Externalbrowser),
        _ => Err(ConfigError::InvalidEnvironment {
            name: "SAYA_DB_AUTH_TYPE".into(),
            reason: "expected keypair, userpass, or externalbrowser".into(),
        }),
    }
}

fn validate_secrets(
    auth_type: &SnowflakeAuth,
    private_key: Option<&SecretRef>,
    password: Option<&SecretRef>,
) -> Result<(), ConfigError> {
    let required = match auth_type {
        SnowflakeAuth::Keypair => (private_key, "SAYA_DB_PRIVATE_KEY"),
        SnowflakeAuth::Userpass => (password, "SAYA_DB_PASSWORD"),
        SnowflakeAuth::Externalbrowser => return Ok(()),
    };
    required
        .0
        .map(|_| ())
        .ok_or(ConfigError::MissingDatabaseField { name: required.1 })
}

fn secret(env: &BTreeMap<String, String>, name: &'static str) -> Option<SecretRef> {
    env.contains_key(name)
        .then(|| SecretRef::Env { env: name.into() })
}

fn text(env: &BTreeMap<String, String>, name: &str) -> Option<String> {
    env.get(name).cloned()
}
