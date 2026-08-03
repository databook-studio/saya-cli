use super::{
    connection_schema,
    output::{failure, result},
    state,
};
use crate::{cli::ConnectionCommand, config_runtime::RuntimeConfig, render::RenderFormat};
use saya_connectors::{ConnectorOptions, DatabaseConnector, build_connector_with_prompt};
use saya_store::{AuditOperation, AuditStatus, SqliteStateStore};
use saya_types::DatabaseProfile;
use std::time::Instant;

pub(super) async fn run(
    command: ConnectionCommand,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    can_prompt: bool,
    state_db: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    match command {
        ConnectionCommand::List => list(runtime, format),
        ConnectionCommand::Test { profile_name } => {
            test(&profile_name, runtime, format, can_prompt, state_db).await
        }
        ConnectionCommand::Schema {
            profile_name,
            refresh,
        } => {
            connection_schema::run(
                &profile_name,
                refresh,
                runtime,
                format,
                can_prompt,
                state_db,
            )
            .await
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
    can_prompt: bool,
    state_db: &SqliteStateStore,
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
    let started = Instant::now();
    let identity = state::identity(name, profile, &runtime.cache_scope);
    let Some(connector) = connector(profile, runtime, 3, format, can_prompt).await? else {
        state::audit(
            state_db,
            &identity,
            AuditOperation::ConnectionTest,
            AuditStatus::Failure,
            started.elapsed(),
            None,
            None,
            format,
        )
        .await;
        return Ok(3);
    };
    match connector.connect().await {
        Ok(()) => {
            state::audit(
                state_db,
                &identity,
                AuditOperation::ConnectionTest,
                AuditStatus::Success,
                started.elapsed(),
                None,
                None,
                format,
            )
            .await;
            result(format!("Connection succeeded: {name}"), format)
        }
        Err(error) => {
            state::audit(
                state_db,
                &identity,
                AuditOperation::ConnectionTest,
                AuditStatus::Failure,
                started.elapsed(),
                None,
                None,
                format,
            )
            .await;
            failure(3, error, format)
        }
    }
}

pub(super) async fn connector(
    profile: &DatabaseProfile,
    runtime: &RuntimeConfig,
    code: i32,
    format: RenderFormat,
    can_prompt: bool,
) -> Result<Option<Box<dyn DatabaseConnector>>, Box<dyn std::error::Error>> {
    match build(profile, runtime, can_prompt).await {
        Ok(connector) => Ok(Some(connector)),
        Err(error) => {
            failure(code, error, format)?;
            Ok(None)
        }
    }
}

pub(super) async fn build(
    profile: &DatabaseProfile,
    runtime: &RuntimeConfig,
    can_prompt: bool,
) -> Result<Box<dyn DatabaseConnector>, saya_types::ConnectionError> {
    let resolver = runtime.secret_resolver();
    let settings = ConnectorOptions {
        query_timeout_seconds: runtime.resolved.query_timeout_seconds,
        ..Default::default()
    };
    build_connector_with_prompt(profile, &resolver, settings, can_prompt).await
}
