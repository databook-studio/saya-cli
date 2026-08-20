use super::registry::{ConnectionEntry, ConnectionRegistry};
use crate::agent::runtime::AgentRuntimeError;
use futures_util::stream::{self, StreamExt};
use saya_config::SecretResolver;
use saya_connectors::{ConnectorOptions, build_connector_with_prompt};
use saya_types::DatabaseProfile;
use std::path::Path;

const MAX_CONCURRENT_SECONDARY_CONNECTIONS: usize = 8;

enum SecondaryResult {
    Success {
        name: String,
        entry: ConnectionEntry,
    },
    Failure {
        name: String,
        reason: String,
    },
}

/// Builds a registry of live connections: the primary plus each secondary.
/// The primary MUST connect (its failure is returned as an error). Each secondary is
/// connected concurrently (bounded cap of 8) with no interactive auth; any secondary that
/// fails to build or connect is captured as a failure and returned along with the registry.
pub(crate) async fn build_registry(
    resolver: &dyn SecretResolver,
    cache_scope: &Path,
    query_timeout_seconds: u64,
    can_prompt: bool,
    primary_name: &str,
    primary_profile: &DatabaseProfile,
    secondaries: &[(String, DatabaseProfile)],
) -> Result<(ConnectionRegistry, Vec<(String, String)>), AgentRuntimeError> {
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
            profile_id: Some(profile_id.to_string()),
        },
    );

    let results = stream::iter(secondaries.iter().enumerate().map(
        |(idx, (name, profile))| async move {
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
                Err(err) => {
                    return (
                        idx,
                        SecondaryResult::Failure {
                            name: name.clone(),
                            reason: err.to_string(),
                        },
                    );
                }
            };

            if let Err(err) = connector.connect().await {
                return (
                    idx,
                    SecondaryResult::Failure {
                        name: name.clone(),
                        reason: err.to_string(),
                    },
                );
            }

            let dialect = connector.dialect();
            let profile_id = crate::profile_identity::profile_identity(name, profile, cache_scope);

            (
                idx,
                SecondaryResult::Success {
                    name: name.clone(),
                    entry: ConnectionEntry {
                        connector,
                        dialect,
                        profile_id: Some(profile_id.to_string()),
                    },
                },
            )
        },
    ))
    .buffer_unordered(MAX_CONCURRENT_SECONDARY_CONNECTIONS)
    .collect::<Vec<_>>()
    .await;

    let mut sorted_results = results;
    sorted_results.sort_by_key(|(idx, _)| *idx);

    let mut failures = Vec::new();
    for (_, result) in sorted_results {
        match result {
            SecondaryResult::Success { name, entry } => {
                registry.insert(&name, entry);
            }
            SecondaryResult::Failure { name, reason } => {
                failures.push((name, reason));
            }
        }
    }

    Ok((registry, failures))
}

#[cfg(test)]
#[path = "build_tests.rs"]
mod tests;
