use crate::agent_runtime::AgentRuntimeError;
use crate::connection_registry::{ConnectionEntry, ConnectionRegistry};
use saya_config::SecretResolver;
use saya_connectors::{ConnectorOptions, build_connector_with_prompt};
use saya_types::DatabaseProfile;
use std::path::Path;

/// Builds a registry of live connections: the primary plus each secondary.
/// The primary MUST connect (its failure is returned as an error). Each secondary is
/// connected fail-fast with no interactive auth; any secondary that fails to build or
/// connect is skipped so one bad secondary never breaks the primary run.
pub(crate) async fn build_registry(
    resolver: &dyn SecretResolver,
    cache_scope: &Path,
    query_timeout_seconds: u64,
    can_prompt: bool,
    primary_name: &str,
    primary_profile: &DatabaseProfile,
    secondaries: &[(String, DatabaseProfile)],
) -> Result<ConnectionRegistry, AgentRuntimeError> {
    let mut registry = ConnectionRegistry::new(primary_name);

    let connector = build_connector_with_prompt(
        primary_profile,
        resolver,
        ConnectorOptions {
            query_timeout_seconds,
            ..Default::default()
        },
        can_prompt,
    )
    .await
    .map_err(|err| AgentRuntimeError::Database(err.to_string()))?;

    connector
        .connect()
        .await
        .map_err(|err| AgentRuntimeError::Database(err.to_string()))?;

    let dialect = connector.dialect();
    let profile_id =
        crate::profile_identity::profile_identity(primary_name, primary_profile, cache_scope);

    registry.insert(
        primary_name,
        ConnectionEntry {
            connector,
            dialect,
            profile_id: Some(profile_id),
        },
    );

    for (name, profile) in secondaries {
        let connector = match build_connector_with_prompt(
            profile,
            resolver,
            ConnectorOptions {
                query_timeout_seconds,
                ..Default::default()
            },
            false,
        )
        .await
        {
            Ok(c) => c,
            Err(_) => continue,
        };

        if connector.connect().await.is_err() {
            continue;
        }

        let dialect = connector.dialect();
        let profile_id = crate::profile_identity::profile_identity(name, profile, cache_scope);

        registry.insert(
            name,
            ConnectionEntry {
                connector,
                dialect,
                profile_id: Some(profile_id),
            },
        );
    }

    Ok(registry)
}

#[cfg(test)]
#[path = "connection_build_tests.rs"]
mod tests;
