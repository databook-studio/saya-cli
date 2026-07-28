use saya_config::SecretResolver;
use saya_types::{ConnectionError, DatabaseProfile, SecretRef, SnowflakeAuth};

use crate::snowflake::{Auth, Context, ExternalBrowser, Keypair, Userpass};
use crate::{ConnectorOptions, DatabaseConnector, SnowflakeConnector};

pub(super) fn build(
    profile: &DatabaseProfile,
    resolver: &dyn SecretResolver,
    settings: ConnectorOptions,
    can_prompt: bool,
) -> Result<Box<dyn DatabaseConnector>, ConnectionError> {
    let DatabaseProfile::Snowflake {
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
    } = profile
    else {
        unreachable!()
    };
    let auth = match auth_type {
        SnowflakeAuth::Keypair => Auth::Keypair(Keypair {
            private_key: required(private_key.as_ref(), resolver)?,
            passphrase: optional(passphrase.as_ref(), resolver)?,
        }),
        SnowflakeAuth::Userpass => Auth::Userpass(Userpass {
            password: required(password.as_ref(), resolver)?,
            token: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        }),
        SnowflakeAuth::Externalbrowser => Auth::ExternalBrowser(ExternalBrowser {
            enabled: can_prompt,
            token: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        }),
    };
    let context = Context {
        warehouse: warehouse.clone(),
        database: database.clone(),
        schema: schema.clone(),
        role: role.clone(),
    };
    SnowflakeConnector::new(account.clone(), user.clone(), auth, context, settings)
        .map(|item| Box::new(item) as _)
}

fn required(
    reference: Option<&SecretRef>,
    resolver: &dyn SecretResolver,
) -> Result<String, ConnectionError> {
    reference
        .ok_or_else(|| {
            ConnectionError::InvalidConfiguration(
                "Snowflake authentication secret is required".into(),
            )
        })
        .and_then(|item| {
            resolver
                .resolve(item)
                .map(|value| value.expose().to_owned())
                .map_err(|_| {
                    ConnectionError::InvalidConfiguration(
                        "Snowflake authentication secret could not be resolved".into(),
                    )
                })
        })
}
fn optional(
    reference: Option<&SecretRef>,
    resolver: &dyn SecretResolver,
) -> Result<Option<String>, ConnectionError> {
    reference
        .map(|item| {
            resolver
                .resolve(item)
                .map(|value| value.expose().to_owned())
                .map_err(|_| {
                    ConnectionError::InvalidConfiguration(
                        "Snowflake authentication secret could not be resolved".into(),
                    )
                })
        })
        .transpose()
}
