use saya_config::{ConfigError, SecretResolver};
use saya_types::{ConnectionError, DatabaseProfile, MySqlSslMode, PostgresSslMode};
use sqlx::{
    mysql::{MySqlConnectOptions, MySqlSslMode as DriverMySqlSslMode},
    postgres::{PgConnectOptions, PgSslMode},
};

use crate::{DatabaseConnector, DuckDbConnector, MySqlConnector, PostgresConnector};

mod snowflake_factory;

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
    build_connector_with_prompt(profile, resolver, settings, false).await
}

/// Creates a connector with explicit permission for interactive authentication.
pub async fn build_connector_with_prompt(
    profile: &DatabaseProfile,
    resolver: &dyn SecretResolver,
    settings: ConnectorOptions,
    can_prompt: bool,
) -> Result<Box<dyn DatabaseConnector>, ConnectionError> {
    match profile {
        DatabaseProfile::Postgres {
            host,
            port,
            database,
            user,
            ssl_mode,
            password,
            ..
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
        DatabaseProfile::Mysql {
            host,
            port,
            database,
            user,
            ssl_mode,
            ssl_ca,
            password,
        } => {
            let mut options = MySqlConnectOptions::new()
                .host(host)
                .port(port.unwrap_or(3306))
                .database(database)
                .username(user)
                .ssl_mode(mysql_ssl(ssl_mode.unwrap_or(MySqlSslMode::VerifyIdentity)));
            if let Some(reference) = password {
                let secret = resolver.resolve(reference).map_err(config_error)?;
                options = options.password(secret.expose());
            }
            if let Some(reference) = ssl_ca {
                let secret = resolver.resolve(reference).map_err(config_error)?;
                options = options.ssl_ca_from_pem(secret.expose().as_bytes().to_vec());
            }
            Ok(Box::new(MySqlConnector::from_options(
                options, database, settings,
            )))
        }
        DatabaseProfile::DuckDb { path, read_only } => {
            if path != ":memory:" && read_only.is_none() {
                return Err(ConnectionError::InvalidConfiguration(
                    "DuckDB file profiles must set read_only explicitly".into(),
                ));
            }
            DuckDbConnector::open(path, read_only.unwrap_or(false), settings)
                .await
                .map(|item| Box::new(item) as _)
        }
        DatabaseProfile::Snowflake { .. } => {
            snowflake_factory::build(profile, resolver, settings, can_prompt)
        }
    }
}

fn mysql_ssl(mode: MySqlSslMode) -> DriverMySqlSslMode {
    match mode {
        MySqlSslMode::Disable => DriverMySqlSslMode::Disabled,
        MySqlSslMode::Prefer => DriverMySqlSslMode::Preferred,
        MySqlSslMode::Require => DriverMySqlSslMode::Required,
        MySqlSslMode::VerifyCa => DriverMySqlSslMode::VerifyCa,
        MySqlSslMode::VerifyIdentity => DriverMySqlSslMode::VerifyIdentity,
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
