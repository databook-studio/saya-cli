use crate::{
    cli::ConnectionCommand,
    config_runtime::RuntimeConfig,
    render::{RenderFormat, TerminalEvent},
};
use saya_connectors::{ConnectorOptions, DatabaseConnector, build_connector};
use saya_types::{DatabaseProfile, SqlDialect};

use super::output::{emit, failure, result, unavailable};

pub(super) async fn run(
    command: ConnectionCommand,
    runtime: &RuntimeConfig,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    match command {
        ConnectionCommand::List => list(runtime, format),
        ConnectionCommand::Test { profile_name } => test(&profile_name, runtime, format).await,
        ConnectionCommand::Schema { profile_name, .. } => {
            schema(&profile_name, runtime, format).await
        }
    }
}

fn list(runtime: &RuntimeConfig, format: RenderFormat) -> Result<i32, Box<dyn std::error::Error>> {
    let names = runtime
        .connections
        .profiles
        .iter()
        .map(|(name, profile)| format!("{name} ({})", profile.dialect().as_str()))
        .collect::<Vec<_>>();
    result(
        if names.is_empty() {
            "No configured profiles.".into()
        } else {
            names.join("\n")
        },
        format,
    )
}

async fn test(
    name: &str,
    runtime: &RuntimeConfig,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    let profile = match runtime.named_profile(name) {
        Ok(profile) => profile,
        Err(error) => {
            return failure(
                3,
                saya_types::ConnectionError::InvalidConfiguration(error.to_string()),
                format,
            );
        }
    };
    let Some(connector) = connector(profile, runtime, 3, format).await? else {
        return Ok(3);
    };
    match connector.connect().await {
        Ok(()) => result(format!("Connection succeeded: {name}"), format),
        Err(error) => failure(3, error, format),
    }
}

async fn schema(
    name: &str,
    runtime: &RuntimeConfig,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    let profile = match runtime.named_profile(name) {
        Ok(profile) => profile,
        Err(error) => {
            return failure(
                3,
                saya_types::ConnectionError::InvalidConfiguration(error.to_string()),
                format,
            );
        }
    };
    let Some(connector) = connector(profile, runtime, 3, format).await? else {
        return Ok(3);
    };
    match connector.schema().await {
        Ok(schema) => {
            emit(TerminalEvent::Schema { schema }, format);
            Ok(0)
        }
        Err(error) => failure(3, error, format),
    }
}

pub(super) async fn connector(
    profile: &DatabaseProfile,
    runtime: &RuntimeConfig,
    code: i32,
    format: RenderFormat,
) -> Result<Option<Box<dyn DatabaseConnector>>, Box<dyn std::error::Error>> {
    if profile.dialect() != SqlDialect::Postgres {
        unavailable(
            code,
            format!("{} connector", profile.dialect().as_str()),
            format,
        )?;
        return Ok(None);
    }
    let resolver = runtime.secret_resolver();
    let settings = ConnectorOptions {
        query_timeout_seconds: runtime.resolved.query_timeout_seconds,
        ..Default::default()
    };
    match build_connector(profile, &resolver, settings).await {
        Ok(connector) => Ok(Some(connector)),
        Err(error) => {
            failure(code, error, format)?;
            Ok(None)
        }
    }
}
