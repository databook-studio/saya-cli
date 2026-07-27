use saya_config::{ConfigError, SecretResolver};
use saya_types::{ConnectionError, DatabaseProfile, PostgresSslMode};
use sqlx::postgres::{PgConnectOptions, PgSslMode};

use crate::{DatabaseConnector, PostgresConnector};

/// Runtime limits shared by connector instances created for one command.
#[derive(Debug, Clone, Copy)]
pub struct ConnectorOptions {
    pub query_timeout_seconds: u64,
    pub max_connections: u32,
}

impl Default for ConnectorOptions {
    fn default() -> Self {
        Self {
            query_timeout_seconds: 60,
            max_connections: 4,
        }
    }
}

/// Creates an available connector without ever constructing a credential URL.
pub async fn build_connector(
    profile: &DatabaseProfile,
    resolver: &dyn SecretResolver,
    settings: ConnectorOptions,
) -> Result<Box<dyn DatabaseConnector>, ConnectionError> {
    match profile {
        DatabaseProfile::Postgres {
            host,
            port,
            database,
            user,
            ssl_mode,
            password,
        } => {
            let mut options = PgConnectOptions::new()
                .host(host)
                .port(port.unwrap_or(5432))
                .database(database)
                .username(user)
                .ssl_mode(ssl_mode.map(ssl).unwrap_or(PgSslMode::Prefer));
            if let Some(reference) = password {
                let secret = resolver.resolve(reference).map_err(config_error)?;
                options = options.password(secret.expose());
            }
            Ok(Box::new(PostgresConnector::from_options(options, settings)))
        }
        other => Err(ConnectionError::Unsupported(format!(
            "{} connector is not implemented yet",
            other.dialect().as_str()
        ))),
    }
}

fn ssl(mode: PostgresSslMode) -> PgSslMode {
    match mode {
        PostgresSslMode::Disable => PgSslMode::Disable,
        PostgresSslMode::Prefer => PgSslMode::Prefer,
        PostgresSslMode::Require => PgSslMode::Require,
        PostgresSslMode::VerifyCa => PgSslMode::VerifyCa,
        PostgresSslMode::VerifyFull => PgSslMode::VerifyFull,
    }
}

fn config_error(error: ConfigError) -> ConnectionError {
    ConnectionError::InvalidConfiguration(error.to_string())
}
